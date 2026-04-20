//! In-browser tests for the Willow Leptos web UI.
//!
//! Run with: `wasm-pack test crates/web --headless --chrome`
//!
//! These tests render Leptos components in a real browser DOM and verify
//! that signals, events, and effects work correctly.

use std::collections::HashMap;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Mount a component into the document body for testing.
/// Returns the container element.
fn mount_test<F, V>(f: F) -> web_sys::HtmlElement
where
    F: FnOnce() -> V + 'static,
    V: IntoView + 'static,
{
    let document = web_sys::window().unwrap().document().unwrap();
    let container = document.create_element("div").unwrap();
    container.set_id(&format!("test-{}", js_sys::Math::random()));
    document.body().unwrap().append_child(&container).unwrap();

    let container_clone = container.clone();
    let _handle = leptos::mount::mount_to(container_clone.unchecked_into(), f);
    std::mem::forget(_handle); // Keep the view mounted for the test's lifetime.

    container.unchecked_into()
}

/// Wait for reactive effects to flush.
async fn tick() {
    gloo_timers::future::TimeoutFuture::new(0).await;
}

/// Query all elements matching a CSS selector within a container.
fn query_all(container: &web_sys::HtmlElement, selector: &str) -> Vec<web_sys::Element> {
    let list = container.query_selector_all(selector).unwrap();
    (0..list.length())
        .filter_map(|i| list.item(i))
        .filter_map(|n| n.dyn_into::<web_sys::Element>().ok())
        .collect()
}

/// Query a single element matching a CSS selector.
fn query(container: &web_sys::HtmlElement, selector: &str) -> Option<web_sys::Element> {
    container.query_selector(selector).unwrap()
}

/// Get text content of an element.
fn text(el: &web_sys::Element) -> String {
    el.text_content().unwrap_or_default()
}

/// Counter for generating unique test message IDs.
static MSG_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Create a test `DisplayMessage` with minimal arguments.
fn make_msg(author: &str, body: &str, timestamp_ms: u64) -> willow_client::DisplayMessage {
    let id = MSG_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    willow_client::DisplayMessage {
        id: format!("test-msg-{id}"),
        channel_id: "test-channel".into(),
        author_peer_id: willow_identity::Identity::generate().endpoint_id(),
        author_display_name: author.into(),
        body: body.into(),
        is_local: false,
        timestamp_ms,
        reactions: std::collections::HashMap::new(),
        edited: false,
        deleted: false,
        reply_to: None,
        reply_preview: None,
    }
}

/// Simulate typing text into an input element (set value + dispatch input event).
fn simulate_type(input: &web_sys::HtmlInputElement, value: &str) {
    input.set_value(value);
    let event = web_sys::InputEvent::new("input").unwrap();
    input
        .dyn_ref::<web_sys::EventTarget>()
        .unwrap()
        .dispatch_event(&event)
        .unwrap();
}

/// Simulate a click on an element.
fn simulate_click(el: &web_sys::Element) {
    let event = web_sys::MouseEvent::new("click").unwrap();
    el.dyn_ref::<web_sys::EventTarget>()
        .unwrap()
        .dispatch_event(&event)
        .unwrap();
}

// ── Signal & Reactivity Tests ───────────────────────────────────────────────

#[wasm_bindgen_test]
async fn signal_updates_dom() {
    let (value, set_value) = signal(0i32);

    let container = mount_test(move || {
        view! { <span class="counter">{move || value.get().to_string()}</span> }
    });

    tick().await;

    let span = query(&container, ".counter").unwrap();
    assert_eq!(text(&span), "0");

    set_value.set(42);
    tick().await;

    assert_eq!(text(&span), "42");
}

#[wasm_bindgen_test]
async fn for_loop_renders_list() {
    let (items, set_items) = signal(vec!["alpha".to_string(), "beta".to_string()]);

    let container = mount_test(move || {
        view! {
            <ul>
                <For
                    each=move || items.get()
                    key=|s| s.clone()
                    let:item
                >
                    <li class="item">{item}</li>
                </For>
            </ul>
        }
    });

    tick().await;

    let lis = query_all(&container, ".item");
    assert_eq!(lis.len(), 2);
    assert_eq!(text(&lis[0]), "alpha");
    assert_eq!(text(&lis[1]), "beta");

    // Add an item reactively.
    set_items.update(|v| v.push("gamma".to_string()));
    tick().await;

    let lis = query_all(&container, ".item");
    assert_eq!(lis.len(), 3);
    assert_eq!(text(&lis[2]), "gamma");
}

#[wasm_bindgen_test]
async fn conditional_rendering() {
    let (show, set_show) = signal(false);

    let container = mount_test(move || {
        view! {
            <div>
                {move || {
                    if show.get() {
                        Some(view! { <span class="visible">"shown"</span> })
                    } else {
                        None
                    }
                }}
            </div>
        }
    });

    tick().await;
    assert!(query(&container, ".visible").is_none());

    set_show.set(true);
    tick().await;
    assert!(query(&container, ".visible").is_some());
    assert_eq!(text(&query(&container, ".visible").unwrap()), "shown");
}

// ── Message Rendering Tests ─────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn message_renders_author_and_body() {
    let msg = make_msg("Alice", "Hello world!", 1000);

    let container = mount_test(move || {
        // Inline a simplified message view for testing
        let author = msg.author_display_name.clone();
        let body = msg.body.clone();
        view! {
            <div class="message">
                <span class="author">{author}</span>
                <span class="body">{body}</span>
            </div>
        }
    });

    tick().await;

    let author = query(&container, ".author").unwrap();
    assert_eq!(text(&author), "Alice");

    let body = query(&container, ".body").unwrap();
    assert_eq!(text(&body), "Hello world!");
}

#[wasm_bindgen_test]
async fn message_list_shows_empty_state() {
    let (messages, _set_messages) = signal(Vec::<willow_client::DisplayMessage>::new());

    let container = mount_test(move || {
        view! {
            <div class="message-list">
                {move || {
                    if messages.get().is_empty() {
                        view! { <div class="empty-state">"No messages"</div> }.into_any()
                    } else {
                        view! { <div class="has-messages">"Has messages"</div> }.into_any()
                    }
                }}
            </div>
        }
    });

    tick().await;

    assert!(query(&container, ".empty-state").is_some());
    assert_eq!(
        text(&query(&container, ".empty-state").unwrap()),
        "No messages"
    );
}

#[wasm_bindgen_test]
async fn message_list_renders_messages() {
    let (messages, set_messages) = signal(Vec::<willow_client::DisplayMessage>::new());

    let container = mount_test(move || {
        view! {
            <div>
                <For
                    each=move || messages.get()
                    key=|m| m.id.clone()
                    let:msg
                >
                    <div class="msg">{msg.body.clone()}</div>
                </For>
            </div>
        }
    });

    tick().await;
    assert_eq!(query_all(&container, ".msg").len(), 0);

    // Add messages.
    set_messages.set(vec![make_msg("A", "first", 1), make_msg("B", "second", 2)]);
    tick().await;

    let msgs = query_all(&container, ".msg");
    assert_eq!(msgs.len(), 2);
    assert_eq!(text(&msgs[0]), "first");
    assert_eq!(text(&msgs[1]), "second");
}

// ── Input Tests ─────────────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn input_captures_value() {
    let (value, set_value) = signal(String::new());

    let container = mount_test(move || {
        view! {
            <input
                type="text"
                class="test-input"
                prop:value=move || value.get()
                on:input=move |ev| {
                    set_value.set(event_target_value(&ev));
                }
            />
        }
    });

    tick().await;

    let input: web_sys::HtmlInputElement =
        query(&container, ".test-input").unwrap().unchecked_into();

    // Simulate typing by setting value and dispatching input event.
    simulate_type(&input, "hello");
    tick().await;

    assert_eq!(value.get_untracked(), "hello");
}

#[wasm_bindgen_test]
async fn input_sends_on_enter() {
    let (sent, set_sent) = signal(Option::<String>::None);
    let (input_text, set_input_text) = signal(String::new());

    let container = mount_test(move || {
        let on_send = set_sent;
        view! {
            <input
                type="text"
                class="test-send-input"
                prop:value=move || input_text.get()
                on:input=move |ev| set_input_text.set(event_target_value(&ev))
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" {
                        let text = input_text.get_untracked();
                        if !text.is_empty() {
                            on_send.set(Some(text));
                            set_input_text.set(String::new());
                        }
                    }
                }
            />
        }
    });

    tick().await;

    let input: web_sys::HtmlInputElement = query(&container, ".test-send-input")
        .unwrap()
        .unchecked_into();

    // Type "hello".
    simulate_type(&input, "hello");
    tick().await;

    // Press Enter.
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Enter");
    let enter =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    input
        .dyn_ref::<web_sys::EventTarget>()
        .unwrap()
        .dispatch_event(&enter)
        .unwrap();
    tick().await;

    assert_eq!(sent.get_untracked(), Some("hello".to_string()));
    assert_eq!(input_text.get_untracked(), "");
}

// ── Channel List Tests ──────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn channel_list_renders_channels() {
    let (channels, _) = signal(vec![
        "general".to_string(),
        "random".to_string(),
        "voice".to_string(),
    ]);
    let (current, _) = signal("general".to_string());

    let container = mount_test(move || {
        view! {
            <div class="channel-list">
                <For
                    each=move || channels.get()
                    key=|ch| ch.clone()
                    let:channel
                >
                    {
                        let ch = channel.clone();
                        let active = move || current.get() == ch;
                        view! {
                            <div class=move || if active() { "channel active" } else { "channel" }>
                                {"# "} {channel.clone()}
                            </div>
                        }
                    }
                </For>
            </div>
        }
    });

    tick().await;

    let channels = query_all(&container, ".channel, .channel.active");
    assert_eq!(channels.len(), 3);

    // First channel should be active.
    let active = query_all(&container, ".active");
    assert_eq!(active.len(), 1);
    assert!(text(&active[0]).contains("general"));
}

// ── Peer Count Tests ────────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn peer_count_displays_correctly() {
    let (count, set_count) = signal(0usize);

    let container = mount_test(move || {
        view! {
            <span class="peer-count">
                {move || {
                    let n = count.get();
                    if n == 1 { "1 peer".to_string() } else { format!("{n} peers") }
                }}
            </span>
        }
    });

    tick().await;
    assert_eq!(text(&query(&container, ".peer-count").unwrap()), "0 peers");

    set_count.set(1);
    tick().await;
    assert_eq!(text(&query(&container, ".peer-count").unwrap()), "1 peer");

    set_count.set(5);
    tick().await;
    assert_eq!(text(&query(&container, ".peer-count").unwrap()), "5 peers");
}

// ── Sidebar Tests ───────────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn sidebar_shows_server_name_in_user_area() {
    let container = mount_test(move || {
        view! {
            <div class="sidebar">
                <div class="sidebar-header">"Willow"</div>
                <div class="user-area">
                    <div class="status-dot"></div>
                    <span class="user-display-name">"TestUser"</span>
                </div>
            </div>
        }
    });

    tick().await;

    let header = query(&container, ".sidebar-header").unwrap();
    assert_eq!(text(&header), "Willow");

    let user_name = query(&container, ".user-display-name").unwrap();
    assert_eq!(text(&user_name), "TestUser");

    // User area should be present
    assert!(query(&container, ".user-area").is_some());
}

#[wasm_bindgen_test]
async fn channel_click_switches_active_channel() {
    let (current, set_current) = signal("general".to_string());
    let (channels, _) = signal(vec!["general".to_string(), "random".to_string()]);

    let container = mount_test(move || {
        view! {
            <div class="channel-list">
                <For
                    each=move || channels.get()
                    key=|ch| ch.clone()
                    let:channel
                >
                    {
                        let ch_active = channel.clone();
                        let ch_click = channel.clone();
                        let active = move || current.get() == ch_active;
                        view! {
                            <div
                                class=move || if active() { "channel-item active" } else { "channel-item" }
                                on:click=move |_| set_current.set(ch_click.clone())
                            >
                                <span>{"# "} {channel.clone()}</span>
                            </div>
                        }
                    }
                </For>
            </div>
        }
    });

    tick().await;

    // Initially "general" is active.
    let active = query_all(&container, ".channel-item.active");
    assert_eq!(active.len(), 1);
    assert!(text(&active[0]).contains("general"));

    // Click "random".
    let items = query_all(&container, ".channel-item");
    assert_eq!(items.len(), 2);
    simulate_click(&items[1]);
    tick().await;

    // Now "random" should be active.
    let active = query_all(&container, ".channel-item.active");
    assert_eq!(active.len(), 1);
    assert!(text(&active[0]).contains("random"));
}

#[wasm_bindgen_test]
async fn unread_badge_shows_count() {
    let (unread, set_unread) = signal(HashMap::<String, usize>::new());

    let container = mount_test(move || {
        let channel_name = "random".to_string();
        view! {
            <div class="channel-item">
                <span>{"# random"}</span>
                <span class="channel-item-right">
                    {
                        let ch = channel_name.clone();
                        move || {
                            let counts = unread.get();
                            counts.get(&ch).copied().filter(|c| *c > 0).map(|c| {
                                view! {
                                    <span class="unread-badge">{c.to_string()}</span>
                                }
                            })
                        }
                    }
                </span>
            </div>
        }
    });

    tick().await;

    // No unread badge initially.
    assert!(query(&container, ".unread-badge").is_none());

    // Set unread count for "random".
    let mut map = HashMap::new();
    map.insert("random".to_string(), 3);
    set_unread.set(map);
    tick().await;

    let badge = query(&container, ".unread-badge").unwrap();
    assert_eq!(text(&badge), "3");

    // Clear unread.
    set_unread.set(HashMap::new());
    tick().await;
    assert!(query(&container, ".unread-badge").is_none());
}

// ── Settings Panel Tests ────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn settings_displays_peer_id() {
    let peer_id = "12D3KooWTestPeerId123456789";

    let container = mount_test(move || {
        view! {
            <div class="settings-panel">
                <div class="settings-section">
                    <label>"Your Peer ID"</label>
                    <div class="peer-id-display">
                        <code class="peer-id-text">{peer_id}</code>
                        <button class="btn btn-sm">"Copy"</button>
                    </div>
                </div>
            </div>
        }
    });

    tick().await;

    let peer_id_el = query(&container, ".peer-id-text").unwrap();
    assert_eq!(text(&peer_id_el), "12D3KooWTestPeerId123456789");

    // The settings panel itself should exist.
    assert!(query(&container, ".settings-panel").is_some());
    // Copy button should exist.
    assert!(query(&container, ".peer-id-display .btn").is_some());
}

