//! Command palette — spec layout-primitives.md §Command palette.
//!
//! Anatomy:
//!   - backdrop (blur) → dialog root
//!     - input (scope-aware placeholder + a11y combobox attrs)
//!     - result groups (channels / letters / groves / people / actions)
//!     - empty / loading / error / recents state
//!     - footer hint strip (↑↓ move · ⏎ open · esc close)
//!
//! Scope prefixes:
//!   - `#foo`     → channels
//!   - `@foo`     → peers
//!   - `> foo`    → actions
//!   - `"foo"`    → literal match (wraps the query text)
//!
//! Signals (`show_palette`, `query`, `selected_index`) are preserved.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::palette_actions::{self, CommandContext};
use crate::icons;
use crate::palette_recents::{self, Recent};
use crate::state::AppState;

/// Scope inferred from the leading character of the query.
#[derive(Clone, Copy, PartialEq, Debug)]
enum PaletteScope {
    Mixed,
    Channels,
    Peers,
    Actions,
}

/// Which result group a row belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum ResultGroup {
    Channels,
    Letters,
    Groves,
    People,
    Actions,
    Search,
}

/// Activation payload stored with each row.
#[derive(Clone, PartialEq)]
enum PaletteActivate {
    OpenChannel(String),
    SwitchGrove(String),
    OpenProfile(String),
    RunAction(&'static str),
    Search(PaletteScope, String),
}

/// A single displayable row.
#[derive(Clone, PartialEq)]
struct PaletteRow {
    group: ResultGroup,
    id: String,
    label: String,
    meta: Option<String>,
    activate: PaletteActivate,
}

/// Parse the raw input into `(scope, body, literal)`.
fn parse_input(raw: &str) -> (PaletteScope, String, bool) {
    let trimmed = raw.trim_start();
    let (scope, rest) = match trimmed.chars().next() {
        Some('#') => (PaletteScope::Channels, &trimmed[1..]),
        Some('@') => (PaletteScope::Peers, &trimmed[1..]),
        Some('>') => (PaletteScope::Actions, &trimmed[1..]),
        _ => (PaletteScope::Mixed, trimmed),
    };
    let rest = rest.trim_start();
    if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        (scope, rest[1..rest.len() - 1].to_string(), true)
    } else {
        (scope, rest.to_string(), false)
    }
}

fn matches_scope(scope: PaletteScope, group: ResultGroup) -> bool {
    match (scope, group) {
        (PaletteScope::Mixed, _) => true,
        (PaletteScope::Channels, ResultGroup::Channels) => true,
        (PaletteScope::Peers, ResultGroup::People) => true,
        (PaletteScope::Actions, ResultGroup::Actions) => true,
        _ => false,
    }
}

fn scope_label(scope: PaletteScope) -> &'static str {
    match scope {
        PaletteScope::Mixed => "everything",
        PaletteScope::Channels => "channels",
        PaletteScope::Peers => "peers",
        PaletteScope::Actions => "actions",
    }
}

fn group_label(group: ResultGroup) -> &'static str {
    match group {
        ResultGroup::Channels => "channels",
        ResultGroup::Letters => "letters",
        ResultGroup::Groves => "groves",
        ResultGroup::People => "people",
        ResultGroup::Actions => "actions",
        ResultGroup::Search => "search",
    }
}

fn icon_for_group(group: ResultGroup) -> leptos::tachys::view::any_view::AnyView {
    match group {
        ResultGroup::Channels => icons::icon_hash().into_any(),
        ResultGroup::Letters => icons::icon_edit().into_any(),
        ResultGroup::Groves => icons::icon_compass().into_any(),
        ResultGroup::People => icons::icon_users().into_any(),
        ResultGroup::Actions => icons::icon_settings().into_any(),
        ResultGroup::Search => icons::icon_search().into_any(),
    }
}

fn recent_group(kind: &str) -> ResultGroup {
    match kind {
        "channel" => ResultGroup::Channels,
        "grove" => ResultGroup::Groves,
        "peer" => ResultGroup::People,
        "letter" => ResultGroup::Letters,
        _ => ResultGroup::Actions,
    }
}

