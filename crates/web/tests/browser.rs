//! In-browser tests for the Willow Leptos web UI.
//!
//! Run with: `wasm-pack test crates/web --headless --chrome`
//!
//! These tests render Leptos components in a real browser DOM and verify
//! that signals, events, and effects work correctly.

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
    let msg = willow_client::ChatMessage::new(
        "server-1".into(),
        "topic".into(),
        "Alice".into(),
        "Hello world!".into(),
        false,
        1000,
    );

    let container = mount_test(move || {
        // Inline a simplified message view for testing
        let author = msg.author.clone();
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
    let (messages, _set_messages) = signal(Vec::<willow_client::ChatMessage>::new());

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
    assert_eq!(text(&query(&container, ".empty-state").unwrap()), "No messages");
}

#[wasm_bindgen_test]
async fn message_list_renders_messages() {
    let (messages, set_messages) = signal(Vec::<willow_client::ChatMessage>::new());

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
    set_messages.set(vec![
        willow_client::ChatMessage::new("s".into(), "t".into(), "A".into(), "first".into(), false, 1),
        willow_client::ChatMessage::new("s".into(), "t".into(), "B".into(), "second".into(), false, 2),
    ]);
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

    let input: web_sys::HtmlInputElement = query(&container, ".test-input")
        .unwrap()
        .unchecked_into();

    // Simulate typing by setting value and dispatching input event.
    input.set_value("hello");
    let event = web_sys::InputEvent::new("input").unwrap();
    input
        .dyn_ref::<web_sys::EventTarget>()
        .unwrap()
        .dispatch_event(&event)
        .unwrap();

    tick().await;

    assert_eq!(value.get_untracked(), "hello");
}

#[wasm_bindgen_test]
async fn input_sends_on_enter() {
    let (sent, set_sent) = signal(Option::<String>::None);
    let (input_text, set_input_text) = signal(String::new());

    let container = mount_test(move || {
        let on_send = set_sent.clone();
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
    input.set_value("hello");
    let input_event = web_sys::InputEvent::new("input").unwrap();
    input
        .dyn_ref::<web_sys::EventTarget>()
        .unwrap()
        .dispatch_event(&input_event)
        .unwrap();
    tick().await;

    // Press Enter.
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Enter");
    let enter = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
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