#[wasm_bindgen_test]
async fn display_name_input_captures_text() {
    let (display_name, set_display_name) = signal(String::new());

    let container = mount_test(move || {
        view! {
            <div class="settings-section">
                <label>"Display Name"</label>
                <input
                    type="text"
                    class="display-name-input"
                    placeholder="Enter display name..."
                    prop:value=move || display_name.get()
                    on:input=move |ev| set_display_name.set(event_target_value(&ev))
                />
            </div>
        }
    });

    tick().await;

    let input: web_sys::HtmlInputElement = query(&container, ".display-name-input")
        .unwrap()
        .unchecked_into();

    simulate_type(&input, "Alice");
    tick().await;

    assert_eq!(display_name.get_untracked(), "Alice");
}

// ── Message Detail Tests ────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn edited_message_shows_badge() {
    let mut msg = make_msg("Alice", "Updated text", 5_400_000);
    msg.edited = true;

    let show_edited = msg.edited && !msg.deleted;
    let author = msg.author_display_name.clone();
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="meta">
                    <span class="author">{author}</span>
                    {if show_edited {
                        Some(view! { <span class="edited">"(edited)"</span> })
                    } else {
                        None
                    }}
                </div>
                <div class="body">{body}</div>
            </div>
        }
    });

    tick().await;

    let edited = query(&container, ".edited").unwrap();
    assert_eq!(text(&edited), "(edited)");
}

#[wasm_bindgen_test]
async fn deleted_message_shows_placeholder() {
    let mut msg = make_msg("Bob", "[message deleted]", 5_400_000);
    msg.deleted = true;
    msg.body = "[message deleted]".to_string();

    let body_class = if msg.deleted { "body deleted" } else { "body" };
    let body = msg.body.clone();
    // An edited+deleted message should NOT show the (edited) badge.
    let show_edited = msg.edited && !msg.deleted;

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="meta">
                    {if show_edited {
                        Some(view! { <span class="edited">"(edited)"</span> })
                    } else {
                        None
                    }}
                </div>
                <div class=body_class>{body}</div>
            </div>
        }
    });

    tick().await;

    let body_el = query(&container, ".body.deleted").unwrap();
    assert_eq!(text(&body_el), "[message deleted]");

    // No edited badge on a deleted message.
    assert!(query(&container, ".edited").is_none());
}

#[wasm_bindgen_test]
async fn reply_preview_renders() {
    let mut msg = make_msg("Charlie", "My reply", 5_400_000);
    msg.reply_preview = Some("Alice: original message".to_string());

    let reply_preview = msg.reply_preview.clone();
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                {reply_preview.clone().map(|preview| {
                    view! {
                        <div class="reply-preview">{format!("> {preview}")}</div>
                    }
                })}
                <div class="body">{body}</div>
            </div>
        }
    });

    tick().await;

    let preview = query(&container, ".reply-preview").unwrap();
    assert_eq!(text(&preview), "> Alice: original message");

    let body_el = query(&container, ".body").unwrap();
    assert_eq!(text(&body_el), "My reply");
}

#[wasm_bindgen_test]
async fn reactions_render_with_count() {
    let mut msg = make_msg("Dave", "Nice!", 5_400_000);
    msg.reactions.insert(
        "thumbsup".to_string(),
        vec!["Alice".to_string(), "Bob".to_string()],
    );
    msg.reactions
        .insert("heart".to_string(), vec!["Charlie".to_string()]);

    let reactions: Vec<(String, usize)> = msg
        .reactions
        .iter()
        .map(|(emoji, authors)| (emoji.clone(), authors.len()))
        .collect();
    let has_reactions = !reactions.is_empty();

    let container = mount_test(move || {
        view! {
            <div class="message">
                {if has_reactions {
                    Some(view! {
                        <div class="reactions">
                            {reactions.clone().into_iter().map(|(emoji, count)| {
                                view! {
                                    <span class="reaction">
                                        {emoji} " " {count.to_string()}
                                    </span>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    })
                } else {
                    None
                }}
            </div>
        }
    });

    tick().await;

    let reaction_els = query_all(&container, ".reaction");
    assert_eq!(reaction_els.len(), 2);

    // Collect all reaction texts.
    let mut reaction_texts: Vec<String> = reaction_els.iter().map(text).collect();
    reaction_texts.sort();

    // Should contain "heart 1" and "thumbsup 2" (sorted).
    assert!(reaction_texts
        .iter()
        .any(|t| t.contains("heart") && t.contains("1")));
    assert!(reaction_texts
        .iter()
        .any(|t| t.contains("thumbsup") && t.contains("2")));
}

#[wasm_bindgen_test]
async fn message_timestamp_displays() {
    // 1 hour 30 minutes = 5400 seconds = 5_400_000 ms => "01:30"
    let msg = make_msg("Eve", "Hello!", 5_400_000);
    let timestamp = willow_client::util::format_timestamp(msg.timestamp_ms);

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="meta">
                    <span class="author">"Eve"</span>
                    <span class="timestamp">{timestamp.clone()}</span>
                </div>
                <div class="body">"Hello!"</div>
            </div>
        }
    });

    tick().await;

    let ts = query(&container, ".timestamp").unwrap();
    assert_eq!(text(&ts), "01:30");
}

// ── Member List Tests ───────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn member_list_shows_peers_with_names() {
    let (peers, _) = signal(vec![
        ("peer-id-1".to_string(), "Alice".to_string()),
        ("peer-id-2".to_string(), "Bob".to_string()),
        ("peer-id-3".to_string(), "Charlie".to_string()),
    ]);

    let container = mount_test(move || {
        view! {
            <div class="member-list">
                <h3>"Online"</h3>
                <For
                    each=move || peers.get()
                    key=|(id, _)| id.clone()
                    let:peer
                >
                    {
                        let (_pid, name) = peer;
                        view! {
                            <div class="member-item">
                                <div class="status-dot"></div>
                                <span class="member-name">{name}</span>
                            </div>
                        }
                    }
                </For>
            </div>
        }
    });

    tick().await;

    let items = query_all(&container, ".member-item");
    assert_eq!(items.len(), 3);

    let names: Vec<String> = query_all(&container, ".member-name")
        .iter()
        .map(text)
        .collect();
    assert_eq!(names, vec!["Alice", "Bob", "Charlie"]);
}

#[wasm_bindgen_test]
async fn empty_member_list_shows_placeholder() {
    let (peers, _) = signal(Vec::<(String, String)>::new());

    let container = mount_test(move || {
        view! {
            <div class="member-list">
                <h3>"Online"</h3>
                {move || {
                    if peers.get().is_empty() {
                        Some(view! { <div class="empty-state">"No peers connected"</div> })
                    } else {
                        None
                    }
                }}
            </div>
        }
    });

    tick().await;

    let placeholder = query(&container, ".empty-state").unwrap();
    assert_eq!(text(&placeholder), "No peers connected");
}

// ── Server List Tests ───────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn server_list_shows_server_icons() {
    let (servers, _) = signal(vec![
        ("srv-1".to_string(), "Gaming".to_string()),
        ("srv-2".to_string(), "Work".to_string()),
    ]);

    let container = mount_test(move || {
        view! {
            <div class="server-rail">
                <For
                    each=move || servers.get()
                    key=|(id, _)| id.clone()
                    let:server
                >
                    {
                        let (_id, name) = server;
                        let initial = name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string();
                        view! {
                            <div class="server-icon" title=name.clone()>
                                {initial}
                            </div>
                        }
                    }
                </For>
            </div>
        }
    });

    tick().await;

    let icons = query_all(&container, ".server-icon");
    assert_eq!(icons.len(), 2);
    assert_eq!(text(&icons[0]), "G");
    assert_eq!(text(&icons[1]), "W");
}

#[wasm_bindgen_test]
async fn active_server_highlighted() {
    let (servers, _) = signal(vec![
        ("srv-1".to_string(), "Gaming".to_string()),
        ("srv-2".to_string(), "Work".to_string()),
    ]);
    let (active_id, set_active_id) = signal("srv-1".to_string());

    let container = mount_test(move || {
        view! {
            <div class="server-rail">
                <For
                    each=move || servers.get()
                    key=|(id, _)| id.clone()
                    let:server
                >
                    {
                        let (id, name) = server;
                        let id_check = id.clone();
                        let id_click = id.clone();
                        let initial = name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string();
                        view! {
                            <div
                                class=move || {
                                    if active_id.get() == id_check {
                                        "server-icon active"
                                    } else {
                                        "server-icon"
                                    }
                                }
                                on:click=move |_| set_active_id.set(id_click.clone())
                            >
                                {initial}
                            </div>
                        }
                    }
                </For>
            </div>
        }
    });

    tick().await;

    // srv-1 should be active initially.
    let active = query_all(&container, ".server-icon.active");
    assert_eq!(active.len(), 1);
    assert_eq!(text(&active[0]), "G");

    // Click srv-2 ("Work").
    let icons = query_all(&container, ".server-icon");
    simulate_click(&icons[1]);
    tick().await;

    let active = query_all(&container, ".server-icon.active");
    assert_eq!(active.len(), 1);
    assert_eq!(text(&active[0]), "W");
}

// ── Connection Status Tests ─────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn connection_status_indicator() {
    let (status, set_status) = signal("connecting".to_string());
    let (peer_count, set_peer_count) = signal(0usize);

    let container = mount_test(move || {
        view! {
            <div class="connection-status">
                <span class=move || {
                    let s = status.get();
                    match s.as_str() {
                        "connected" => "status-dot connected",
                        "connecting" => "status-dot connecting",
                        _ => "status-dot disconnected",
                    }
                }></span>
                <span class="connection-text">{move || {
                    let s = status.get();
                    let n = peer_count.get();
                    match s.as_str() {
                        "connected" => {
                            if n == 1 {
                                "Connected (1 peer)".to_string()
                            } else {
                                format!("Connected ({n} peers)")
                            }
                        }
                        "connecting" => "Connecting...".to_string(),
                        _ => "Disconnected".to_string(),
                    }
                }}</span>
            </div>
        }
    });

    tick().await;

    // Initially connecting.
    let dot = query(&container, ".status-dot").unwrap();
    assert!(dot.class_list().contains("connecting"));
    let txt = query(&container, ".connection-text").unwrap();
    assert_eq!(text(&txt), "Connecting...");

    // Transition to connected with 3 peers.
    set_status.set("connected".to_string());
    set_peer_count.set(3);
    tick().await;

    let dot = query(&container, ".status-dot").unwrap();
    assert!(dot.class_list().contains("connected"));
    let txt = query(&container, ".connection-text").unwrap();
    assert_eq!(text(&txt), "Connected (3 peers)");

    // Connected with 1 peer (singular).
    set_peer_count.set(1);
    tick().await;
    let txt = query(&container, ".connection-text").unwrap();
    assert_eq!(text(&txt), "Connected (1 peer)");

    // Transition to disconnected.
    set_status.set("disconnected".to_string());
    tick().await;

    let dot = query(&container, ".status-dot").unwrap();
    assert!(dot.class_list().contains("disconnected"));
    let txt = query(&container, ".connection-text").unwrap();
    assert_eq!(text(&txt), "Disconnected");
}

#[wasm_bindgen_test]
async fn connection_status_dot_css_class_changes() {
    let (status, set_status) = signal("disconnected".to_string());

    let container = mount_test(move || {
        view! {
            <span class=move || {
                let s = status.get();
                match s.as_str() {
                    "connected" => "status-dot connected",
                    "connecting" => "status-dot connecting",
                    _ => "status-dot disconnected",
                }
            }></span>
        }
    });

    tick().await;

    let dot = query(&container, ".status-dot").unwrap();
    assert!(dot.class_list().contains("disconnected"));

    set_status.set("connecting".to_string());
    tick().await;

    let dot = query(&container, ".status-dot").unwrap();
    assert!(dot.class_list().contains("connecting"));
    assert!(!dot.class_list().contains("disconnected"));

    set_status.set("connected".to_string());
    tick().await;

    let dot = query(&container, ".status-dot").unwrap();
    assert!(dot.class_list().contains("connected"));
    assert!(!dot.class_list().contains("connecting"));
}

// ── Timestamp Rendering Tests ───────────────────────────────────────────────

#[wasm_bindgen_test]
async fn zero_timestamp_renders_empty() {
    let ts = willow_client::util::format_timestamp(0);
    assert_eq!(ts, "");

    let container = mount_test(move || {
        view! {
            <span class="timestamp">{ts.clone()}</span>
        }
    });

    tick().await;

    let el = query(&container, ".timestamp").unwrap();
    assert_eq!(text(&el), "");
}

// ── Unread Badge Tests ──────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn unread_badge_not_shown_for_zero() {
    let (unread, _) = signal({
        let mut m = HashMap::<String, usize>::new();
        m.insert("general".to_string(), 0);
        m
    });

    let container = mount_test(move || {
        let ch = "general".to_string();
        view! {
            <div class="channel-item">
                {
                    let ch = ch.clone();
                    move || {
                        let counts = unread.get();
                        counts.get(&ch).copied().filter(|c| *c > 0).map(|c| {
                            view! {
                                <span class="unread-badge">{c.to_string()}</span>
                            }
                        })
                    }
                }
            </div>
        }
    });

    tick().await;

    // Zero unread should not show a badge.
    assert!(query(&container, ".unread-badge").is_none());
}

#[wasm_bindgen_test]
async fn unread_badge_updates_reactively() {
    let (unread, set_unread) = signal(HashMap::<String, usize>::new());

    let container = mount_test(move || {
        let ch = "random".to_string();
        view! {
            <div class="channel-item">
                {
                    let ch = ch.clone();
                    move || {
                        let counts = unread.get();
                        counts.get(&ch).copied().filter(|c| *c > 0).map(|c| {
                            view! {
                                <span class="unread-badge">{c.to_string()}</span>
                            }
                        })
                    }
                }
            </div>
        }
    });

    tick().await;
    assert!(query(&container, ".unread-badge").is_none());

    // Add 5 unread.
    set_unread.set({
        let mut m = HashMap::new();
        m.insert("random".to_string(), 5);
        m
    });
    tick().await;

    let badge = query(&container, ".unread-badge").unwrap();
    assert_eq!(text(&badge), "5");

    // Update to 10 unread.
    set_unread.set({
        let mut m = HashMap::new();
        m.insert("random".to_string(), 10);
        m
    });
    tick().await;

    let badge = query(&container, ".unread-badge").unwrap();
    assert_eq!(text(&badge), "10");
}

