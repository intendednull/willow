//! Toast stack — in-app transient notifications.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/notifications.md`
//!
//! Exposes `Toast`, `Severity`, `ToastAction`, and the `ToastStackView`
//! component plus the `use_toasts()` helper for callers to push toasts.
//! The stack lives in a Leptos context so every component can queue a
//! toast without passing signals around.
//!
//! Rules enforced here:
//! - max 3 visible toasts; a 4th collapses the oldest into a `"{n} more"`
//!   pill (built in task 5),
//! - actionless toasts auto-dismiss after 4 s; hover pauses the timer,
//! - arrivals with the same `dedup_key` replace the active entry and
//!   reset the timer (coalescing).
//!
//! The portal target `#toast-root` is mounted once in `index.html` with
//! `aria-live="polite"` / `aria-relevant="additions"`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen::JsCast;

/// Severity of a toast — drives icon + border + aria-live routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Severity {
    /// Informational. No icon; polite live region.
    #[default]
    Info,
    /// Positive confirmation. `check` icon; polite.
    Success,
    /// Needs attention. `hourglass` icon; assertive.
    Warn,
    /// Failure. `x` icon; assertive.
    Err,
}

impl Severity {
    /// Return `"status"` (polite) for info/success, `"alert"`
    /// (assertive) for warn/err. Drives the toast `role` attribute.
    pub fn aria_role(self) -> &'static str {
        match self {
            Severity::Info | Severity::Success => "status",
            Severity::Warn | Severity::Err => "alert",
        }
    }

    /// CSS modifier for the toast container.
    pub fn class_suffix(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Success => "success",
            Severity::Warn => "warn",
            Severity::Err => "err",
        }
    }
}

/// Optional action pill rendered at the end of a toast.
#[derive(Clone)]
pub struct ToastAction {
    /// Button label — keep under 12 chars.
    pub label: String,
    /// Fired when the user activates the pill. Wrapped in
    /// [`SendWrapper`] to satisfy Leptos's `Send + Sync` bounds on
    /// single-threaded WASM.
    pub on_activate: SendWrapper<Rc<dyn Fn()>>,
}

impl std::fmt::Debug for ToastAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToastAction")
            .field("label", &self.label)
            .finish()
    }
}

/// A toast in the stack. Construct via [`Toast::info`] / [`Toast::success`] /
/// [`Toast::warn`] / [`Toast::err`] and push via [`ToastStack::push`].
#[derive(Clone, Debug)]
pub struct Toast {
    /// Unique id assigned by the stack. Zero before push.
    pub id: u64,
    pub severity: Severity,
    /// Required, ≤ 80 chars — truncated elsewhere.
    pub title: String,
    /// Optional body line, ≤ 140 chars.
    pub body: Option<String>,
    pub action: Option<ToastAction>,
    /// Sticky toasts persist until dismissed or state resolves.
    pub sticky: bool,
    /// Latest-wins — a new toast with the same key replaces the prior
    /// one and resets the auto-dismiss timer.
    pub dedup_key: Option<String>,
}

impl Toast {
    /// Builder for an `info` toast.
    pub fn info(title: impl Into<String>) -> ToastBuilder {
        ToastBuilder::new(Severity::Info, title.into())
    }
    /// Builder for a `success` toast.
    pub fn success(title: impl Into<String>) -> ToastBuilder {
        ToastBuilder::new(Severity::Success, title.into())
    }
    /// Builder for a `warn` toast.
    pub fn warn(title: impl Into<String>) -> ToastBuilder {
        ToastBuilder::new(Severity::Warn, title.into())
    }
    /// Builder for an `err` toast.
    pub fn err(title: impl Into<String>) -> ToastBuilder {
        ToastBuilder::new(Severity::Err, title.into())
    }
}

/// Builder for a [`Toast`]. Chainable; finish with `.build()`.
#[derive(Clone, Debug)]
pub struct ToastBuilder {
    inner: Toast,
}

impl ToastBuilder {
    fn new(severity: Severity, title: String) -> Self {
        Self {
            inner: Toast {
                id: 0,
                severity,
                title,
                body: None,
                action: None,
                sticky: false,
                dedup_key: None,
            },
        }
    }