/// Group rows preserving first-seen order of each group.
fn group_rows(rows: &[PaletteRow]) -> Vec<(ResultGroup, Vec<PaletteRow>)> {
    let mut order: Vec<ResultGroup> = Vec::new();
    let mut buckets: std::collections::HashMap<ResultGroup, Vec<PaletteRow>> =
        std::collections::HashMap::new();
    for r in rows {
        if !order.contains(&r.group) {
            order.push(r.group);
        }
        buckets.entry(r.group).or_default().push(r.clone());
    }
    order
        .into_iter()
        .map(|g| {
            let bucket = buckets.remove(&g).unwrap_or_default();
            (g, bucket)
        })
        .collect()
}

/// Result row for the recents view.
fn recents_to_rows(recents: &[Recent]) -> Vec<PaletteRow> {
    recents
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let activate = match r.kind.as_str() {
                "channel" => PaletteActivate::OpenChannel(r.id.clone()),
                "grove" => PaletteActivate::SwitchGrove(r.id.clone()),
                "peer" => PaletteActivate::OpenProfile(r.id.clone()),
                "letter" => PaletteActivate::OpenChannel(r.id.clone()),
                _ => PaletteActivate::RunAction("tweaks"),
            };
            PaletteRow {
                group: recent_group(&r.kind),
                id: format!("pr-recent-{i}"),
                label: r.label.clone(),
                meta: Some("recent".to_string()),
                activate,
            }
        })
        .collect()
}