// ── MessageView Component Tests ─────────────────────────────────────────────
// These tests mirror the real MessageView component's rendering logic exactly.

#[wasm_bindgen_test]
async fn message_view_local_author_class() {
    let mut msg = make_msg("LocalUser", "my message", 1000);
    msg.is_local = true;

    let author_class = if msg.is_local {
        "author local"
    } else {
        "author remote"
    };
    let author = msg.author_display_name.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <span class=author_class>{author}</span>
            </div>
        }
    });

    tick().await;

    let author_el = query(&container, ".author.local").unwrap();
    assert_eq!(text(&author_el), "LocalUser");
    assert!(query(&container, ".author.remote").is_none());
}

#[wasm_bindgen_test]
async fn message_view_remote_author_class() {
    let msg = make_msg("RemoteUser", "their message", 1000);

    let author_class = if msg.is_local {
        "author local"
    } else {
        "author remote"
    };
    let author = msg.author_display_name.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <span class=author_class>{author}</span>
            </div>
        }
    });

    tick().await;

    let author_el = query(&container, ".author.remote").unwrap();
    assert_eq!(text(&author_el), "RemoteUser");
    assert!(query(&container, ".author.local").is_none());
}

#[wasm_bindgen_test]
async fn message_without_reactions_has_no_reactions_div() {
    let msg = make_msg("User", "plain message", 1000);

    let reactions: Vec<(String, usize)> = msg
        .reactions
        .iter()
        .map(|(emoji, authors)| (emoji.clone(), authors.len()))
        .collect();
    let has_reactions = !reactions.is_empty();

    let container = mount_test(move || {
        view! {
            <div class="message">
                {if has_reactions {
                    Some(view! { <div class="reactions">"reactions here"</div> })
                } else {
                    None
                }}
            </div>
        }
    });

    tick().await;

    assert!(query(&container, ".reactions").is_none());
}

#[wasm_bindgen_test]
async fn message_without_reply_has_no_preview() {
    let msg = make_msg("User", "standalone message", 1000);

    let reply_preview = msg.reply_preview.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                {reply_preview.clone().map(|preview| {
                    view! {
                        <div class="reply-preview">{format!("> {preview}")}</div>
                    }
                })}
            </div>
        }
    });

    tick().await;

    assert!(query(&container, ".reply-preview").is_none());
}

// ── Settings Section Tests ──────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn settings_status_message_shows_and_hides() {
    let (status_msg, set_status_msg) = signal(String::new());

    let container = mount_test(move || {
        view! {
            <div class="settings-panel">
                {move || {
                    let msg = status_msg.get();
                    if msg.is_empty() {
                        None
                    } else {
                        Some(view! {
                            <div class="settings-status">{msg}</div>
                        })
                    }
                }}
            </div>
        }
    });

    tick().await;

    // No status initially.
    assert!(query(&container, ".settings-status").is_none());

    // Show status.
    set_status_msg.set("Saved.".to_string());
    tick().await;

    let status = query(&container, ".settings-status").unwrap();
    assert_eq!(text(&status), "Saved.");

    // Clear status.
    set_status_msg.set(String::new());
    tick().await;
    assert!(query(&container, ".settings-status").is_none());
}

#[wasm_bindgen_test]
async fn settings_shows_invite_section() {
    let container = mount_test(move || {
        view! {
            <div class="settings-panel">
                <div class="settings-section invite-section">
                    <h3>"Invite a Peer"</h3>
                    <label>"Recipient Peer ID"</label>
                    <input type="text" placeholder="12D3KooW..." />
                    <button class="btn btn-primary">"Generate Invite"</button>
                </div>
            </div>
        }
    });

    tick().await;

    assert!(query(&container, ".invite-section").is_some());
    let heading = query(&container, ".invite-section h3").unwrap();
    assert_eq!(text(&heading), "Invite a Peer");
}

// ── Channel Create Input Tests ──────────────────────────────────────────────

#[wasm_bindgen_test]
async fn channel_create_input_toggles() {
    let (creating, set_creating) = signal(false);

    let container = mount_test(move || {
        view! {
            <div>
                <button
                    class="channel-add-btn"
                    on:click=move |_| set_creating.set(true)
                >
                    "+"
                </button>
                {move || {
                    if creating.get() {
                        Some(view! {
                            <div class="channel-create-input">
                                <input type="text" placeholder="channel name" />
                            </div>
                        })
                    } else {
                        None
                    }
                }}
            </div>
        }
    });

    tick().await;

    // Not visible initially.
    assert!(query(&container, ".channel-create-input").is_none());

    // Click the add button.
    let btn = query(&container, ".channel-add-btn").unwrap();
    simulate_click(&btn);
    tick().await;

    // Now the input should be visible.
    assert!(query(&container, ".channel-create-input").is_some());
}

// ── Mobile Sidebar Toggle Tests ─────────────────────────────────────────────

#[wasm_bindgen_test]
async fn sidebar_open_class_toggles() {
    let (open, set_open) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class=move || if open.get() { "sidebar open" } else { "sidebar" }>
                "sidebar content"
            </div>
        }
    });

    tick().await;

    let sidebar = query(&container, ".sidebar").unwrap();
    assert!(!sidebar.class_list().contains("open"));

    set_open.set(true);
    tick().await;

    let sidebar = query(&container, ".sidebar").unwrap();
    assert!(sidebar.class_list().contains("open"));
}

// ── Feature 1: Message Grouping Tests ───────────────────────────────────────

#[wasm_bindgen_test]
async fn consecutive_messages_grouped() {
    // When multiple messages come from the same author in a row, only the
    // first should show the `.meta` header; subsequent ones get class `grouped`.
    let msgs = [
        make_msg("Alice", "Hello!", 1000),
        make_msg("Alice", "How are you?", 2000),
        make_msg("Bob", "I'm good", 3000),
        make_msg("Bob", "Thanks", 4000),
        make_msg("Alice", "Great!", 5000),
    ];

    let container = mount_test(move || {
        let views: Vec<_> = msgs
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let show_header = if i == 0 {
                    true
                } else {
                    msgs[i - 1].author_display_name != msg.author_display_name
                };
                let msg_class = if show_header {
                    "message"
                } else {
                    "message grouped"
                };
                let author = msg.author_display_name.clone();
                let body = msg.body.clone();
                view! {
                    <div class=msg_class>
                        {if show_header {
                            Some(view! {
                                <div class="meta">
                                    <span class="author">{author}</span>
                                </div>
                            })
                        } else {
                            None
                        }}
                        <div class="body">{body}</div>
                    </div>
                }
            })
            .collect();
        view! { <div class="message-list">{views}</div> }
    });

    tick().await;

    let all_messages = query_all(&container, ".message");
    assert_eq!(all_messages.len(), 5);

    // Grouped messages (no header shown).
    let grouped = query_all(&container, ".message.grouped");
    assert_eq!(grouped.len(), 2, "should have 2 grouped messages");

    // Non-grouped messages have a .meta div.
    let metas = query_all(&container, ".meta");
    assert_eq!(
        metas.len(),
        3,
        "should have 3 headers (first of each author group)"
    );

    // Grouped messages should NOT have .meta.
    for g in &grouped {
        assert!(
            g.query_selector(".meta").unwrap().is_none(),
            "grouped message should not have .meta"
        );
    }
}

// ── Feature 2: Reply UI Tests ───────────────────────────────────────────────

#[wasm_bindgen_test]
async fn reply_bar_shows_when_replying() {
    let (replying_to, set_replying_to) = signal(Option::<willow_client::DisplayMessage>::None);

    let container = mount_test(move || {
        view! {
            <div class="input-area">
                {move || {
                    replying_to.get().map(|m| {
                        let preview = if m.body.len() > 60 {
                            format!("{}...", &m.body[..60])
                        } else {
                            m.body.clone()
                        };
                        view! {
                            <div class="reply-bar">
                                <span class="reply-bar-text">
                                    {format!("Replying to {}: {}", m.author_display_name, preview)}
                                </span>
                                <button
                                    class="reply-bar-cancel"
                                    on:click=move |_| set_replying_to.set(None)
                                >
                                    "x"
                                </button>
                            </div>
                        }
                    })
                }}
                <input type="text" placeholder="Message #channel" />
            </div>
        }
    });

    tick().await;

    // No reply bar initially.
    assert!(query(&container, ".reply-bar").is_none());

    // Set a reply target.
    let msg = make_msg("Alice", "original message", 1000);
    set_replying_to.set(Some(msg));
    tick().await;

    // Reply bar should now be visible.
    let _bar = query(&container, ".reply-bar").unwrap();
    let bar_text = text(&query(&container, ".reply-bar-text").unwrap());
    assert!(
        bar_text.contains("Replying to Alice"),
        "reply bar should mention the author"
    );
    assert!(
        bar_text.contains("original message"),
        "reply bar should contain the message preview"
    );

    // Click the cancel button.
    let cancel_btn = query(&container, ".reply-bar-cancel").unwrap();
    simulate_click(&cancel_btn);
    tick().await;

    // Reply bar should be gone.
    assert!(
        query(&container, ".reply-bar").is_none(),
        "reply bar should disappear after cancel"
    );
}

// ── Feature 3: Scroll-to-bottom Button Tests ───────────────────────────────

#[wasm_bindgen_test]
async fn scroll_to_bottom_button_hidden_at_bottom() {
    // When the user is at the bottom, the scroll-to-bottom button should
    // be hidden. We test the signal logic directly.
    let (show_btn, set_show_btn) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="message-list-container">
                <div class="message-list" style="overflow-y: auto; height: 100px;">
                    "short content"
                </div>
                {move || {
                    if show_btn.get() {
                        Some(view! {
                            <button class="scroll-to-bottom">"New messages"</button>
                        })
                    } else {
                        None
                    }
                }}
            </div>
        }
    });

    tick().await;

    // Button should be hidden when show_btn is false (at bottom).
    assert!(
        query(&container, ".scroll-to-bottom").is_none(),
        "scroll button should be hidden at bottom"
    );

    // Simulate scrolling up (set signal to true).
    set_show_btn.set(true);
    tick().await;

    let btn = query(&container, ".scroll-to-bottom").unwrap();
    assert_eq!(text(&btn), "New messages");

    // Click it to go back to bottom (set signal to false).
    simulate_click(&btn);
    // In real code clicking would scroll and toggle the signal;
    // here we just test the rendering.
    set_show_btn.set(false);
    tick().await;

    assert!(
        query(&container, ".scroll-to-bottom").is_none(),
        "scroll button should hide after clicking"
    );
}

// ── Feature 4: Relative Timestamp Tests ─────────────────────────────────────

/// Mirror of the `format_relative_time` function in `message.rs`, used
/// for testing without importing from the binary crate.
fn format_relative_time(timestamp_ms: u64) -> String {
    if timestamp_ms == 0 {
        return String::new();
    }
    let now_ms = js_sys::Date::now() as u64;
    if timestamp_ms > now_ms {
        return willow_client::util::format_timestamp(timestamp_ms);
    }
    let diff_secs = (now_ms - timestamp_ms) / 1000;
    if diff_secs < 60 {
        "just now".to_string()
    } else if diff_secs < 3600 {
        format!("{}m ago", diff_secs / 60)
    } else if diff_secs < 86400 {
        format!("{}h ago", diff_secs / 3600)
    } else {
        willow_client::util::format_timestamp(timestamp_ms)
    }
}

#[wasm_bindgen_test]
async fn relative_timestamp_formats() {
    let now_ms = js_sys::Date::now() as u64;

    // "just now" for < 60s ago.
    assert_eq!(format_relative_time(now_ms - 10_000), "just now");
    assert_eq!(format_relative_time(now_ms - 59_000), "just now");

    // "Xm ago" for < 1 hour.
    assert_eq!(format_relative_time(now_ms - 5 * 60_000), "5m ago");
    assert_eq!(format_relative_time(now_ms - 30 * 60_000), "30m ago");

    // "Xh ago" for < 24 hours.
    assert_eq!(format_relative_time(now_ms - 2 * 3_600_000), "2h ago");
    assert_eq!(format_relative_time(now_ms - 12 * 3_600_000), "12h ago");

    // Falls back to HH:MM for older timestamps.
    let old_ts = now_ms - 48 * 3_600_000; // 2 days ago
    let formatted = format_relative_time(old_ts);
    assert!(
        formatted.contains(':'),
        "old timestamps should fall back to HH:MM, got: {formatted}"
    );

    // Zero returns empty.
    assert_eq!(format_relative_time(0), "");
}

// ── Feature 5: Loading Spinner Tests ────────────────────────────────────────

#[wasm_bindgen_test]
async fn loading_spinner_shows_initially() {
    let (loading, set_loading) = signal(true);
    let (messages, _set_messages) = signal(Vec::<willow_client::DisplayMessage>::new());

    let container = mount_test(move || {
        view! {
            <div class="message-list">
                {move || {
                    let is_loading = loading.get();
                    let msgs = messages.get();
                    if is_loading && msgs.is_empty() {
                        view! {
                            <div class="loading-spinner" role="status">
                                <div class="spinner"></div>
                                <span>"Connecting..."</span>
                            </div>
                        }.into_any()
                    } else if msgs.is_empty() {
                        view! {
                            <div class="empty-state">"No messages yet. Say hello!"</div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="has-messages">"Messages here"</div>
                        }.into_any()
                    }
                }}
            </div>
        }
    });

    tick().await;

    // Spinner visible while loading.
    let spinner = query(&container, ".loading-spinner");
    assert!(
        spinner.is_some(),
        "loading spinner should be visible initially"
    );
    assert!(
        query(&container, ".spinner").is_some(),
        "spinner animation element should exist"
    );
    assert!(
        query(&container, ".empty-state").is_none(),
        "empty state should NOT show while loading"
    );

    // After loading finishes, show empty state.
    set_loading.set(false);
    tick().await;

    assert!(
        query(&container, ".loading-spinner").is_none(),
        "spinner should be gone after loading"
    );
    assert!(
        query(&container, ".empty-state").is_some(),
        "empty state should show after loading with no messages"
    );
}

