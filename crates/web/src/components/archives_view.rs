//! Phase 2d — archives surface, auto-archived subgroup.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`
//! §Archive surface.

use leptos::prelude::*;

use super::kind_chip::{KindChip, KindChipKind};
use willow_client::{ArchivedChannelSummary, ArchivesView};

/// The archives surface. Renders the auto-archived subgroup of
/// ephemeral channels with a `revive` link beside each row.
#[component]
pub fn ArchivesPane(view: Signal<ArchivesView>, on_revive: Callback<String>) -> impl IntoView {
    view! {
        <section class="archives-pane" aria-label="archives">
            <header class="archives-subgroup-header">
                {"auto-archived"}
            </header>
            <ul class="archives-subgroup archives-subgroup--auto">
                <For
                    each=move || view.get().entries
                    key=|e| e.channel_id.clone()
                    let:entry
                >
                    {render_row(entry, on_revive)}
                </For>
            </ul>
        </section>
    }
}

fn render_row(entry: ArchivedChannelSummary, on_revive: Callback<String>) -> impl IntoView {
    let chip = match entry.kind {
        willow_state::EphemeralKind::Channel => KindChipKind::Channel,
        willow_state::EphemeralKind::Thread => KindChipKind::Thread,
        willow_state::EphemeralKind::Whisper => KindChipKind::Whisper,
    };
    let name = entry.name.clone();
    let name_for_data = name.clone();
    let name_for_text = name.clone();
    let name_for_revive = name;
    view! {
        <li class="archives-row" data-channel-name=name_for_data>
            <KindChip kind=chip/>
            <span class="archives-row-name">{name_for_text}</span>
            <button
                class="archives-revive-link"
                type="button"
                on:click=move |_| on_revive.run(name_for_revive.clone())
            >
                "revive"
            </button>
        </li>
    }
}