    /// Set the optional body line.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.inner.body = Some(body.into());
        self
    }

    /// Attach a single action.
    pub fn action(mut self, label: impl Into<String>, on_activate: Rc<dyn Fn()>) -> Self {
        self.inner.action = Some(ToastAction {
            label: label.into(),
            on_activate: SendWrapper::new(on_activate),
        });
        self
    }

    /// Mark the toast as sticky (never auto-dismisses).
    pub fn sticky(mut self) -> Self {
        self.inner.sticky = true;
        self
    }

    /// Set a dedup key. New arrivals with the same key replace in place.
    pub fn dedup(mut self, key: impl Into<String>) -> Self {
        self.inner.dedup_key = Some(key.into());
        self
    }

    /// Finalize the builder.
    pub fn build(self) -> Toast {
        self.inner
    }
}

/// Auto-dismiss duration for non-sticky, action-less toasts (ms).
pub const TOAST_AUTO_DISMISS_MS: i32 = 4_000;

/// Maximum toasts rendered inline before the overflow pill appears.
pub const TOAST_MAX_VISIBLE: usize = 3;

/// The reactive toast stack. Cloneable handle — stored in Leptos context
/// via [`provide_toast_stack`] / [`use_toasts`].
#[derive(Clone)]
pub struct ToastStack {
    toasts: RwSignal<Vec<Toast>>,
    next_id: SendWrapper<Rc<std::cell::Cell<u64>>>,
    /// Active dismiss timers keyed by toast id.
    timers: SendWrapper<Rc<RefCell<HashMap<u64, i32>>>>,
}

impl Default for ToastStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastStack {
    /// Construct a fresh stack.
    pub fn new() -> Self {
        Self {
            toasts: RwSignal::new(Vec::new()),
            next_id: SendWrapper::new(Rc::new(std::cell::Cell::new(1))),
            timers: SendWrapper::new(Rc::new(RefCell::new(HashMap::new()))),
        }
    }

    /// Read-only access to the current stack.
    pub fn toasts(&self) -> ReadSignal<Vec<Toast>> {
        self.toasts.read_only()
    }

    /// Push a toast. If the dedup key matches an existing toast, the
    /// prior one is replaced and the timer resets. Returns the stable
    /// toast id.
    pub fn push(&self, mut toast: Toast) -> u64 {
        // Dedup — replace-in-place and reset the timer.
        if let Some(key) = toast.dedup_key.clone() {
            let mut replaced_id = None;
            self.toasts.update(|list| {
                if let Some(pos) = list.iter().position(|t| t.dedup_key.as_ref() == Some(&key)) {
                    let prior = &list[pos];
                    replaced_id = Some(prior.id);
                    toast.id = prior.id;
                    list[pos] = toast.clone();
                }
            });
            if let Some(id) = replaced_id {
                self.cancel_timer(id);
                self.arm_timer(id, &toast);
                return id;
            }
        }

        // Fresh push.
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        toast.id = id;
        self.toasts.update(|list| list.push(toast.clone()));
        self.arm_timer(id, &toast);
        id
    }

    /// Dismiss a toast by id. Cancels its timer.
    pub fn dismiss(&self, id: u64) {
        self.cancel_timer(id);
        self.toasts.update(|list| list.retain(|t| t.id != id));
    }

    /// Pause the auto-dismiss timer for a toast (e.g. on hover).
    pub fn pause(&self, id: u64) {
        self.cancel_timer(id);
    }

    /// Resume the auto-dismiss timer for a toast with the default
    /// duration. Called on mouse-leave.
    pub fn resume(&self, id: u64) {
        let toasts = self.toasts.get_untracked();
        if let Some(t) = toasts.iter().find(|t| t.id == id).cloned() {
            self.arm_timer(id, &t);
        }
    }