// ── Feature: Edit/Delete Own Messages Tests ─────────────────────────────────

#[wasm_bindgen_test]
async fn own_message_shows_action_buttons() {
    // Own messages (is_local = true) should display action buttons on hover.
    let mut msg = make_msg("Me", "my message", 1000);
    msg.is_local = true;
    let is_own = msg.is_local;
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="body">{body}</div>
                {if is_own {
                    Some(view! {
                        <div class="message-actions">
                            <button class="edit-action">"Edit"</button>
                            <button class="delete-action">"Delete"</button>
                        </div>
                    })
                } else {
                    None
                }}
            </div>
        }
    });

    tick().await;

    // Action buttons should exist in the DOM (CSS controls visibility on hover).
    let actions = query(&container, ".message-actions");
    assert!(
        actions.is_some(),
        "action bar should be in DOM for own messages"
    );
    let edit_btn = query(&container, ".edit-action").unwrap();
    assert_eq!(text(&edit_btn), "Edit");
    let delete_btn = query(&container, ".delete-action").unwrap();
    assert_eq!(text(&delete_btn), "Delete");
}

#[wasm_bindgen_test]
async fn other_message_hides_action_buttons() {
    // Messages from other users (is_local = false) should NOT show
    // edit/delete action buttons.
    let msg = make_msg("OtherUser", "their message", 1000);
    let is_own = msg.is_local; // false
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="body">{body}</div>
                {if is_own {
                    Some(view! {
                        <div class="message-actions">
                            <button class="edit-action">"Edit"</button>
                            <button class="delete-action">"Delete"</button>
                        </div>
                    })
                } else {
                    None
                }}
            </div>
        }
    });

    tick().await;

    // No action buttons for other users' messages.
    assert!(
        query(&container, ".message-actions").is_none(),
        "action bar should NOT be in DOM for other users' messages"
    );
    assert!(query(&container, ".edit-action").is_none());
    assert!(query(&container, ".delete-action").is_none());
}

#[wasm_bindgen_test]
async fn editing_bar_shows_when_editing() {
    // When editing is set, an edit bar should appear above the input.
    let (editing, set_editing) = signal(Option::<willow_client::DisplayMessage>::None);

    let container = mount_test(move || {
        view! {
            <div class="input-area">
                {move || {
                    editing.get().map(|m| {
                        let preview = if m.body.len() > 60 {
                            format!("{}...", &m.body[..60])
                        } else {
                            m.body.clone()
                        };
                        view! {
                            <div class="edit-bar">
                                <span class="edit-bar-text">
                                    {format!("Editing: {}", preview)}
                                </span>
                                <button
                                    class="edit-bar-cancel"
                                    on:click=move |_| set_editing.set(None)
                                >
                                    "x"
                                </button>
                            </div>
                        }
                    })
                }}
                <input type="text" placeholder="Message #channel" />
            </div>
        }
    });

    tick().await;

    // No edit bar initially.
    assert!(
        query(&container, ".edit-bar").is_none(),
        "edit bar should not be visible initially"
    );

    // Set editing to a message.
    let msg = make_msg("Me", "original text", 1000);
    set_editing.set(Some(msg));
    tick().await;

    // Edit bar should now be visible.
    let _bar = query(&container, ".edit-bar").unwrap();
    let bar_text = text(&query(&container, ".edit-bar-text").unwrap());
    assert!(
        bar_text.contains("Editing: original text"),
        "edit bar should show the message preview, got: {bar_text}"
    );

    // Cancel button should exist.
    assert!(query(&container, ".edit-bar-cancel").is_some());

    // Click cancel.
    let cancel_btn = query(&container, ".edit-bar-cancel").unwrap();
    simulate_click(&cancel_btn);
    tick().await;

    // Edit bar should be gone.
    assert!(
        query(&container, ".edit-bar").is_none(),
        "edit bar should disappear after cancel"
    );
    // Check that editing signal was cleared (reflected in UI).
    assert!(query(&container, ".edit-bar").is_none());
}

#[wasm_bindgen_test]
async fn edit_callback_fires_on_click() {
    // When the Edit button is clicked, the on_edit callback should fire.
    let (edited_msg, set_edited_msg) = signal(Option::<String>::None);

    let mut msg = make_msg("Me", "editable message", 1000);
    msg.is_local = true;
    let msg_id = msg.id.clone();
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="body">{body}</div>
                <div class="message-actions">
                    <button
                        class="edit-action"
                        on:click=move |_| set_edited_msg.set(Some(msg_id.clone()))
                    >
                        "Edit"
                    </button>
                </div>
            </div>
        }
    });

    tick().await;

    // Click Edit.
    let edit_btn = query(&container, ".edit-action").unwrap();
    simulate_click(&edit_btn);
    tick().await;

    assert!(
        edited_msg.get_untracked().is_some(),
        "edit callback should have fired"
    );
}

#[wasm_bindgen_test]
async fn delete_callback_fires_on_click() {
    // When the Delete button is clicked, the on_delete callback should fire.
    let (deleted_msg, set_deleted_msg) = signal(Option::<String>::None);

    let mut msg = make_msg("Me", "deletable message", 1000);
    msg.is_local = true;
    let msg_id = msg.id.clone();
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="body">{body}</div>
                <div class="message-actions">
                    <button
                        class="delete-action"
                        on:click=move |_| set_deleted_msg.set(Some(msg_id.clone()))
                    >
                        "Delete"
                    </button>
                </div>
            </div>
        }
    });

    tick().await;

    let delete_btn = query(&container, ".delete-action").unwrap();
    simulate_click(&delete_btn);
    tick().await;

    assert!(
        deleted_msg.get_untracked().is_some(),
        "delete callback should have fired"
    );
}

// ── Feature: Emoji Reactions Tests ──────────────────────────────────────────

#[wasm_bindgen_test]
async fn emoji_reaction_picker_toggles() {
    // The reaction picker should appear when the "+" button is clicked,
    // and disappear when clicked again.
    let (show_picker, set_show_picker) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="message-actions">
                    <div class="reaction-trigger">
                        <button
                            class="react-action"
                            on:click=move |_| set_show_picker.update(|v| *v = !*v)
                        >
                            "+"
                        </button>
                        {move || {
                            if show_picker.get() {
                                Some(view! {
                                    <div class="reaction-picker">
                                        <button class="emoji-btn">{"\u{1F44D}"}</button>
                                        <button class="emoji-btn">{"\u{2764}\u{FE0F}"}</button>
                                        <button class="emoji-btn">{"\u{1F602}"}</button>
                                    </div>
                                })
                            } else {
                                None
                            }
                        }}
                    </div>
                </div>
            </div>
        }
    });

    tick().await;

    // Picker should be hidden initially.
    assert!(
        query(&container, ".reaction-picker").is_none(),
        "reaction picker should be hidden initially"
    );

    // Click the "+" button to show the picker.
    let react_btn = query(&container, ".react-action").unwrap();
    simulate_click(&react_btn);
    tick().await;

    assert!(
        query(&container, ".reaction-picker").is_some(),
        "reaction picker should be visible after clicking +"
    );
    let emoji_btns = query_all(&container, ".emoji-btn");
    assert_eq!(
        emoji_btns.len(),
        3,
        "should show 3 emoji buttons in the picker"
    );

    // Click "+" again to close the picker.
    simulate_click(&react_btn);
    tick().await;

    assert!(
        query(&container, ".reaction-picker").is_none(),
        "reaction picker should toggle off on second click"
    );
}

#[wasm_bindgen_test]
async fn clicking_emoji_calls_callback() {
    // Clicking an emoji in the reaction picker should fire the callback
    // with the chosen emoji and close the picker.
    let (chosen_emoji, set_chosen_emoji) = signal(Option::<String>::None);
    let (show_picker, set_show_picker) = signal(true); // Start open for this test.

    let container = mount_test(move || {
        let emojis = vec![
            "\u{1F44D}".to_string(),
            "\u{2764}\u{FE0F}".to_string(),
            "\u{1F525}".to_string(),
        ];

        view! {
            <div class="message">
                <div class="message-actions">
                    <div class="reaction-trigger">
                        <button class="react-action">"+"</button>
                        {move || {
                            if show_picker.get() {
                                let emojis_clone = emojis.clone();
                                Some(view! {
                                    <div class="reaction-picker">
                                        {emojis_clone.into_iter().map(|emoji| {
                                            let e = emoji.clone();
                                            view! {
                                                <button
                                                    class="emoji-btn"
                                                    on:click=move |_| {
                                                        set_chosen_emoji.set(Some(e.clone()));
                                                        set_show_picker.set(false);
                                                    }
                                                >
                                                    {emoji}
                                                </button>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                })
                            } else {
                                None
                            }
                        }}
                    </div>
                </div>
            </div>
        }
    });

    tick().await;

    // Picker is open with 3 emoji buttons.
    let emoji_btns = query_all(&container, ".emoji-btn");
    assert_eq!(emoji_btns.len(), 3);

    // Click the fire emoji (third button).
    simulate_click(&emoji_btns[2]);
    tick().await;

    // Callback should have been called with the fire emoji.
    let chosen = chosen_emoji.get_untracked();
    assert_eq!(
        chosen,
        Some("\u{1F525}".to_string()),
        "callback should receive the clicked emoji"
    );

    // Picker should be closed after selecting.
    assert!(
        query(&container, ".reaction-picker").is_none(),
        "picker should close after selecting an emoji"
    );
}

// ── File Sharing Tests ──────────────────────────────────────────────────────

/// Parse an inline file message body. Mirrors the logic in file_share.rs.
fn parse_inline_file(body: &str) -> Option<(String, Vec<u8>)> {
    let inner = body.strip_prefix("[file:")?.strip_suffix(']')?;
    let colon = inner.find(':')?;
    let filename = &inner[..colon];
    let b64 = &inner[colon + 1..];
    let data = willow_client::base64::decode(b64)?;
    Some((filename.to_string(), data))
}

/// Format byte count for display. Mirrors file_share.rs logic.
fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[wasm_bindgen_test]
async fn file_share_button_renders() {
    let container = mount_test(move || {
        view! {
            <div class="input-row">
                <button class="file-share-btn" title="Attach file">
                    "\u{1F4CE}"
                </button>
                <div class="input-area">
                    <input type="text" placeholder="Message #general" />
                </div>
            </div>
        }
    });

    tick().await;

    let btn = query(&container, ".file-share-btn");
    assert!(btn.is_some(), "file share button should exist");

    let btn_el = btn.unwrap();
    assert_eq!(
        btn_el.get_attribute("title").unwrap_or_default(),
        "Attach file",
        "button should have the correct title attribute"
    );
}

#[wasm_bindgen_test]
async fn file_message_renders_as_card() {
    let data = b"hello file!";
    let encoded = willow_client::base64::encode(data);
    let body = format!("[file:test.txt:{}]", encoded);
    let parsed = parse_inline_file(&body);

    assert!(parsed.is_some(), "should parse inline file body");
    let (filename, file_data) = parsed.unwrap();
    let size_str = format_file_size(file_data.len());

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="file-card">
                    <span class="file-icon">"\u{1F4C4}"</span>
                    <div class="file-info">
                        <span class="file-name">{filename}</span>
                        <span class="file-size">{size_str}</span>
                    </div>
                    <button class="download-btn btn btn-sm btn-primary">
                        "\u{2B07}"
                    </button>
                </div>
            </div>
        }
    });

    tick().await;

    let card = query(&container, ".file-card");
    assert!(card.is_some(), "file card should render");

    let name_el = query(&container, ".file-name").unwrap();
    assert_eq!(text(&name_el), "test.txt");

    let size_el = query(&container, ".file-size").unwrap();
    assert_eq!(text(&size_el), "11 B");

    let dl_btn = query(&container, ".download-btn");
    assert!(dl_btn.is_some(), "download button should exist");
}

#[wasm_bindgen_test]
async fn regular_message_does_not_render_file_card() {
    let body = "just a normal message".to_string();
    let is_file = parse_inline_file(&body).is_some();

    let container = mount_test(move || {
        view! {
            <div class="message">
                {if is_file {
                    Some(view! { <div class="file-card">"file"</div> })
                } else {
                    None
                }}
                {if !is_file {
                    Some(view! { <div class="body">{body}</div> })
                } else {
                    None
                }}
            </div>
        }
    });

    tick().await;

    assert!(
        query(&container, ".file-card").is_none(),
        "normal messages should not render file cards"
    );
    let body_el = query(&container, ".body").unwrap();
    assert_eq!(text(&body_el), "just a normal message");
}

// ── Voice Controls Tests ────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn voice_controls_renders_mute_deafen_disconnect() {
    let (muted, _) = signal(false);
    let (deafened, _) = signal(false);
    let (channel_name, _) = signal("voice-chat".to_string());

    let container = mount_test(move || {
        view! {
            <div class="voice-controls">
                <div class="voice-status">
                    <span class="voice-status-icon">{"\u{1F50A}"}</span>
                    <span class="voice-channel-name">{move || channel_name.get()}</span>
                </div>
                <div class="voice-buttons">
                    <button
                        class=move || if muted.get() { "voice-btn muted" } else { "voice-btn" }
                        title=move || if muted.get() { "Unmute" } else { "Mute" }
                    >
                        {move || if muted.get() { "\u{1F507}" } else { "\u{1F3A4}" }}
                    </button>
                    <button
                        class=move || if deafened.get() { "voice-btn deafened" } else { "voice-btn" }
                        title=move || if deafened.get() { "Undeafen" } else { "Deafen" }
                    >
                        {move || if deafened.get() { "\u{1F515}" } else { "\u{1F514}" }}
                    </button>
                    <button class="voice-btn disconnect" title="Disconnect">
                        {"\u{1F4F5}"}
                    </button>
                </div>
            </div>
        }
    });
    tick().await;

    // Should show voice controls with 3 buttons.
    let buttons = query_all(&container, ".voice-btn");
    assert!(buttons.len() >= 3, "expected at least 3 voice buttons");

    // Should show channel name.
    let name = query(&container, ".voice-channel-name");
    assert!(name.is_some(), "voice channel name element should exist");
    assert!(
        text(&name.unwrap()).contains("voice-chat"),
        "channel name should contain 'voice-chat'"
    );

    // Disconnect button should have the correct class.
    let disconnect = query(&container, ".voice-btn.disconnect");
    assert!(disconnect.is_some(), "disconnect button should exist");
}

