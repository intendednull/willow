//! Mobile bottom tab bar — four primary routes + badges.
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Tab bar
//!
//! - iOS viewport renders a blurred translucent bar.
//! - Android viewport renders a solid bar with a 54 × 28 pill behind
//!   the active icon.
//! - Both paths share typography, spacing, color tokens.

use leptos::prelude::*;

use crate::components::mobile_shell::MobileTab;
use crate::icons;

/// Bottom tab bar.
#[component]
pub fn TabBar(
    /// Currently-selected tab.
    #[prop(into)]
    active: Signal<MobileTab>,
    /// Unread badges, keyed by tab id ("home" / "letters" / ...).
    #[prop(into)]
    badges: Signal<Vec<(String, usize)>>,
    /// Hide the tab bar when a full-screen push is on screen.
    #[prop(into)]
    visible: Signal<bool>,
    /// Called with the newly-selected tab on click.
    #[prop(into)]
    on_tab_change: Callback<MobileTab>,
) -> impl IntoView {
    let tabs = [
        MobileTab::Home,
        MobileTab::Letters,
        MobileTab::Discover,
        MobileTab::You,
    ];

    view! {
        <nav
            class="mobile-tab-bar"
            role="navigation"
            aria-label="primary"
            data-visible=move || if visible.get() { "true" } else { "false" }
        >
            {tabs.iter().map(|tab| {
                let tab = *tab;
                let id = tab.id();
                let label = tab.label();
                let active_for_class = active;
                let active_for_aria = active;
                let badges_for_tab = badges;
                view! {
                    <button
                        class=move || if active_for_class.get() == tab {
                            "tab tab-active".to_string()
                        } else {
                            "tab".to_string()
                        }
                        role="tab"
                        aria-label=label
                        aria-selected=move || {
                            if active_for_aria.get() == tab { "true" } else { "false" }
                        }
                        data-tab=id
                        on:click=move |_| on_tab_change.run(tab)
                    >
                        <span class="tab-icon" aria-hidden="true">
                            {tab_icon(tab)}
                        </span>
                        <span class="tab-label">{label}</span>
                        {move || {
                            let count = badges_for_tab
                                .get()
                                .into_iter()
                                .find(|(k, _)| k == id)
                                .map(|(_, v)| v)
                                .unwrap_or(0);
                            if count > 0 {
                                let text = if count > 99 {
                                    "99+".to_string()
                                } else {
                                    count.to_string()
                                };
                                Some(view! {
                                    <span class="tab-badge" aria-label=format!("{count} unread")>
                                        {text}
                                    </span>
                                })
                            } else {
                                None
                            }
                        }}
                    </button>
                }
            }).collect_view()}
        </nav>
    }
}

fn tab_icon(tab: MobileTab) -> leptos::tachys::view::any_view::AnyView {
    match tab {
        MobileTab::Home => icons::icon_hash().into_any(),
        MobileTab::Letters => icons::icon_thread().into_any(),
        MobileTab::Discover => icons::icon_compass().into_any(),
        MobileTab::You => icons::icon_user().into_any(),
    }
}
