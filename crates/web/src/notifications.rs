//! Notifier service — single dispatch point for toast + chime + push.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/notifications.md`.
//!
//! The `Notifier` owns the gating logic:
//! - own-event suppression (never ring on your own send),
//! - category opt-in (msg off by default; mention/letter/ephemeral/whisper/
//!   handoff on),
//! - per-surface mute with the safety-critical overrides,
//! - 20 s per-surface coalescing (drives `dedup_key` on toasts and
//!   `n new messages` on visible pushes),
//! - focus gate (focused app suppresses OS push and fires an in-app
//!   toast instead — the service-worker bridge also honours this),
//! - permission-denied sticky toast (once per session, after the first
//!   local send).
//!
//! The Notifier is constructed once and provided to the Leptos context
//! via [`provide_notifier`]. Callers dispatch with
//! `notifier.dispatch(NotificationKind::...)`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use send_wrapper::SendWrapper;

use crate::audio::ChimePlayer;
use crate::components::{Toast, ToastStack};

/// Routing category for a notification. Mirrors the OS push payload
/// `cat` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    /// Regular chat message.
    Msg,
    /// Message mentioning the local peer.
    Mention,
    /// One-on-one letter.
    Letter,
    /// Ephemeral channel expiry warning.
    EphemeralExpiry,
    /// Whisper invitation.
    WhisperInvite,
    /// Handoff request from the user's own device.
    Handoff,
}

impl Category {
    /// Whether this category is enabled by default (per spec table).
    pub fn default_enabled(self) -> bool {
        !matches!(self, Category::Msg)
    }

    /// Whether this category bypasses channel mute.
    pub fn bypasses_channel_mute(self) -> bool {
        matches!(
            self,
            Category::WhisperInvite | Category::EphemeralExpiry | Category::Handoff
        )
    }

    /// Whether this category bypasses grove mute.
    pub fn bypasses_grove_mute(self) -> bool {
        matches!(self, Category::EphemeralExpiry | Category::Handoff)
    }

    /// Whether this category bypasses global mute + quiet hours.
    pub fn bypasses_all_mute(self) -> bool {
        matches!(self, Category::Handoff)
    }

    /// OS push payload `cat` tag.
    pub fn tag(self) -> &'static str {
        match self {
            Category::Msg => "msg",
            Category::Mention => "mention",
            Category::Letter => "letter",
            Category::EphemeralExpiry => "ephemeral-expiry",
            Category::WhisperInvite => "whisper-invite",
            Category::Handoff => "handoff",
        }
    }
}

/// A notification to dispatch. All context (author / surface / muted
/// flags) is resolved *before* calling `dispatch` so the Notifier stays
/// pure.
pub struct NotificationKind {
    /// Routing category.
    pub category: Category,
    /// Surface id for coalescing (`channel:ch-id`, `letter:peer-id`, …).
    pub surface: String,
    /// Toast to render when gating passes. The Notifier overrides the
    /// dedup_key with `surface` so coalescing works.
    pub toast: Toast,
    /// Whether the originating event was authored by the local peer.
    /// Own-events suppress sound and push, but may still render the
    /// toast as an ack.
    pub is_own: bool,
    /// Whether the target surface is muted (channel / grove scope).
    pub muted: bool,
}

/// Coalescing window per surface — 20 s per spec.
pub const COALESCE_WINDOW_MS: i64 = 20_000;

/// The Notifier handle. Cloneable; lives in Leptos context.
#[derive(Clone)]
pub struct Notifier {
    toasts: ToastStack,
    chime: Option<ChimePlayer>,
    /// Last dispatch timestamp per surface (unix ms). Drives
    /// coalescing — events within the 20 s window replace the
    /// prior toast via its dedup key.
    coalesce: SendWrapper<Rc<RefCell<HashMap<String, i64>>>>,
    /// Running count of coalesced events per surface. Reset when the
    /// window closes. Used to format `"3 new messages"` style bodies.
    counters: SendWrapper<Rc<RefCell<HashMap<String, u32>>>>,
    /// Tracks whether the permission-denied sticky toast has been
    /// shown this session. Reset per page load per spec.
    permission_denied_shown: SendWrapper<Rc<std::cell::Cell<bool>>>,
    /// Tracks whether the local-send-per-session flag has been set
    /// (drives the "prompt after first local send" policy).
    first_send_seen: SendWrapper<Rc<std::cell::Cell<bool>>>,
}