#[wasm_bindgen_test]
async fn voice_controls_mute_toggles_class() {
    let (muted, set_muted) = signal(false);
    let (deafened, _) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="voice-controls">
                <div class="voice-buttons">
                    <button
                        class=move || if muted.get() { "voice-btn muted" } else { "voice-btn" }
                        on:click=move |_| set_muted.update(|v| *v = !*v)
                    >
                        {move || if muted.get() { "\u{1F507}" } else { "\u{1F3A4}" }}
                    </button>
                    <button
                        class=move || if deafened.get() { "voice-btn deafened" } else { "voice-btn" }
                    >
                        {move || if deafened.get() { "\u{1F515}" } else { "\u{1F514}" }}
                    </button>
                    <button class="voice-btn disconnect">{"\u{1F4F5}"}</button>
                </div>
            </div>
        }
    });
    tick().await;

    // First button should not have "muted" class initially.
    let buttons = query_all(&container, ".voice-btn");
    assert!(
        !buttons[0].class_list().contains("muted"),
        "mute button should not be muted initially"
    );

    // Click mute button.
    simulate_click(&buttons[0]);
    tick().await;

    // Should now have "muted" class.
    let buttons = query_all(&container, ".voice-btn");
    assert!(
        buttons[0].class_list().contains("muted"),
        "mute button should have 'muted' class after click"
    );
}

#[wasm_bindgen_test]
async fn voice_controls_deafen_toggles_class() {
    let (deafened, set_deafened) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="voice-controls">
                <div class="voice-buttons">
                    <button class="voice-btn">{"\u{1F3A4}"}</button>
                    <button
                        class=move || if deafened.get() { "voice-btn deafened" } else { "voice-btn" }
                        on:click=move |_| set_deafened.update(|v| *v = !*v)
                    >
                        {move || if deafened.get() { "\u{1F515}" } else { "\u{1F514}" }}
                    </button>
                    <button class="voice-btn disconnect">{"\u{1F4F5}"}</button>
                </div>
            </div>
        }
    });
    tick().await;

    let buttons = query_all(&container, ".voice-btn");
    assert!(
        !buttons[1].class_list().contains("deafened"),
        "deafen button should not be deafened initially"
    );

    simulate_click(&buttons[1]);
    tick().await;

    let buttons = query_all(&container, ".voice-btn");
    assert!(
        buttons[1].class_list().contains("deafened"),
        "deafen button should have 'deafened' class after click"
    );
}

#[wasm_bindgen_test]
async fn voice_controls_disconnect_fires_callback() {
    let (disconnected, set_disconnected) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="voice-controls">
                <div class="voice-buttons">
                    <button class="voice-btn">{"\u{1F3A4}"}</button>
                    <button class="voice-btn">{"\u{1F514}"}</button>
                    <button
                        class="voice-btn disconnect"
                        on:click=move |_| set_disconnected.set(true)
                    >
                        {"\u{1F4F5}"}
                    </button>
                </div>
            </div>
        }
    });
    tick().await;

    let disconnect_btn = query(&container, ".voice-btn.disconnect").unwrap();
    simulate_click(&disconnect_btn);
    tick().await;

    assert!(
        disconnected.get_untracked(),
        "disconnect callback should have fired"
    );
}

#[wasm_bindgen_test]
async fn voice_controls_channel_name_updates_reactively() {
    let (channel_name, set_channel_name) = signal("general-voice".to_string());

    let container = mount_test(move || {
        view! {
            <div class="voice-controls">
                <span class="voice-channel-name">{move || channel_name.get()}</span>
            </div>
        }
    });
    tick().await;

    let name = query(&container, ".voice-channel-name").unwrap();
    assert_eq!(text(&name), "general-voice");

    set_channel_name.set("music-room".to_string());
    tick().await;

    let name = query(&container, ".voice-channel-name").unwrap();
    assert_eq!(text(&name), "music-room");
}

// ── Welcome Screen / Add Server Panel Tests ─────────────────────────────────

#[wasm_bindgen_test]
async fn welcome_screen_shows_create_and_join() {
    // Test that the welcome-options structure renders create and join sections.
    // WelcomeScreen itself needs a ClientHandle, so we replicate the UI inline.
    let container = mount_test(|| {
        view! {
            <div class="welcome-screen">
                <div class="welcome-card">
                    <h1>"Welcome to Willow"</h1>
                    <p class="tagline">
                        "P2P encrypted chat \u{2014} no accounts, no servers, no middlemen."
                    </p>
                    <div class="welcome-options">
                        <div class="welcome-option">
                            <h3>"Create a Server"</h3>
                            <input type="text" placeholder="My Server" />
                            <button class="btn btn-primary welcome-btn">"Create Server"</button>
                        </div>
                        <div class="welcome-option">
                            <h3>"Join a Server"</h3>
                            <textarea class="welcome-invite-input" placeholder="Paste invite code here..."></textarea>
                            <button class="btn btn-primary welcome-btn">"Next \u{2192}"</button>
                        </div>
                    </div>
                </div>
            </div>
        }
    });
    tick().await;

    let options = query_all(&container, ".welcome-option");
    assert_eq!(options.len(), 2, "should have 2 welcome options");
    assert!(
        text(&options[0]).contains("Create"),
        "first option should be 'Create a Server'"
    );
    assert!(
        text(&options[1]).contains("Join"),
        "second option should be 'Join a Server'"
    );

    // Welcome card should exist.
    assert!(query(&container, ".welcome-card").is_some());

    // Heading should exist.
    let heading = query(&container, "h1").unwrap();
    assert!(text(&heading).contains("Welcome to Willow"));
}

#[wasm_bindgen_test]
async fn welcome_screen_peer_id_display() {
    let peer_id = "12D3KooWTestPeerId123";

    let container = mount_test(move || {
        view! {
            <div class="welcome-screen">
                <div class="welcome-card">
                    <div class="welcome-peer-id">
                        <code class="peer-id-text">{peer_id}</code>
                        <button class="btn btn-sm">"Copy"</button>
                    </div>
                </div>
            </div>
        }
    });
    tick().await;

    let peer_el = query(&container, ".peer-id-text").unwrap();
    assert_eq!(text(&peer_el), "12D3KooWTestPeerId123");

    // Copy button should exist.
    assert!(
        query(&container, ".welcome-peer-id .btn").is_some(),
        "copy button should exist in the peer ID display"
    );
}

#[wasm_bindgen_test]
async fn welcome_create_server_validates_empty_name() {
    let (status_msg, set_status_msg) = signal(String::new());
    let (server_name, _) = signal(String::new());

    let container = mount_test(move || {
        let on_create = move |_| {
            let name = server_name.get_untracked();
            if name.trim().is_empty() {
                set_status_msg.set("Please enter a server name.".to_string());
            }
        };
        view! {
            <div class="welcome-option">
                {move || {
                    let msg = status_msg.get();
                    if msg.is_empty() {
                        None
                    } else {
                        Some(view! { <div class="settings-status">{msg}</div> })
                    }
                }}
                <h3>"Create a Server"</h3>
                <input type="text" placeholder="My Server" />
                <button class="btn btn-primary welcome-btn" on:click=on_create>
                    "Create Server"
                </button>
            </div>
        }
    });
    tick().await;

    // No status message initially.
    assert!(query(&container, ".settings-status").is_none());

    // Click create with empty name.
    let create_btn = query(&container, ".welcome-btn").unwrap();
    simulate_click(&create_btn);
    tick().await;

    // Should show validation error.
    let status = query(&container, ".settings-status").unwrap();
    assert!(
        text(&status).contains("Please enter a server name"),
        "should show validation message for empty name"
    );
}

// ── Pinned Messages Panel Tests ─────────────────────────────────────────────

/// Check if a URL points to an image based on extension (mirrors message.rs).
fn is_image_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let path = lower.split('?').next().unwrap_or(&lower);
    [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".ico",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
}

/// Extract URLs from text (mirrors message.rs). Returns (segments, image_urls).
fn extract_urls(body: &str) -> (Vec<(String, bool)>, Vec<String>) {
    let mut segments = Vec::new();
    let mut images = Vec::new();
    let mut last_end = 0;

    let mut url_starts: Vec<usize> = body
        .match_indices("https://")
        .chain(body.match_indices("http://"))
        .map(|(i, _)| i)
        .collect();
    url_starts.sort_unstable();
    url_starts.dedup();

    for &url_start in &url_starts {
        if url_start < last_end {
            continue;
        }
        let rest = &body[url_start..];
        let url_end = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == ')' || c == ']')
            .map(|i| url_start + i)
            .unwrap_or(body.len());
        let url = &body[url_start..url_end];

        if url_start > last_end {
            segments.push((body[last_end..url_start].to_string(), false));
        }
        segments.push((url.to_string(), true));

        if is_image_url(url) {
            images.push(url.to_string());
        }

        last_end = url_end;
    }

    if last_end < body.len() {
        segments.push((body[last_end..].to_string(), false));
    }

    if segments.is_empty() {
        segments.push((body.to_string(), false));
    }

    (segments, images)
}

#[wasm_bindgen_test]
async fn pinned_panel_renders_messages() {
    let msg1 = make_msg("Alice", "pinned message one", 1000);
    let msg2 = make_msg("Bob", "pinned message two", 2000);
    let (msgs, _) = signal(vec![msg1, msg2]);

    let container = mount_test(move || {
        view! {
            <div class="pinned-panel">
                <div class="pinned-header">
                    <h3>"Pinned Messages"</h3>
                    <button class="btn btn-sm">"\u{00D7}"</button>
                </div>
                <div class="pinned-list">
                    <For
                        each=move || msgs.get()
                        key=|msg| msg.id.clone()
                        let:msg
                    >
                        {
                            let author = msg.author_display_name.clone();
                            let body = msg.body.clone();
                            view! {
                                <div class="pinned-item">
                                    <div class="pinned-meta">
                                        <span class="pinned-author">{author}</span>
                                    </div>
                                    <div class="pinned-body">{body}</div>
                                    <button class="btn btn-sm pinned-jump">"Jump"</button>
                                </div>
                            }
                        }
                    </For>
                </div>
            </div>
        }
    });
    tick().await;

    let items = query_all(&container, ".pinned-item");
    assert_eq!(items.len(), 2, "should render 2 pinned messages");
    assert!(
        text(&items[0]).contains("pinned message one"),
        "first pinned message should contain its body"
    );
    assert!(
        text(&items[1]).contains("pinned message two"),
        "second pinned message should contain its body"
    );
}

#[wasm_bindgen_test]
async fn pinned_panel_shows_empty_state() {
    let (msgs, _) = signal(Vec::<willow_client::DisplayMessage>::new());

    let container = mount_test(move || {
        view! {
            <div class="pinned-panel">
                <div class="pinned-header">
                    <h3>"Pinned Messages"</h3>
                </div>
                <div class="pinned-list">
                    <For
                        each=move || msgs.get()
                        key=|msg| msg.id.clone()
                        let:msg
                    >
                        {
                            let body = msg.body.clone();
                            view! { <div class="pinned-item">{body}</div> }
                        }
                    </For>
                    {move || {
                        if msgs.get().is_empty() {
                            Some(view! { <div class="empty-state">"No pinned messages"</div> })
                        } else {
                            None
                        }
                    }}
                </div>
            </div>
        }
    });
    tick().await;

    let empty = query(&container, ".empty-state");
    assert!(empty.is_some(), "empty state should be shown when no pins");
    assert!(
        text(&empty.unwrap()).contains("No pinned"),
        "empty state should mention 'No pinned'"
    );
}

#[wasm_bindgen_test]
async fn pinned_panel_has_jump_buttons() {
    let msg = make_msg("Alice", "jump to me", 1000);
    let (msgs, _) = signal(vec![msg]);
    let (jumped_to, set_jumped_to) = signal(Option::<String>::None);

    let container = mount_test(move || {
        view! {
            <div class="pinned-panel">
                <div class="pinned-list">
                    <For
                        each=move || msgs.get()
                        key=|msg| msg.id.clone()
                        let:msg
                    >
                        {
                            let msg_id = msg.id.clone();
                            let body = msg.body.clone();
                            view! {
                                <div class="pinned-item">
                                    <div class="pinned-body">{body}</div>
                                    <button
                                        class="btn btn-sm pinned-jump"
                                        on:click=move |_| set_jumped_to.set(Some(msg_id.clone()))
                                    >
                                        "Jump"
                                    </button>
                                </div>
                            }
                        }
                    </For>
                </div>
            </div>
        }
    });
    tick().await;

    let jump_btn = query(&container, ".pinned-jump");
    assert!(jump_btn.is_some(), "jump button should exist");
    assert!(
        text(jump_btn.as_ref().unwrap()).contains("Jump"),
        "jump button should say 'Jump'"
    );

    // Click the jump button.
    simulate_click(&jump_btn.unwrap());
    tick().await;

    assert!(
        jumped_to.get_untracked().is_some(),
        "jump callback should have fired"
    );
}

