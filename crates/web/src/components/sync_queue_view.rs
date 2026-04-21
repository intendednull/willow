//! Sync-queue view — Phase 2b.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Sync queue
//! screen. Shared full-surface component mounted in the desktop
//! right-pane (gated on `app.queue.open == true`) and the mobile
//! `/sync-queue` route.
//!
//! v1 scope (this commit): header + close + status card + outbound /
//! inbound tabs + per-peer row list + recent-arrivals section +
//! footer `retry now` / `mark as read locally` + verbatim privacy
//! footnote. Per-message expansion, virtualisation, relay-only glyph,
//! and permanent-unreachable card are tracked in Task 18.

use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::{sync_queue_copy, RelaySignalButton};
use crate::icons;
use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Outbound,
    Inbound,
}

/// Full sync-queue surface. Header + status card + tabs + rows +
/// recent arrivals + footer + footnote.
#[component]
pub fn SyncQueueView() -> impl IntoView {
    let app =
        use_context::<AppState>().expect("<SyncQueueView> mounted outside an AppState context");
    let handle = use_context::<WebClientHandle>()
        .expect("<SyncQueueView> requires a WebClientHandle context");
    let queue_view = app.queue.view;
    let queue_open = app.queue.open;

    let tab = RwSignal::new(Tab::Outbound);
    let busy = RwSignal::new(false);

    let status_label = move || {
        let v = queue_view.get();
        if v.depth == 0 {
            sync_queue_copy::SCREEN_CARD_DRAINED.to_string()
        } else {
            sync_queue_copy::SCREEN_CARD_REACHING_OUT.to_string()
        }
    };

    let peer_counts = move || {
        let v = queue_view.get();
        let total = v.per_peer.len() as u32;
        // Best-effort: peers with `last_attempt_at == Some(_)` count as
        // reached; others pending.
        let reached = v
            .per_peer
            .values()
            .filter(|s| s.last_attempt_at.is_some())
            .count() as u32;
        (reached, total)
    };

    let retry_handle = handle.clone();
    let retry_click = move |_| {
        if busy.get() {
            return;
        }
        busy.set(true);
        let h = retry_handle.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = h.retry_queue().await;
            busy.set(false);
        });
    };

    let mark_read_handle = handle.clone();
    let mark_read_click = move |_| {
        let h = mark_read_handle.clone();
        let view = queue_view.get();
        let peers: Vec<_> = view.inbound_per_peer.keys().copied().collect();
        wasm_bindgen_futures::spawn_local(async move {
            for peer in peers {
                let _ = h.mark_queue_read(peer).await;
            }
        });
    };

    view! {
        <section class="sync-queue-view" role="region" aria-label="sync queue">
            // ── Header ──────────────────────────────────────────────
            <header class="sync-queue-view__header">
                <button
                    class="sync-queue-view__close"
                    type="button"
                    aria-label="close sync queue"
                    on:click=move |_| queue_open.set(false)
                >
                    "×"
                </button>
                <div class="sync-queue-view__titles">
                    <h2 class="sync-queue-view__title">{sync_queue_copy::SCREEN_TITLE}</h2>
                    <p class="sync-queue-view__subtitle">{sync_queue_copy::SCREEN_SUBTITLE}</p>
                </div>
                <RelaySignalButton />
            </header>

            // ── Status card ─────────────────────────────────────────
            <div class="sync-queue-view__status">
                <span class=move || {
                    if queue_view.get().depth == 0 {
                        "sync-queue-view__dot sync-queue-view__dot--drained"
                    } else {
                        "sync-queue-view__dot sync-queue-view__dot--pulsing"
                    }
                }>
                    {move || if queue_view.get().depth == 0 {
                        Some(icons::icon_check_small())
                    } else {
                        None
                    }}
                </span>
                <span class="sync-queue-view__status-label">{status_label}</span>
                <span class="sync-queue-view__status-count">
                    {move || {
                        let (r, t) = peer_counts();
                        sync_queue_copy::screen_card_count(r, t)
                    }}
                </span>
            </div>

            // ── Tabs ────────────────────────────────────────────────
            <div class="sync-queue-view__tabs" role="tablist">
                <button
                    role="tab"
                    class=move || {
                        if tab.get() == Tab::Outbound {
                            "sync-queue-view__tab sync-queue-view__tab--active"
                        } else {
                            "sync-queue-view__tab"
                        }
                    }
                    aria-selected=move || (tab.get() == Tab::Outbound).to_string()
                    on:click=move |_| tab.set(Tab::Outbound)
                >
                    "outbound"
                </button>
                <button
                    role="tab"
                    class=move || {
                        if tab.get() == Tab::Inbound {
                            "sync-queue-view__tab sync-queue-view__tab--active"
                        } else {
                            "sync-queue-view__tab"
                        }
                    }
                    aria-selected=move || (tab.get() == Tab::Inbound).to_string()
                    on:click=move |_| tab.set(Tab::Inbound)
                >
                    "inbound"
                </button>
            </div>

            // ── Row list ────────────────────────────────────────────
            <ul class="sync-queue-view__rows" role="list">
                {move || {
                    let view = queue_view.get();
                    let rows: Vec<_> = if tab.get() == Tab::Outbound {
                        view.per_peer.iter()
                            .map(|(pid, sum)| {
                                let pid_str = pid.to_string();
                                let short = if pid_str.len() > 8 { format!("{}...", &pid_str[..6]) } else { pid_str };
                                let count = sum.outbound;
                                view! {
                                    <li class="sync-queue-row" role="listitem" tabindex="0">
                                        <span class="sync-queue-row__name">{short}</span>
                                        <span class="sync-queue-row__count queue-pill">
                                            {icons::icon_hourglass_sm()}
                                            <span aria-hidden="true">{sync_queue_copy::pill_queued(count)}</span>
                                        </span>
                                    </li>
                                }
                            })
                            .collect()
                    } else {
                        view.inbound_per_peer.iter()
                            .map(|(pid, count)| {
                                let pid_str = pid.to_string();
                                let short = if pid_str.len() > 8 { format!("{}...", &pid_str[..6]) } else { pid_str };
                                let n = *count;
                                view! {
                                    <li class="sync-queue-row" role="listitem" tabindex="0">
                                        <span class="sync-queue-row__name">{short}</span>
                                        <span class="sync-queue-row__count queue-pill">
                                            {icons::icon_hourglass_sm()}
                                            <span aria-hidden="true">{format!("pending · {n}")}</span>
                                        </span>
                                    </li>
                                }
                            })
                            .collect()
                    };
                    rows
                }}
            </ul>

            // ── Recent arrivals ─────────────────────────────────────
            <Show when=move || !queue_view.get().recent_arrivals.is_empty()>
                <section class="sync-queue-view__arrivals" role="list" aria-label="recent arrivals">
                    <h3 class="sync-queue-view__arrivals-title">{sync_queue_copy::SCREEN_SECTION_RECENT}</h3>
                    <ul>
                        {move || {
                            queue_view.get().recent_arrivals.iter().map(|a| {
                                let peer_str = a.peer_id.to_string();
                                let short = if peer_str.len() > 8 { format!("{}...", &peer_str[..6]) } else { peer_str };
                                let count = a.count;
                                view! {
                                    <li role="listitem" class="sync-queue-arrival">
                                        <span class="sync-queue-arrival__name">{short}</span>
                                        <span class="sync-queue-arrival__pill">
                                            {icons::icon_check_small()}
                                            <span>{format!("synced · {count}")}</span>
                                        </span>
                                    </li>
                                }
                            }).collect::<Vec<_>>()
                        }}
                    </ul>
                </section>
            </Show>

            // ── Footer ──────────────────────────────────────────────
            <footer class="sync-queue-view__footer">
                <button
                    type="button"
                    class="sync-queue-view__retry"
                    aria-busy=move || busy.get().to_string()
                    disabled=move || busy.get() || queue_view.get().depth == 0
                    on:click=retry_click
                >
                    {move || if busy.get() { sync_queue_copy::ACTION_RETRY_BUSY } else { sync_queue_copy::ACTION_RETRY }}
                </button>
                <Show when=move || tab.get() == Tab::Inbound>
                    <button
                        type="button"
                        class="sync-queue-view__mark-read"
                        on:click=mark_read_click.clone()
                    >
                        {sync_queue_copy::ACTION_MARK_READ}
                    </button>
                </Show>
            </footer>

            // ── Footnote (verbatim) ─────────────────────────────────
            <p class="sync-queue-view__footnote">
                {icons::icon_signal()}
                " "
                {sync_queue_copy::SCREEN_FOOTNOTE}
            </p>
        </section>
    }
}