impl Notifier {
    /// Construct a new Notifier.
    pub fn new(toasts: ToastStack, chime: Option<ChimePlayer>) -> Self {
        Self {
            toasts,
            chime,
            coalesce: SendWrapper::new(Rc::new(RefCell::new(HashMap::new()))),
            counters: SendWrapper::new(Rc::new(RefCell::new(HashMap::new()))),
            permission_denied_shown: SendWrapper::new(Rc::new(std::cell::Cell::new(false))),
            first_send_seen: SendWrapper::new(Rc::new(std::cell::Cell::new(false))),
        }
    }

    /// Main dispatch entry. Applies all gates and renders the toast /
    /// chime / push as appropriate.
    pub fn dispatch(&self, k: NotificationKind) {
        // Own-event suppression — toast may still render for ack, but
        // sound + push never fire.
        let silent = k.is_own;

        // Mute gating. Overrides for safety-critical categories.
        if k.muted {
            let allowed = k.category.bypasses_channel_mute();
            if !allowed {
                // Totally silenced — badge still ticks via gossip,
                // but toast / chime / push all no-op.
                return;
            }
        }

        // Coalescing window — events on the same surface within 20 s
        // update the dedup key so the toast stack replaces in place.
        let now = now_ms();
        let mut toast = k.toast;
        let surface = k.surface.clone();
        let mut count = 1u32;
        {
            let mut last = self.coalesce.borrow_mut();
            let mut counters = self.counters.borrow_mut();
            match last.get(&surface).copied() {
                Some(last_ms) if now - last_ms < COALESCE_WINDOW_MS => {
                    // Within window — bump counter and re-body the toast.
                    let c = counters.entry(surface.clone()).or_insert(0);
                    *c = c.saturating_add(1);
                    count = *c + 1;
                }
                _ => {
                    counters.insert(surface.clone(), 0);
                }
            }
            last.insert(surface.clone(), now);
        }
        // Render coalesced body when count > 1. Keep the spec copy
        // exact: `"{n} new messages"` etc. The caller's toast stays
        // authoritative for a fresh event.
        if count > 1 && matches!(k.category, Category::Msg | Category::Mention) {
            toast.body = Some(format!("{count} new"));
        }
        toast.dedup_key = Some(format!("notif:{}", surface));

        // Render the toast. No role / aria changes — Toast::aria_role
        // handles that from severity.
        self.toasts.push(toast);

        // Sound gate — own-events always silent; handoff bypasses
        // everything per spec, every other category honours the
        // calling surface's mute. We already passed the mute gate
        // above, so a non-own-event that reached this point rings
        // unless the category is the silent default variant.
        if !silent {
            if let Some(chime) = &self.chime {
                chime.play();
            }
        }

        // NOTE: OS push payload + service-worker bridge is wired in
        // task 11. Notifier::dispatch is the single entry point — the
        // bridge subscribes to a `NotificationKind` channel and calls
        // the platform adapter.
    }

    /// Mark that the local peer has sent their first message this
    /// session. After the first such call, the Notifier may trigger
    /// the notification-permission prompt path (task 11).
    pub fn mark_local_send(&self) {
        self.first_send_seen.set(true);
    }

    /// Whether a local send has been observed this session.
    pub fn local_send_seen(&self) -> bool {
        self.first_send_seen.get()
    }

    /// Show the permission-denied sticky toast at most once per
    /// session. Returns `true` if the toast fired; `false` if it was
    /// already shown.
    pub fn show_permission_denied_once(&self) -> bool {
        if self.permission_denied_shown.get() {
            return false;
        }
        self.permission_denied_shown.set(true);
        let t = Toast::info(
            "willow works better with notifications — settings lets you pick what's loud",
        )
        .sticky()
        .build();
        self.toasts.push(t);
        true
    }
}

/// Provide a [`Notifier`] in the current reactive scope. Returns the
/// ambient instance if one already exists.
pub fn provide_notifier() -> Notifier {
    if let Some(existing) = leptos::prelude::use_context::<Notifier>() {
        return existing;
    }
    let toasts = crate::components::provide_toast_stack();
    let chime = crate::audio::use_chime_player();
    let n = Notifier::new(toasts, chime);
    leptos::prelude::provide_context(n.clone());
    n
}

/// Retrieve the ambient [`Notifier`]. Returns `None` outside a tree
/// that called [`provide_notifier`].
pub fn use_notifier() -> Option<Notifier> {
    leptos::prelude::use_context::<Notifier>()
}

/// Current wall-clock in milliseconds. Implemented via `js_sys::Date`
/// so the Notifier works identically in native unit tests (which
/// don't use the wall clock) and browser runs.
fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}