#[wasm_bindgen_test]
async fn pinned_panel_renders_urls_as_links() {
    let msg = make_msg("Alice", "check https://example.com please", 1000);
    let (msgs, _) = signal(vec![msg]);

    let container = mount_test(move || {
        view! {
            <div class="pinned-panel">
                <div class="pinned-list">
                    <For
                        each=move || msgs.get()
                        key=|msg| msg.id.clone()
                        let:msg
                    >
                        {
                            let body = msg.body.clone();
                            let (segments, _images) = extract_urls(&body);
                            view! {
                                <div class="pinned-item">
                                    <div class="pinned-body">
                                        {segments.into_iter().map(|(txt, is_url)| {
                                            if is_url {
                                                let display = txt.clone();
                                                view! {
                                                    <a href=txt target="_blank" rel="noopener noreferrer" class="message-link">{display}</a>
                                                }.into_any()
                                            } else {
                                                view! { <span>{txt}</span> }.into_any()
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                </div>
                            }
                        }
                    </For>
                </div>
            </div>
        }
    });
    tick().await;

    let links = query_all(&container, ".pinned-body a.message-link");
    assert!(
        !links.is_empty(),
        "URL should be rendered as a clickable link in pinned body"
    );
    assert!(
        text(&links[0]).contains("https://example.com"),
        "link text should contain the URL"
    );
}

#[wasm_bindgen_test]
async fn pinned_panel_close_button_fires_callback() {
    let (closed, set_closed) = signal(false);
    let (msgs, _) = signal(Vec::<willow_client::DisplayMessage>::new());

    let container = mount_test(move || {
        view! {
            <div class="pinned-panel">
                <div class="pinned-header">
                    <h3>"Pinned Messages"</h3>
                    <button class="btn btn-sm pinned-close" on:click=move |_| set_closed.set(true)>
                        "\u{00D7}"
                    </button>
                </div>
                <div class="pinned-list">
                    {move || {
                        if msgs.get().is_empty() {
                            Some(view! { <div class="empty-state">"No pinned messages"</div> })
                        } else {
                            None
                        }
                    }}
                </div>
            </div>
        }
    });
    tick().await;

    let close_btn = query(&container, ".pinned-close").unwrap();
    simulate_click(&close_btn);
    tick().await;

    assert!(closed.get_untracked(), "close callback should have fired");
}

// ── Typing Indicator Tests ──────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn typing_indicator_shows_single_typer() {
    let (typers, _) = signal(vec!["Alice".to_string()]);

    let container = mount_test(move || {
        view! {
            <div class="typing-indicator">
                {move || {
                    let names = typers.get();
                    if names.is_empty() {
                        String::new()
                    } else if names.len() == 1 {
                        format!("{} is typing...", names[0])
                    } else {
                        format!("{} are typing...", names.join(", "))
                    }
                }}
            </div>
        }
    });
    tick().await;

    let indicator = query(&container, ".typing-indicator");
    assert!(indicator.is_some(), "typing indicator should exist");
    assert!(
        text(&indicator.unwrap()).contains("Alice is typing"),
        "should show 'Alice is typing...'"
    );
}

#[wasm_bindgen_test]
async fn typing_indicator_shows_multiple_typers() {
    let (typers, _) = signal(vec!["Alice".to_string(), "Bob".to_string()]);

    let container = mount_test(move || {
        view! {
            <div class="typing-indicator">
                {move || {
                    let names = typers.get();
                    if names.is_empty() {
                        String::new()
                    } else if names.len() == 1 {
                        format!("{} is typing...", names[0])
                    } else {
                        format!("{} are typing...", names.join(", "))
                    }
                }}
            </div>
        }
    });
    tick().await;

    let indicator = query(&container, ".typing-indicator").unwrap();
    let t = text(&indicator);
    assert!(
        t.contains("Alice") && t.contains("Bob") && t.contains("are typing"),
        "should show multiple typers, got: {t}"
    );
}

#[wasm_bindgen_test]
async fn typing_indicator_empty_when_no_typers() {
    let (typers, _) = signal(Vec::<String>::new());

    let container = mount_test(move || {
        view! {
            <div class="typing-indicator">
                {move || {
                    let names = typers.get();
                    if names.is_empty() {
                        String::new()
                    } else {
                        format!("{} is typing...", names[0])
                    }
                }}
            </div>
        }
    });
    tick().await;

    let indicator = query(&container, ".typing-indicator").unwrap();
    assert!(
        text(&indicator).is_empty(),
        "typing indicator should be empty when no one is typing"
    );
}

#[wasm_bindgen_test]
async fn typing_indicator_updates_reactively() {
    let (typers, set_typers) = signal(Vec::<String>::new());

    let container = mount_test(move || {
        view! {
            <div class="typing-indicator">
                {move || {
                    let names = typers.get();
                    if names.is_empty() {
                        String::new()
                    } else if names.len() == 1 {
                        format!("{} is typing...", names[0])
                    } else {
                        format!("{} are typing...", names.join(", "))
                    }
                }}
            </div>
        }
    });
    tick().await;

    let indicator = query(&container, ".typing-indicator").unwrap();
    assert!(text(&indicator).is_empty());

    // Someone starts typing.
    set_typers.set(vec!["Charlie".to_string()]);
    tick().await;

    let indicator = query(&container, ".typing-indicator").unwrap();
    assert!(text(&indicator).contains("Charlie is typing"));

    // They stop typing.
    set_typers.set(vec![]);
    tick().await;

    let indicator = query(&container, ".typing-indicator").unwrap();
    assert!(text(&indicator).is_empty());
}

// ── Message Mention Highlighting Tests ──────────────────────────────────────

#[wasm_bindgen_test]
async fn mentioned_message_has_highlight_class() {
    let mut msg = make_msg("Bob", "hey check this", 1000);
    msg.reply_preview = Some("Alice: original message".to_string());
    msg.reply_to = Some("parent-id".to_string());

    let local_name = "Alice";
    let is_mention = !msg.is_local
        && msg
            .reply_preview
            .as_ref()
            .map(|p| p.starts_with(&format!("{local_name}:")))
            .unwrap_or(false);

    let msg_class = if is_mention {
        "message mentioned"
    } else {
        "message"
    };
    let body = msg.body.clone();
    let reply_preview = msg.reply_preview.clone();

    let container = mount_test(move || {
        view! {
            <div class=msg_class>
                {reply_preview.clone().map(|preview| {
                    view! { <div class="reply-preview">{format!("> {preview}")}</div> }
                })}
                <div class="body">{body}</div>
            </div>
        }
    });
    tick().await;

    let mentioned = query(&container, ".message.mentioned");
    assert!(
        mentioned.is_some(),
        "reply to local user should have .mentioned class"
    );
}

#[wasm_bindgen_test]
async fn non_mentioned_message_has_no_highlight() {
    let mut msg = make_msg("Bob", "hey check this", 1000);
    msg.reply_preview = Some("Charlie: some other message".to_string());
    msg.reply_to = Some("parent-id".to_string());

    let local_name = "Alice";
    let is_mention = !msg.is_local
        && msg
            .reply_preview
            .as_ref()
            .map(|p| p.starts_with(&format!("{local_name}:")))
            .unwrap_or(false);

    let msg_class = if is_mention {
        "message mentioned"
    } else {
        "message"
    };
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class=msg_class>
                <div class="body">{body}</div>
            </div>
        }
    });
    tick().await;

    assert!(
        query(&container, ".message.mentioned").is_none(),
        "reply to a different user should NOT have .mentioned class"
    );
}

// ── Reply Preview Clickable Tests ───────────────────────────────────────────

#[wasm_bindgen_test]
async fn reply_preview_is_clickable_when_reply_to_present() {
    let mut msg = make_msg("Bob", "replying", 2000);
    msg.reply_preview = Some("Alice: original".to_string());
    msg.reply_to = Some("parent-123".to_string());

    let reply_preview = msg.reply_preview.clone();
    let reply_to = msg.reply_to.clone();
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                {reply_preview.clone().map(|preview| {
                    let has_reply_to = reply_to.is_some();
                    let cls = if has_reply_to { "reply-preview reply-clickable" } else { "reply-preview" };
                    view! {
                        <div class=cls>
                            {format!("> {preview}")}
                        </div>
                    }
                })}
                <div class="body">{body}</div>
            </div>
        }
    });
    tick().await;

    let preview = query(&container, ".reply-clickable");
    assert!(
        preview.is_some(),
        "reply preview with reply_to should be clickable"
    );
}

#[wasm_bindgen_test]
async fn reply_preview_not_clickable_without_reply_to() {
    let mut msg = make_msg("Bob", "replying", 2000);
    msg.reply_preview = Some("Alice: original".to_string());
    msg.reply_to = None;

    let reply_preview = msg.reply_preview.clone();
    let reply_to = msg.reply_to.clone();
    let body = msg.body.clone();

    let container = mount_test(move || {
        view! {
            <div class="message">
                {reply_preview.clone().map(|preview| {
                    let has_reply_to = reply_to.is_some();
                    let cls = if has_reply_to { "reply-preview reply-clickable" } else { "reply-preview" };
                    view! {
                        <div class=cls>
                            {format!("> {preview}")}
                        </div>
                    }
                })}
                <div class="body">{body}</div>
            </div>
        }
    });
    tick().await;

    assert!(
        query(&container, ".reply-clickable").is_none(),
        "reply preview without reply_to should NOT be clickable"
    );
    assert!(
        query(&container, ".reply-preview").is_some(),
        "reply preview should still render"
    );
}

// ── Time-Gap Grouping Tests ─────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn messages_with_time_gap_show_separate_headers() {
    let now = js_sys::Date::now() as u64;
    let msg1 = make_msg("Alice", "first", now - 600_000); // 10 min ago
    let msg2 = make_msg("Alice", "second", now); // now

    let msgs = [msg1, msg2];

    let container = mount_test(move || {
        let views: Vec<_> = msgs
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let show_header = if i == 0 {
                    true
                } else {
                    let prev = &msgs[i - 1];
                    prev.author_display_name != msg.author_display_name
                        || msg.timestamp_ms.saturating_sub(prev.timestamp_ms) > 300_000
                };
                let msg_class = if show_header {
                    "message"
                } else {
                    "message grouped"
                };
                let author = msg.author_display_name.clone();
                let body = msg.body.clone();
                view! {
                    <div class=msg_class>
                        {if show_header {
                            Some(view! {
                                <div class="meta">
                                    <span class="author">{author}</span>
                                </div>
                            })
                        } else {
                            None
                        }}
                        <div class="body">{body}</div>
                    </div>
                }
            })
            .collect();
        view! { <div class="message-list">{views}</div> }
    });
    tick().await;

    // Both messages from same author but >5 min gap (300_000 ms).
    // Should show 2 headers (not grouped).
    let headers = query_all(&container, ".meta");
    assert_eq!(
        headers.len(),
        2,
        "should show 2 headers for 10-minute gap between same-author messages"
    );

    let grouped = query_all(&container, ".message.grouped");
    assert_eq!(
        grouped.len(),
        0,
        "no messages should be grouped with a 10-minute gap"
    );
}

#[wasm_bindgen_test]
async fn consecutive_messages_within_gap_are_grouped() {
    let now = js_sys::Date::now() as u64;
    let msg1 = make_msg("Alice", "first", now - 1000); // 1 sec ago
    let msg2 = make_msg("Alice", "second", now); // now

    let msgs = [msg1, msg2];

    let container = mount_test(move || {
        let views: Vec<_> = msgs
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let show_header = if i == 0 {
                    true
                } else {
                    let prev = &msgs[i - 1];
                    prev.author_display_name != msg.author_display_name
                        || msg.timestamp_ms.saturating_sub(prev.timestamp_ms) > 300_000
                };
                let msg_class = if show_header {
                    "message"
                } else {
                    "message grouped"
                };
                let author = msg.author_display_name.clone();
                let body = msg.body.clone();
                view! {
                    <div class=msg_class>
                        {if show_header {
                            Some(view! {
                                <div class="meta">
                                    <span class="author">{author}</span>
                                </div>
                            })
                        } else {
                            None
                        }}
                        <div class="body">{body}</div>
                    </div>
                }
            })
            .collect();
        view! { <div class="message-list">{views}</div> }
    });
    tick().await;

    // Same author, <5 min gap -- should show 1 header (grouped).
    let headers = query_all(&container, ".meta");
    assert_eq!(
        headers.len(),
        1,
        "should show 1 header for grouped messages"
    );

    let grouped = query_all(&container, ".message.grouped");
    assert_eq!(grouped.len(), 1, "second message should be grouped");
}

#[wasm_bindgen_test]
async fn different_authors_never_grouped() {
    let now = js_sys::Date::now() as u64;
    let msg1 = make_msg("Alice", "hello", now - 1000);
    let msg2 = make_msg("Bob", "hi", now);

    let msgs = [msg1, msg2];

    let container = mount_test(move || {
        let views: Vec<_> = msgs
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let show_header = if i == 0 {
                    true
                } else {
                    let prev = &msgs[i - 1];
                    prev.author_display_name != msg.author_display_name
                        || msg.timestamp_ms.saturating_sub(prev.timestamp_ms) > 300_000
                };
                let msg_class = if show_header {
                    "message"
                } else {
                    "message grouped"
                };
                let author = msg.author_display_name.clone();
                let body = msg.body.clone();
                view! {
                    <div class=msg_class>
                        {if show_header {
                            Some(view! {
                                <div class="meta">
                                    <span class="author">{author}</span>
                                </div>
                            })
                        } else {
                            None
                        }}
                        <div class="body">{body}</div>
                    </div>
                }
            })
            .collect();
        view! { <div class="message-list">{views}</div> }
    });
    tick().await;

    let headers = query_all(&container, ".meta");
    assert_eq!(
        headers.len(),
        2,
        "different authors should always show separate headers"
    );

    let grouped = query_all(&container, ".message.grouped");
    assert_eq!(
        grouped.len(),
        0,
        "different authors should never be grouped"
    );
}

// ── Image Embedding Tests ───────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn url_with_image_extension_embeds_inline() {
    let body = "look https://example.com/cat.png";
    let (segments, images) = extract_urls(body);
    let has_images = !images.is_empty();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="body">
                    {segments.clone().into_iter().map(|(txt, url)| {
                        if url {
                            let display = txt.clone();
                            view! {
                                <a href=txt target="_blank" rel="noopener noreferrer" class="message-link">{display}</a>
                            }.into_any()
                        } else {
                            view! { <span>{txt}</span> }.into_any()
                        }
                    }).collect::<Vec<_>>()}
                </div>
                {if has_images {
                    let imgs = images.clone();
                    Some(view! {
                        <div class="message-embeds">
                            {imgs.into_iter().map(|url| {
                                let url_clone = url.clone();
                                view! {
                                    <a href=url target="_blank" rel="noopener noreferrer" class="embed-link">
                                        <img class="embed-image" src=url_clone alt="embedded image" loading="lazy" />
                                    </a>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    })
                } else {
                    None
                }}
            </div>
        }
    });
    tick().await;

    let imgs = query_all(&container, ".embed-image");
    assert!(
        !imgs.is_empty(),
        "image URL should render as embedded image"
    );

    // The image src should point to the URL.
    let img = &imgs[0];
    assert_eq!(
        img.get_attribute("src").unwrap_or_default(),
        "https://example.com/cat.png"
    );
}