    fn arm_timer(&self, id: u64, toast: &Toast) {
        if toast.sticky || toast.action.is_some() {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let this = self.clone();
        let cb =
            wasm_bindgen::closure::Closure::once_into_js(move || {
                this.dismiss(id);
            });
        if let Ok(handle) = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.unchecked_ref(),
                TOAST_AUTO_DISMISS_MS,
            )
        {
            self.timers.borrow_mut().insert(id, handle);
        }
    }

    fn cancel_timer(&self, id: u64) {
        if let Some(handle) = self.timers.borrow_mut().remove(&id) {
            if let Some(window) = web_sys::window() {
                window.clear_timeout_with_handle(handle);
            }
        }
    }
}

/// Inject a [`ToastStack`] into the current reactive scope.
pub fn provide_toast_stack() -> ToastStack {
    if let Some(existing) = use_context::<ToastStack>() {
        return existing;
    }
    let stack = ToastStack::new();
    provide_context(stack.clone());
    stack
}

/// Retrieve the ambient [`ToastStack`]. Panics if the caller forgot to
/// `provide_toast_stack()` higher up the tree — that is a bug.
pub fn use_toasts() -> ToastStack {
    use_context::<ToastStack>()
        .expect("ToastStack not provided — call provide_toast_stack() in app root")
}

/// The toast stack view. Mount once near the root — renders into the
/// `#toast-root` portal declared in `index.html` so it floats above all
/// layout surfaces.
#[component]
pub fn ToastStackView() -> impl IntoView {
    let stack = provide_toast_stack();
    let toasts = stack.toasts();
    let stack_dismiss = stack.clone();
    let stack_hover = stack.clone();
    let stack_leave = stack.clone();

    view! {
        <div class="toast-stack" role="log" aria-live="polite" aria-relevant="additions">
            <For
                each=move || {
                    let all = toasts.get();
                    // Render at most TOAST_MAX_VISIBLE — the rest
                    // aggregate into the "{n} more" pill. The oldest
                    // toasts fall off first so the pill always
                    // represents a backlog behind current focus.
                    if all.len() <= TOAST_MAX_VISIBLE {
                        all
                    } else {
                        all.into_iter().rev().take(TOAST_MAX_VISIBLE).rev().collect()
                    }
                }
                key=|t: &Toast| t.id
                let:toast
            >
                {
                    let stack_for_close = stack_dismiss.clone();
                    let stack_for_hover = stack_hover.clone();
                    let stack_for_leave = stack_leave.clone();
                    let id = toast.id;
                    let class = format!("toast toast--{}", toast.severity.class_suffix());
                    let role = toast.severity.aria_role();
                    let title = toast.title.clone();
                    let body = toast.body.clone();
                    let action = toast.action.clone();
                    view! {
                        <div
                            class=class
                            role=role
                            data-toast-id=id.to_string()
                            tabindex="0"
                            on:mouseenter=move |_| stack_for_hover.pause(id)
                            on:mouseleave=move |_| stack_for_leave.resume(id)
                        >
                            <div class="toast-body">
                                <div class="toast-title">{title}</div>
                                {body.map(|b| view! { <div class="toast-desc">{b}</div> })}
                            </div>
                            {action.map(|a| {
                                let label = a.label.clone();
                                let cb = a.on_activate.clone();
                                let stack_for_action = stack_for_close.clone();
                                view! {
                                    <button
                                        class="toast-action"
                                        on:click=move |_| {
                                            (cb)();
                                            stack_for_action.dismiss(id);
                                        }
                                    >{label}</button>
                                }
                            })}
                            <button
                                class="toast-close"
                                aria-label="dismiss"
                                on:click=move |_| stack_for_close.dismiss(id)
                            >
                                "x"
                            </button>
                        </div>
                    }
                }
            </For>
            {move || {
                let len = toasts.get().len();
                if len > TOAST_MAX_VISIBLE {
                    let n = len - TOAST_MAX_VISIBLE;
                    Some(view! {
                        <div
                            class="toast-overflow-pill"
                            role="button"
                            tabindex="0"
                            aria-label=format!("{n} more notifications, activate to expand")
                        >
                            {format!("{n} more")}
                        </div>
                    })
                } else {
                    None
                }
            }}
        </div>
    }
}