/// Command palette overlay.
///
/// Opened via ⌘K / Ctrl-K or the search button.
/// Arrow keys navigate, Enter activates, Escape closes.
#[allow(clippy::too_many_arguments)]
#[component]
pub fn CommandPalette(
    on_close: Callback<()>,
    on_switch_channel: Callback<String>,
    on_switch_server: Callback<String>,
    on_open_members: Callback<()>,
    /// Open the settings panel.
    #[prop(optional, into)]
    on_open_settings: Option<Callback<()>>,
    /// Open the tweaks panel (same surface as settings in v1).
    #[prop(optional, into)]
    on_open_tweaks: Option<Callback<()>>,
    /// Start "add server / new grove" flow.
    #[prop(optional, into)]
    on_create_grove: Option<Callback<()>>,
    /// New channel.
    #[prop(optional, into)]
    on_new_channel: Option<Callback<()>>,
    /// New letter / DM.
    #[prop(optional, into)]
    on_new_letter: Option<Callback<()>>,
    /// Open the sync queue panel.
    #[prop(optional, into)]
    on_open_sync_queue: Option<Callback<()>>,
    /// Move the active call to another device.
    #[prop(optional, into)]
    on_move_call: Option<Callback<()>>,
    /// Toggle theme (deferred; stub in v1).
    #[prop(optional, into)]
    on_toggle_theme: Option<Callback<()>>,
    /// Sign out.
    #[prop(optional, into)]
    on_sign_out: Option<Callback<()>>,
    /// Search handoff callback. v1 logs only.
    #[prop(optional, into)]
    on_search: Option<Callback<String>>,
) -> impl IntoView {
    let app_state = use_context::<AppState>().unwrap();

    let (query, set_query) = signal(String::new());
    let (selected_index, set_selected_index) = signal(0usize);
    let (recents, set_recents) = signal(palette_recents::load());

    // v1: local search is not wired yet; keep signals for the state renderer.
    let (search_running, _set_search_running) = signal(false);
    let (search_error, _set_search_error) = signal(false);

    // Build the command context from props (each callback wraps an Option).
    let ctx = CommandContext {
        on_open_tweaks: on_open_tweaks.unwrap_or_else(|| Callback::new(|_| {})),
        on_open_settings: on_open_settings.unwrap_or_else(|| Callback::new(|_| {})),
        on_new_channel: on_new_channel.unwrap_or_else(|| Callback::new(|_| {})),
        on_new_letter: on_new_letter.unwrap_or_else(|| Callback::new(|_| {})),
        on_create_grove: on_create_grove.unwrap_or_else(|| Callback::new(|_| {})),
        on_open_sync_queue: on_open_sync_queue.unwrap_or_else(|| Callback::new(|_| {})),
        on_move_call: on_move_call.unwrap_or_else(|| Callback::new(|_| {})),
        on_toggle_theme: on_toggle_theme.unwrap_or_else(|| Callback::new(|_| {})),
        on_sign_out: on_sign_out.unwrap_or_else(|| Callback::new(|_| {})),
    };
    let ctx_for_run = ctx.clone();

    // Result memo: rebuilds whenever query + state change.
    let rows = Memo::new(move |_| {
        let (scope, q, literal) = parse_input(&query.get());
        let q_lc = q.to_lowercase();
        let matches = |label: &str| -> bool {
            if q.is_empty() {
                true
            } else if literal {
                label == q
            } else {
                label.to_lowercase().contains(&q_lc)
            }
        };

        let mut out: Vec<PaletteRow> = Vec::new();

        // Channels.
        if matches_scope(scope, ResultGroup::Channels) {
            for (i, name) in app_state.chat.channels.get().into_iter().enumerate() {
                if matches(&name) {
                    out.push(PaletteRow {
                        group: ResultGroup::Channels,
                        id: format!("pr-ch-{i}"),
                        label: name.clone(),
                        meta: None,
                        activate: PaletteActivate::OpenChannel(name),
                    });
                }
            }
        }

        // Groves.
        if matches_scope(scope, ResultGroup::Groves) {
            for (i, (gid, name)) in app_state.server.servers.get().into_iter().enumerate() {
                if matches(&name) {
                    out.push(PaletteRow {
                        group: ResultGroup::Groves,
                        id: format!("pr-gv-{i}"),
                        label: name,
                        meta: None,
                        activate: PaletteActivate::SwitchGrove(gid),
                    });
                }
            }
        }

        // People.
        if matches_scope(scope, ResultGroup::People) {
            for (i, (pid, name, _online)) in
                app_state.network.peers.get().into_iter().enumerate()
            {
                if matches(&name) || matches(&pid) {
                    out.push(PaletteRow {
                        group: ResultGroup::People,
                        id: format!("pr-pp-{i}"),
                        label: name,
                        meta: Some(pid.chars().take(8).collect::<String>()),
                        activate: PaletteActivate::OpenProfile(pid),
                    });
                }
            }
        }

        // Actions.
        if matches_scope(scope, ResultGroup::Actions) {
            for (i, action) in palette_actions::catalog().into_iter().enumerate() {
                if (action.available)(&app_state) && matches(action.label) {
                    out.push(PaletteRow {
                        group: ResultGroup::Actions,
                        id: format!("pr-act-{i}"),
                        label: action.label.to_string(),
                        meta: None,
                        activate: PaletteActivate::RunAction(action.id),
                    });
                }
            }
        }

        // Letters — feature-flagged until letters-dms.md lands.

        // Search handoff row — always last when there's a query.
        if !q.is_empty() {
            out.push(PaletteRow {
                group: ResultGroup::Search,
                id: "pr-search".into(),
                label: format!("search \"{q}\" in {}", scope_label(scope)),
                meta: None,
                activate: PaletteActivate::Search(scope, q),
            });
        }

        out
    });

    let active_row_id = Memo::new(move |_| {
        let idx = selected_index.get();
        rows.get().get(idx).map(|r| r.id.clone()).unwrap_or_default()
    });

    // Activate dispatch closure. Wrapped in `SendWrapper<Rc<...>>` so
    // it can be cloned into the three consumer closures below
    // (keydown, render_rows, render_recents) without moving — the
    // wrapper makes the !Send Rc Send-qualified for Leptos' Render
    // bound on single-threaded WASM.
    let dispatch_activate: send_wrapper::SendWrapper<std::rc::Rc<dyn Fn(&PaletteRow)>> =
        send_wrapper::SendWrapper::new(std::rc::Rc::new({
        let ctx = ctx_for_run.clone();
        move |row: &PaletteRow| {
            // Persist to recents.
            let recent = match &row.activate {
                PaletteActivate::OpenChannel(id) => Some(Recent {
                    kind: "channel".into(),
                    id: id.clone(),
                    label: row.label.clone(),
                }),
                PaletteActivate::SwitchGrove(id) => Some(Recent {
                    kind: "grove".into(),
                    id: id.clone(),
                    label: row.label.clone(),
                }),
                PaletteActivate::OpenProfile(id) => Some(Recent {
                    kind: "peer".into(),
                    id: id.clone(),
                    label: row.label.clone(),
                }),
                PaletteActivate::RunAction(id) => Some(Recent {
                    kind: "action".into(),
                    id: (*id).to_string(),
                    label: row.label.clone(),
                }),
                PaletteActivate::Search(_, _) => None,
            };
            if let Some(r) = recent {
                palette_recents::push(r);
            }

            // Dispatch.
            match &row.activate {
                PaletteActivate::OpenChannel(id) => on_switch_channel.run(id.clone()),
                PaletteActivate::SwitchGrove(id) => on_switch_server.run(id.clone()),
                PaletteActivate::OpenProfile(_) => on_open_members.run(()),
                PaletteActivate::RunAction(id) => palette_actions::run(id, &ctx),
                PaletteActivate::Search(_scope, q) => {
                    if let Some(cb) = on_search {
                        cb.run(q.clone());
                    } else {
                        tracing::info!(%q, "palette search handoff (v1 stub)");
                    }
                }
            }
            on_close.run(());
        }
    }));

    let dispatch_for_keydown = dispatch_activate.clone();
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let all_rows = rows.get_untracked();
        let len = all_rows.len();
        match ev.key().as_str() {
            "Escape" => {
                ev.prevent_default();
                if !query.get_untracked().is_empty() {
                    set_query.set(String::new());
                    set_selected_index.set(0);
                } else {
                    on_close.run(());
                }
            }
            "ArrowDown" => {
                ev.prevent_default();
                if len > 0 {
                    set_selected_index.update(|i| *i = (*i + 1) % len);
                }
            }
            "ArrowUp" => {
                ev.prevent_default();
                if len > 0 {
                    set_selected_index.update(|i| {
                        *i = if *i == 0 { len - 1 } else { *i - 1 };
                    });
                }
            }
            "Enter" => {
                ev.prevent_default();
                let idx = selected_index.get_untracked();
                if let Some(row) = all_rows.get(idx) {
                    dispatch_for_keydown(row);
                }
            }
            _ => {}
        }
    };

    // Will-change scrub — let the browser optimize layer creation for
    // the palette's pop-in, then release the hint.
    Effect::new(move |_| {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(node) = doc.query_selector(".palette-root").ok().flatten() {
                if let Ok(html) = node.dyn_into::<web_sys::HtmlElement>() {
                    let _ = html
                        .style()
                        .set_property("will-change", "transform, opacity");
                    let h = html.clone();
                    set_timeout(
                        move || {
                            let _ = h.style().remove_property("will-change");
                        },
                        std::time::Duration::from_millis(180),
                    );
                }
            }
        }
    });

    // Load recents whenever the palette opens.
    Effect::new(move |_| {
        set_recents.set(palette_recents::load());
    });

    // Helper: render the group sections.
    let dispatch_for_rows = dispatch_activate.clone();
    let render_rows = move || {
        let all_rows = rows.get();
        let sel = selected_index.get();
        let groups = group_rows(&all_rows);
        let sections: Vec<_> = groups
            .into_iter()
            .map(|(group, group_rows_list)| {
                let label = group_label(group);
                let rows_views: Vec<_> = group_rows_list
                    .into_iter()
                    .map(|row| {
                        let flat_idx = all_rows
                            .iter()
                            .position(|r| r.id == row.id)
                            .unwrap_or(0);
                        let selected = flat_idx == sel;
                        let row_activate = row.clone();
                        let row_for_mouse = flat_idx;
                        let icon_view = icon_for_group(row.group);
                        let meta = row.meta.clone();
                        let dispatch = dispatch_for_rows.clone();
                        view! {
                            <div
                                role="option"
                                id=row.id.clone()
                                aria-selected=move || selected.to_string()
                                class="palette-row"
                                on:click=move |_| dispatch(&row_activate)
                                on:mouseenter=move |_| set_selected_index.set(row_for_mouse)
                            >
                                <span class="icon">{icon_view}</span>
                                <span>{row.label.clone()}</span>
                                {meta.map(|m| view! {
                                    <span class="palette-row-meta">{m}</span>
                                })}
                            </div>
                        }
                    })
                    .collect();
                view! {
                    <>
                        <div class="palette-group-label" aria-hidden="true">{label}</div>
                        {rows_views}
                    </>
                }
            })
            .collect();
        view! { <div>{sections}</div> }
    };

    let dispatch_for_recents = dispatch_activate.clone();
    let render_recents = move || {
        let r = recents.get();
        let recent_rows = recents_to_rows(&r);
        let sel = selected_index.get();
        let rows_views: Vec<_> = recent_rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| {
                let selected = i == sel;
                let row_activate = row.clone();
                let icon_view = icon_for_group(row.group);
                let meta = row.meta.clone();
                let dispatch = dispatch_for_recents.clone();
                view! {
                    <div
                        role="option"
                        id=row.id.clone()
                        aria-selected=move || selected.to_string()
                        class="palette-row"
                        on:click=move |_| dispatch(&row_activate)
                        on:mouseenter=move |_| set_selected_index.set(i)
                    >
                        <span class="icon">{icon_view}</span>
                        <span>{row.label.clone()}</span>
                        {meta.map(|m| view! {
                            <span class="palette-row-meta">{m}</span>
                        })}
                    </div>
                }
            })
            .collect();
        view! {
            <div>
                <div class="palette-group-label" aria-hidden="true">"recents"</div>
                {rows_views}
            </div>
        }
    };

    let body = move || {
        let q = query.get();
        let rows_len = rows.get().len();
        if q.is_empty() && recents.get().is_empty() {
            view! {
                <div class="palette-empty">
                    "jump or search across willow — try #channel, @peer, > for actions"
                </div>
            }
            .into_any()
        } else if q.is_empty() {
            render_recents().into_any()
        } else if rows_len == 0 && !search_running.get() {
            view! {
                <div class="palette-empty">
                    {format!("nothing matches '{q}' — try > for actions or /search")}
                </div>
            }
            .into_any()
        } else if search_running.get() {
            view! {
                <div class="palette-loading" role="status" aria-live="polite">
                    "searching… (local only)"
                </div>
            }
            .into_any()
        } else if search_error.get() {
            view! {
                <div class="palette-error" role="status" aria-live="polite">
                    "search indexer is rebuilding — try again in a moment"
                </div>
            }
            .into_any()
        } else {
            render_rows().into_any()
        }
    };

    view! {
        <div
            class="palette-backdrop"
            role="presentation"
            on:click=move |_| on_close.run(())
        >
            <div
                class="palette-root"
                role="dialog"
                aria-modal="true"
                aria-label="command palette"
                on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()
            >
                <input
                    class="palette-input"
                    type="text"
                    placeholder="jump or search…"
                    aria-label="command palette input"
                    aria-autocomplete="list"
                    aria-controls="palette-listbox"
                    aria-activedescendant=move || active_row_id.get()
                    prop:value=move || query.get()
                    on:input=move |ev| {
                        set_query.set(event_target_value(&ev));
                        set_selected_index.set(0);
                    }
                    on:keydown=on_keydown
                    autofocus=true
                />
                <div
                    id="palette-listbox"
                    class="palette-results"
                    role="listbox"
                    aria-label="results"
                >
                    {body}
                </div>
                <div class="palette-footer" aria-hidden="true">
                    <span>"↑↓ move"</span>
                    <span>"⏎ open"</span>
                    <span>"esc close"</span>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod parse_input_tests {
    use super::*;

    #[test]
    fn mixed_default() {
        assert_eq!(
            parse_input("foo"),
            (PaletteScope::Mixed, "foo".into(), false)
        );
    }

    #[test]
    fn channel_prefix() {
        assert_eq!(
            parse_input("#general"),
            (PaletteScope::Channels, "general".into(), false)
        );
    }

    #[test]
    fn peer_prefix() {
        assert_eq!(
            parse_input("@alice"),
            (PaletteScope::Peers, "alice".into(), false)
        );
    }

    #[test]
    fn action_prefix() {
        assert_eq!(
            parse_input("> new channel"),
            (PaletteScope::Actions, "new channel".into(), false)
        );
    }

    #[test]
    fn literal_quotes() {
        assert_eq!(
            parse_input("\"reading list\""),
            (PaletteScope::Mixed, "reading list".into(), true)
        );
    }

    #[test]
    fn scope_matches() {
        assert!(matches_scope(PaletteScope::Mixed, ResultGroup::Channels));
        assert!(matches_scope(
            PaletteScope::Channels,
            ResultGroup::Channels
        ));
        assert!(!matches_scope(PaletteScope::Channels, ResultGroup::People));
        assert!(matches_scope(PaletteScope::Peers, ResultGroup::People));
        assert!(matches_scope(PaletteScope::Actions, ResultGroup::Actions));
    }
}