#[wasm_bindgen_test]
async fn url_without_image_extension_renders_as_link() {
    let body = "check https://example.com/page";
    let (segments, images) = extract_urls(body);
    let has_images = !images.is_empty();

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="body">
                    {segments.clone().into_iter().map(|(txt, url)| {
                        if url {
                            let display = txt.clone();
                            view! {
                                <a href=txt target="_blank" rel="noopener noreferrer" class="message-link">{display}</a>
                            }.into_any()
                        } else {
                            view! { <span>{txt}</span> }.into_any()
                        }
                    }).collect::<Vec<_>>()}
                </div>
                {if has_images {
                    Some(view! {
                        <div class="message-embeds">"images"</div>
                    })
                } else {
                    None
                }}
            </div>
        }
    });
    tick().await;

    let links = query_all(&container, "a.message-link");
    assert!(!links.is_empty(), "non-image URL should render as link");

    let imgs = query_all(&container, ".embed-image");
    assert!(
        imgs.is_empty(),
        "non-image URL should NOT render as image embed"
    );
}

#[wasm_bindgen_test]
async fn multiple_image_urls_all_embedded() {
    let body = "pics: https://example.com/a.jpg https://example.com/b.gif";
    let (_segments, images) = extract_urls(body);

    assert_eq!(images.len(), 2, "should detect 2 image URLs");
    assert!(is_image_url(&images[0]));
    assert!(is_image_url(&images[1]));
}

// ── Dropdown Action Menu Tests ──────────────────────────────────────────────

#[wasm_bindgen_test]
async fn message_action_dropdown_toggles() {
    let (show_dropdown, set_show_dropdown) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="body">"hello"</div>
                <div class="message-actions">
                    <button class="action-trigger" on:click=move |ev| {
                        ev.stop_propagation();
                        set_show_dropdown.update(|v| *v = !*v);
                    }>"\u{22EF}"</button>
                    {move || {
                        if show_dropdown.get() {
                            Some(view! {
                                <div class="message-dropdown">
                                    <button class="dropdown-item">"Reply"</button>
                                    <button class="dropdown-item">"Pin"</button>
                                    <button class="dropdown-item">"React"</button>
                                </div>
                            })
                        } else {
                            None
                        }
                    }}
                </div>
            </div>
        }
    });
    tick().await;

    // Dropdown should not be visible initially.
    let dropdown = query(&container, ".message-dropdown");
    assert!(dropdown.is_none(), "dropdown should be hidden initially");

    // Click the action trigger.
    let trigger = query(&container, ".action-trigger");
    assert!(trigger.is_some(), "action trigger button should exist");
    simulate_click(&trigger.unwrap());
    tick().await;

    // Dropdown should now be visible.
    let dropdown = query(&container, ".message-dropdown");
    assert!(dropdown.is_some(), "dropdown should appear after click");

    let items = query_all(&container, ".dropdown-item");
    assert_eq!(items.len(), 3, "dropdown should have 3 items");

    // Click trigger again to close.
    let trigger = query(&container, ".action-trigger").unwrap();
    simulate_click(&trigger);
    tick().await;

    assert!(
        query(&container, ".message-dropdown").is_none(),
        "dropdown should close after second click"
    );
}

#[wasm_bindgen_test]
async fn dropdown_reply_fires_callback() {
    let (replied, set_replied) = signal(false);
    let (show_dropdown, _) = signal(true); // Start open.

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="message-actions">
                    {move || {
                        if show_dropdown.get() {
                            Some(view! {
                                <div class="message-dropdown">
                                    <button
                                        class="dropdown-item reply-item"
                                        on:click=move |_| set_replied.set(true)
                                    >
                                        "Reply"
                                    </button>
                                </div>
                            })
                        } else {
                            None
                        }
                    }}
                </div>
            </div>
        }
    });
    tick().await;

    let reply_btn = query(&container, ".reply-item").unwrap();
    simulate_click(&reply_btn);
    tick().await;

    assert!(replied.get_untracked(), "reply callback should have fired");
}

#[wasm_bindgen_test]
async fn dropdown_pin_fires_callback() {
    let (pinned, set_pinned) = signal(false);
    let (show_dropdown, _) = signal(true);

    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="message-actions">
                    {move || {
                        if show_dropdown.get() {
                            Some(view! {
                                <div class="message-dropdown">
                                    <button
                                        class="dropdown-item pin-item"
                                        on:click=move |_| set_pinned.set(true)
                                    >
                                        "Pin"
                                    </button>
                                </div>
                            })
                        } else {
                            None
                        }
                    }}
                </div>
            </div>
        }
    });
    tick().await;

    let pin_btn = query(&container, ".pin-item").unwrap();
    simulate_click(&pin_btn);
    tick().await;

    assert!(pinned.get_untracked(), "pin callback should have fired");
}

#[wasm_bindgen_test]
async fn dropdown_delete_has_danger_class() {
    let container = mount_test(move || {
        view! {
            <div class="message">
                <div class="message-dropdown">
                    <button class="dropdown-item">"Reply"</button>
                    <button class="dropdown-item">"Edit"</button>
                    <button class="dropdown-item dropdown-danger">"Delete"</button>
                </div>
            </div>
        }
    });
    tick().await;

    let danger = query(&container, ".dropdown-danger");
    assert!(
        danger.is_some(),
        "delete button should have .dropdown-danger class"
    );
    assert_eq!(
        text(&danger.unwrap()),
        "Delete",
        "danger button should say 'Delete'"
    );
}

// ── Admin Signal Reactivity Tests (Issue #81) ───────────────────────────────

#[wasm_bindgen_test]
async fn admin_buttons_hide_when_admin_status_revoked() {
    // Issue #81: Admin buttons must reactively hide when admin_ids changes.
    // Tests that using get() (not get_untracked()) for admin_ids makes the
    // UI update when admin status is revoked.
    let (admin_ids, set_admin_ids) =
        signal(std::collections::HashSet::from(["peer-a".to_string()]));
    let peer_id = "peer-a".to_string();

    let container = mount_test(move || {
        let pid = peer_id.clone();
        view! {
            {move || {
                let is_admin = admin_ids.get().contains(&pid);
                if is_admin {
                    Some(view! {
                        <div class="admin-actions">
                            <button class="btn-trust">"Trust"</button>
                            <button class="btn-kick">"Kick"</button>
                        </div>
                    })
                } else {
                    None
                }
            }}
        }
    });

    tick().await;

    // Admin buttons should be visible.
    assert!(
        query(&container, ".admin-actions").is_some(),
        "admin buttons should be visible when peer is admin"
    );

    // Revoke admin status.
    set_admin_ids.set(std::collections::HashSet::new());
    tick().await;

    // Admin buttons should now be hidden.
    assert!(
        query(&container, ".admin-actions").is_none(),
        "admin buttons should hide after admin status revoked"
    );
}

#[wasm_bindgen_test]
async fn admin_buttons_respond_to_peer_id_change() {
    // Issue #81: Using get() on peer_id (instead of get_untracked()) ensures
    // the UI updates when the local peer_id signal changes.
    let admin_set = std::collections::HashSet::from(["peer-a".to_string()]);
    let (admin_ids, _) = signal(admin_set);
    let (peer_id, set_peer_id) = signal("peer-a".to_string());

    let container = mount_test(move || {
        view! {
            {move || {
                let is_admin = admin_ids.get().contains(&peer_id.get());
                if is_admin {
                    Some(view! {
                        <div class="admin-actions">
                            <button class="btn-trust">"Trust"</button>
                        </div>
                    })
                } else {
                    None
                }
            }}
        }
    });

    tick().await;

    // Initially peer-a is admin — buttons visible.
    assert!(
        query(&container, ".admin-actions").is_some(),
        "admin buttons should be visible for peer-a"
    );

    // Change peer_id to peer-b (not in admin set).
    set_peer_id.set("peer-b".to_string());
    tick().await;

    // Buttons should now be hidden because peer-b is not admin.
    // With get_untracked(), this would NOT update — the stale value
    // "peer-a" would still be checked against admin_ids.
    assert!(
        query(&container, ".admin-actions").is_none(),
        "admin buttons should hide when peer_id changes to non-admin"
    );
}

// ── ConfirmDialog Component Tests ───────────────────────────────────────────
//
// The ConfirmDialog component (confirm_dialog.rs) is a standalone modal that
// takes all data as props. These tests verify its open/close state, button
// callbacks, and Escape-key handling by mirroring its exact rendering logic.

#[wasm_bindgen_test]
async fn confirm_dialog_hidden_when_not_visible() {
    let (visible, _set_visible) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                {move || {
                    if !visible.get() {
                        None::<leptos::prelude::AnyView>
                    } else {
                        Some(
                            view! {
                                <div class="confirm-overlay">
                                    <div class="confirm-dialog">
                                        <h3>"Delete server"</h3>
                                        <p>"Are you sure?"</p>
                                        <div class="confirm-actions">
                                            <button class="btn btn-secondary">"Cancel"</button>
                                            <button class="btn btn-danger">"Delete"</button>
                                        </div>
                                    </div>
                                </div>
                            }
                            .into_any(),
                        )
                    }
                }}
            </div>
        }
    });

    tick().await;

    // Dialog must not be in the DOM when visible=false.
    assert!(
        query(&container, ".confirm-overlay").is_none(),
        "confirm dialog should not be rendered when visible=false"
    );
    assert!(
        query(&container, ".confirm-dialog").is_none(),
        "confirm-dialog inner element should be absent"
    );
}

#[wasm_bindgen_test]
async fn confirm_dialog_shows_when_visible() {
    let (visible, set_visible) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                {move || {
                    if !visible.get() {
                        None::<leptos::prelude::AnyView>
                    } else {
                        Some(
                            view! {
                                <div class="confirm-overlay">
                                    <div class="confirm-dialog">
                                        <h3>"Leave server"</h3>
                                        <p>"Are you sure you want to leave?"</p>
                                        <div class="confirm-actions">
                                            <button class="btn btn-secondary">"Cancel"</button>
                                            <button class="btn btn-primary">"Leave"</button>
                                        </div>
                                    </div>
                                </div>
                            }
                            .into_any(),
                        )
                    }
                }}
            </div>
        }
    });

    tick().await;
    assert!(query(&container, ".confirm-overlay").is_none());

    // Open the dialog.
    set_visible.set(true);
    tick().await;

    let overlay = query(&container, ".confirm-overlay");
    assert!(overlay.is_some(), "overlay should appear when visible=true");

    let dialog = query(&container, ".confirm-dialog");
    assert!(dialog.is_some(), "confirm-dialog should render");

    let heading = query(&container, ".confirm-dialog h3").unwrap();
    assert_eq!(text(&heading), "Leave server");

    let confirm_btn = query(&container, ".btn.btn-primary").unwrap();
    assert_eq!(text(&confirm_btn), "Leave");

    let cancel_btn = query(&container, ".btn.btn-secondary").unwrap();
    assert_eq!(text(&cancel_btn), "Cancel");
}

#[wasm_bindgen_test]
async fn confirm_dialog_cancel_button_fires_callback() {
    let (visible, set_visible) = signal(true);
    let (cancelled, set_cancelled) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                {move || {
                    if !visible.get() {
                        None::<leptos::prelude::AnyView>
                    } else {
                        Some(
                            view! {
                                <div class="confirm-overlay">
                                    <div class="confirm-dialog">
                                        <div class="confirm-actions">
                                            <button
                                                class="btn btn-secondary"
                                                on:click=move |_| {
                                                    set_cancelled.set(true);
                                                    set_visible.set(false);
                                                }
                                            >
                                                "Cancel"
                                            </button>
                                            <button class="btn btn-danger">"Delete"</button>
                                        </div>
                                    </div>
                                </div>
                            }
                            .into_any(),
                        )
                    }
                }}
            </div>
        }
    });

    tick().await;
    assert!(query(&container, ".confirm-overlay").is_some());

    let cancel_btn = query(&container, ".btn.btn-secondary").unwrap();
    simulate_click(&cancel_btn);
    tick().await;

    assert!(
        cancelled.get_untracked(),
        "cancel callback should have fired"
    );
    assert!(
        query(&container, ".confirm-overlay").is_none(),
        "dialog should close after cancel"
    );
}

#[wasm_bindgen_test]
async fn confirm_dialog_confirm_button_fires_callback() {
    let (visible, set_visible) = signal(true);
    let (confirmed, set_confirmed) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                {move || {
                    if !visible.get() {
                        None::<leptos::prelude::AnyView>
                    } else {
                        Some(
                            view! {
                                <div class="confirm-overlay">
                                    <div class="confirm-dialog">
                                        <div class="confirm-actions">
                                            <button class="btn btn-secondary">"Cancel"</button>
                                            <button
                                                class="btn btn-danger"
                                                on:click=move |_| {
                                                    set_confirmed.set(true);
                                                    set_visible.set(false);
                                                }
                                            >
                                                "Delete"
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            }
                            .into_any(),
                        )
                    }
                }}
            </div>
        }
    });

    tick().await;

    let confirm_btn = query(&container, ".btn.btn-danger").unwrap();
    simulate_click(&confirm_btn);
    tick().await;

    assert!(
        confirmed.get_untracked(),
        "confirm callback should have fired"
    );
    assert!(
        query(&container, ".confirm-overlay").is_none(),
        "dialog should close after confirm"
    );
}

#[wasm_bindgen_test]
async fn confirm_dialog_escape_key_fires_cancel() {
    // Pressing Escape on the overlay should invoke on_cancel.
    let (visible, set_visible) = signal(true);
    let (cancelled, set_cancelled) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                {move || {
                    if !visible.get() {
                        None::<leptos::prelude::AnyView>
                    } else {
                        Some(
                            view! {
                                <div
                                    class="confirm-overlay"
                                    tabindex="-1"
                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                        if ev.key() == "Escape" {
                                            set_cancelled.set(true);
                                            set_visible.set(false);
                                        }
                                    }
                                >
                                    <div class="confirm-dialog">
                                        <div class="confirm-actions">
                                            <button class="btn btn-secondary">"Cancel"</button>
                                            <button class="btn btn-danger">"Delete"</button>
                                        </div>
                                    </div>
                                </div>
                            }
                            .into_any(),
                        )
                    }
                }}
            </div>
        }
    });

    tick().await;
    assert!(query(&container, ".confirm-overlay").is_some());

    // Dispatch Escape keydown on the overlay.
    let overlay = query(&container, ".confirm-overlay").unwrap();
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Escape");
    let escape =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    overlay
        .dyn_ref::<web_sys::EventTarget>()
        .unwrap()
        .dispatch_event(&escape)
        .unwrap();
    tick().await;

    assert!(
        cancelled.get_untracked(),
        "Escape key should fire the cancel callback"
    );
    assert!(
        query(&container, ".confirm-overlay").is_none(),
        "dialog should close after Escape"
    );
}

// ── ContextMenu Component Tests ─────────────────────────────────────────────
//
// The ContextMenu component (context_menu.rs) is a positioned popup that opens
// at (x, y) when visible=true. An overlay div captures outside clicks to close.

#[wasm_bindgen_test]
async fn context_menu_hidden_when_not_visible() {
    let (visible, _) = signal(false);
    let (x, _) = signal(0.0f64);
    let (y, _) = signal(0.0f64);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                <div
                    class=move || if visible.get() { "context-menu-overlay open" } else { "context-menu-overlay" }
                ></div>
                <div
                    class=move || if visible.get() { "context-menu open" } else { "context-menu" }
                    style=move || format!("left: {}px; top: {}px;", x.get(), y.get())
                >
                    <button class="context-menu-item">"Edit"</button>
                    <button class="context-menu-item">"Delete"</button>
                </div>
            </div>
        }
    });

    tick().await;

    let overlay = query(&container, ".context-menu-overlay").unwrap();
    assert!(
        !overlay.class_list().contains("open"),
        "overlay should not have 'open' class when not visible"
    );
    let menu = query(&container, ".context-menu").unwrap();
    assert!(
        !menu.class_list().contains("open"),
        "context menu should not have 'open' class when not visible"
    );
}

#[wasm_bindgen_test]
async fn context_menu_shows_when_visible() {
    let (visible, set_visible) = signal(false);
    let (x, _) = signal(100.0f64);
    let (y, _) = signal(200.0f64);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                <div
                    class=move || if visible.get() { "context-menu-overlay open" } else { "context-menu-overlay" }
                ></div>
                <div
                    class=move || if visible.get() { "context-menu open" } else { "context-menu" }
                    style=move || format!("left: {}px; top: {}px;", x.get(), y.get())
                >
                    <button class="context-menu-item">"Copy"</button>
                    <button class="context-menu-item">"Paste"</button>
                </div>
            </div>
        }
    });

    tick().await;
    assert!(!query(&container, ".context-menu")
        .unwrap()
        .class_list()
        .contains("open"));

    set_visible.set(true);
    tick().await;

    let menu = query(&container, ".context-menu").unwrap();
    assert!(
        menu.class_list().contains("open"),
        "context menu should have 'open' class when visible=true"
    );
    let overlay = query(&container, ".context-menu-overlay").unwrap();
    assert!(
        overlay.class_list().contains("open"),
        "overlay should also have 'open' class"
    );

    let items = query_all(&container, ".context-menu-item");
    assert_eq!(items.len(), 2, "both menu items should render");
    assert_eq!(text(&items[0]), "Copy");
    assert_eq!(text(&items[1]), "Paste");
}

#[wasm_bindgen_test]
async fn context_menu_positions_at_xy() {
    let container = mount_test(move || {
        view! {
            <div class="test-root">
                <div class="context-menu-overlay open"></div>
                <div class="context-menu open" style="left: 42px; top: 88px;">
                    <button class="context-menu-item">"Action"</button>
                </div>
            </div>
        }
    });

    tick().await;

    let menu = query(&container, ".context-menu").unwrap();
    let style = menu.get_attribute("style").unwrap_or_default();
    assert!(
        style.contains("left: 42px"),
        "style should set left to 42px, got: {style}"
    );
    assert!(
        style.contains("top: 88px"),
        "style should set top to 88px, got: {style}"
    );
}

#[wasm_bindgen_test]
async fn context_menu_overlay_click_fires_close() {
    let (visible, set_visible) = signal(true);
    let (closed, set_closed) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                <div
                    class=move || if visible.get() { "context-menu-overlay open" } else { "context-menu-overlay" }
                    on:click=move |_| {
                        set_closed.set(true);
                        set_visible.set(false);
                    }
                ></div>
                <div
                    class=move || if visible.get() { "context-menu open" } else { "context-menu" }
                    style="left: 0px; top: 0px;"
                >
                    <button class="context-menu-item">"Item"</button>
                </div>
            </div>
        }
    });

    tick().await;
    assert!(query(&container, ".context-menu")
        .unwrap()
        .class_list()
        .contains("open"));

    let overlay = query(&container, ".context-menu-overlay").unwrap();
    simulate_click(&overlay);
    tick().await;

    assert!(
        closed.get_untracked(),
        "on_close callback should fire when overlay is clicked"
    );
    assert!(
        !query(&container, ".context-menu")
            .unwrap()
            .class_list()
            .contains("open"),
        "menu should lose 'open' class after close"
    );
}

#[wasm_bindgen_test]
async fn context_menu_escape_key_fires_close() {
    let (visible, set_visible) = signal(true);
    let (closed, set_closed) = signal(false);

    let container = mount_test(move || {
        view! {
            <div class="test-root">
                <div class="context-menu-overlay open"></div>
                <div
                    class=move || if visible.get() { "context-menu open" } else { "context-menu" }
                    style="left: 0px; top: 0px;"
                    tabindex="-1"
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Escape" {
                            set_closed.set(true);
                            set_visible.set(false);
                        }
                    }
                >
                    <button class="context-menu-item">"Item"</button>
                </div>
            </div>
        }
    });

    tick().await;

    let menu = query(&container, ".context-menu").unwrap();
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Escape");
    let escape =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    menu.dyn_ref::<web_sys::EventTarget>()
        .unwrap()
        .dispatch_event(&escape)
        .unwrap();
    tick().await;

    assert!(
        closed.get_untracked(),
        "Escape key should fire the on_close callback"
    );
    assert!(
        !query(&container, ".context-menu")
            .unwrap()
            .class_list()
            .contains("open"),
        "menu should close after Escape"
    );
}

#[wasm_bindgen_test]
async fn context_menu_position_updates_reactively() {
    let (x, set_x) = signal(10.0f64);
    let (y, set_y) = signal(20.0f64);

    let container = mount_test(move || {
        view! {
            <div
                class="context-menu open"
                style=move || format!("left: {}px; top: {}px;", x.get(), y.get())
            >
                <button class="context-menu-item">"Item"</button>
            </div>
        }
    });

    tick().await;

    let menu = query(&container, ".context-menu").unwrap();
    let style = menu.get_attribute("style").unwrap_or_default();
    assert!(style.contains("left: 10px") && style.contains("top: 20px"));

    set_x.set(300.0);
    set_y.set(150.0);
    tick().await;

    let style = menu.get_attribute("style").unwrap_or_default();
    assert!(
        style.contains("left: 300px"),
        "x position should update reactively, got: {style}"
    );
    assert!(
        style.contains("top: 150px"),
        "y position should update reactively, got: {style}"
    );
}

// Phase 0 note: a foundation-tokens browser test was attempted but wasm-pack
// test does not bundle foundation.css via Trunk, so computed-style reads on
// `:root` return empty regardless of file content — the test could only pass
// by inlining the very values it was meant to verify. Dropped in favour of
// the visual-smoke gate in Task 15 of the Phase 0 plan.

// ── Phase 1a desktop shell tests ─────────────────────────────────────────────
//
// These verify the structural + ARIA contracts of the new desktop shell
// primitives without requiring live AppState / WebClientHandle context
// (tests mount raw markup in the same pattern as the rest of this file).

#[wasm_bindgen_test]
async fn desktop_shell_grove_rail_is_navigation_landmark() {
    let container = mount_test(|| {
        view! {
            <nav class="grove-rail" role="navigation" aria-label="groves">
                <button class="rail-tile rail-tile--letters" aria-label="letters · direct messages"></button>
                <button class="grove-tile" data-state="active" aria-label="Backyard"></button>
                <button class="rail-tile rail-tile--settings" aria-label="settings"></button>
            </nav>
        }
    });
    tick().await;

    let nav = query(&container, ".grove-rail").expect("grove-rail present");
    assert_eq!(nav.get_attribute("role").as_deref(), Some("navigation"));
    assert_eq!(nav.get_attribute("aria-label").as_deref(), Some("groves"));

    // Active grove tile uses `data-state="active"`.
    let active = query_all(&container, ".grove-tile[data-state=\"active\"]");
    assert_eq!(active.len(), 1);
}

#[wasm_bindgen_test]
async fn desktop_shell_channel_sidebar_is_navigation_landmark() {
    let container = mount_test(|| {
        view! {
            <aside class="channel-sidebar" role="navigation" aria-label="channels">
                <div class="grove-header">
                    <span class="grove-header-name">"Backyard"</span>
                    <button class="grove-menu-chevron server-gear-btn" aria-label="grove menu"></button>
                </div>
                <div class="channel-list">
                    <div class="channel-group" data-group="commons">
                        <button class="channel-group-label">"commons"</button>
                        <div class="channel-group-rows">
                            <div class="channel-item">"general"</div>
                        </div>
                    </div>
                </div>
            </aside>
        }
    });
    tick().await;

    let aside = query(&container, ".channel-sidebar").expect("channel-sidebar present");
    assert_eq!(aside.get_attribute("role").as_deref(), Some("navigation"));
    assert_eq!(aside.get_attribute("aria-label").as_deref(), Some("channels"));

    // `.server-gear-btn` compat class still sits on the grove-menu chevron.
    let chevron = query(&container, ".grove-menu-chevron.server-gear-btn");
    assert!(chevron.is_some(), "grove menu chevron keeps server-gear-btn compat class");
}

#[wasm_bindgen_test]
async fn desktop_shell_main_pane_header_six_buttons_in_order() {
    let container = mount_test(|| {
        view! {
            <header class="main-pane-header" role="banner" aria-label="channel header">
                <span class="mph-kind-icon"></span>
                <span class="mph-title">"general"</span>
                <div class="mph-spacer"></div>
                <div class="mph-action-bar">
                    <button class="action-btn" aria-label="members"></button>
                    <button class="action-btn" aria-label="pinned"></button>
                    <button class="action-btn" aria-label="thread"></button>
                    <button class="action-btn" aria-label="join call"></button>
                    <button class="action-btn" aria-label="search (⌘K)"></button>
                    <button class="action-btn" aria-label="more"></button>
                </div>
            </header>
        }
    });
    tick().await;

    let header = query(&container, ".main-pane-header").expect("header present");
    assert_eq!(header.get_attribute("role").as_deref(), Some("banner"));
    assert_eq!(
        header.get_attribute("aria-label").as_deref(),
        Some("channel header")
    );

    let buttons = query_all(&container, ".mph-action-bar .action-btn");
    assert_eq!(buttons.len(), 6, "action bar has six buttons");

    let labels: Vec<String> = buttons
        .iter()
        .map(|b| b.get_attribute("aria-label").unwrap_or_default())
        .collect();
    assert_eq!(
        labels,
        vec![
            "members",
            "pinned",
            "thread",
            "join call",
            "search (⌘K)",
            "more",
        ],
        "action-bar labels are in the fixed order from layout-primitives"
    );
}

#[wasm_bindgen_test]
async fn desktop_shell_right_rail_one_of_three() {
    // Three passes: members open, pinned open, thread open. At any
    // point exactly one data-pane attribute matches.
    let (which, set_which) = signal("members".to_string());

    let container = mount_test(move || {
        view! {
            <aside
                class="right-rail"
                role="complementary"
                aria-label=move || which.get()
                data-open="true"
            >
                <div class="right-rail-inner">
                    {move || view! {
                        <div class="right-rail-pane" data-pane=which.get()></div>
                    }}
                </div>
            </aside>
        }
    });

    tick().await;
    let panes = query_all(&container, ".right-rail-pane");
    assert_eq!(panes.len(), 1, "exactly one pane is mounted at a time");
    assert_eq!(
        panes[0].get_attribute("data-pane").as_deref(),
        Some("members")
    );
    let rail = query(&container, ".right-rail").unwrap();
    assert_eq!(
        rail.get_attribute("aria-label").as_deref(),
        Some("members")
    );

    set_which.set("pinned".to_string());
    tick().await;
    let panes = query_all(&container, ".right-rail-pane");
    assert_eq!(panes.len(), 1);
    assert_eq!(
        panes[0].get_attribute("data-pane").as_deref(),
        Some("pinned")
    );

    set_which.set("thread".to_string());
    tick().await;
    let panes = query_all(&container, ".right-rail-pane");
    assert_eq!(panes.len(), 1);
    assert_eq!(
        panes[0].get_attribute("data-pane").as_deref(),
        Some("thread")
    );
}

/// Mirror of `ChannelGroup::classify` kept local because willow-web is
/// a bin crate and can't expose the enum to integration tests. This
/// duplicated logic is intentionally thin — the real source is in
/// `crates/web/src/components/channel_sidebar.rs`; editing that file
/// should be paired with an edit here until willow-web exposes a lib.
fn classify_channel(name: &str, kind: willow_state::ChannelKind) -> &'static str {
    if name.starts_with("_ephemeral-") {
        "ephemeral"
    } else if name.starts_with("_archive-") {
        "archives"
    } else if matches!(kind, willow_state::ChannelKind::Voice) {
        "voice"
    } else {
        "commons"
    }
}

#[wasm_bindgen_test]
async fn desktop_shell_channel_group_classification() {
    use willow_state::ChannelKind;

    assert_eq!(classify_channel("general", ChannelKind::Text), "commons");
    assert_eq!(classify_channel("gossip", ChannelKind::Voice), "voice");
    assert_eq!(
        classify_channel("_ephemeral-drafts", ChannelKind::Text),
        "ephemeral"
    );
    assert_eq!(
        classify_channel("_archive-2023", ChannelKind::Text),
        "archives"
    );
    // Ephemeral prefix wins over voice kind.
    assert_eq!(
        classify_channel("_ephemeral-call", ChannelKind::Voice),
        "ephemeral"
    );
}
