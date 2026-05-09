//! `<DragOverlay>` — full-viewport drop target for file uploads.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/files-inline.md`
//! §Drag-and-drop (desktop).
//!
//! Mounts at the app root, installs page-level `dragenter`,
//! `dragover`, `dragleave`, and `drop` listeners on `document`, and
//! renders a tinted full-viewport overlay (with a dashed panel + 32 px
//! upload icon + the spec copy `drop to attach`) while a file drag is
//! in progress. Dropping anywhere in the window enqueues the dropped
//! files into the shared `UploadQueue` and flips `queue.open` so the
//! `<UploadDialog>` mounts with the rows already populated.
//!
//! The overlay's visibility is owned by `queue.drag_active`. Two
//! reasons it lives in the queue (rather than in a separate context):
//! 1. The drag-overlay → dialog handoff is sequential; sharing one
//!    state struct keeps the wiring honest.
//! 2. Future paste-to-upload (T12) wants to enqueue into the same
//!    queue without the overlay appearing — keeping the surface
//!    cohesive avoids two contexts to plumb.

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use super::upload_dialog::enqueue_file_list;
use crate::app::WebClientHandle;
use crate::icons;
use crate::upload_state::use_upload_queue;

/// Page-level drag-and-drop overlay. Mount once at the app root.
#[component]
pub fn DragOverlay() -> impl IntoView {
    let queue = use_upload_queue();
    let handle = use_context::<WebClientHandle>();

    install_drag_listeners(queue, handle);

    view! {
        <Show when=move || queue.drag_active.get()>
            <div
                class="drag-overlay"
                role="presentation"
                aria-hidden="true"
            >
                <div class="drag-overlay__panel">
                    <span class="drag-overlay__icon">
                        {icons::icon_upload()}
                    </span>
                    <span class="drag-overlay__label">"drop to attach"</span>
                </div>
            </div>
        </Show>
    }
}

/// Install document-level drag/drop listeners; tear them down when
/// the owning component unmounts. The listeners drive
/// `queue.drag_active` (overlay visibility) and on `drop` enqueue
/// files into `queue.entries` + flip `queue.open` so the
/// `<UploadDialog>` mounts.
///
/// `dragenter` and `dragleave` fire repeatedly as the cursor crosses
/// child elements; we use a depth counter (incremented on enter,
/// decremented on leave) so the overlay only hides when the cursor
/// truly leaves the window (depth == 0).
fn install_drag_listeners(
    queue: crate::upload_state::UploadQueue,
    handle: Option<WebClientHandle>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let target: web_sys::EventTarget = document.into();

    // Stored on the heap so each listener has its own counter
    // reference. Leptos' single-threaded WASM environment makes
    // Rc<Cell<_>> the canonical pattern (see CLAUDE.md state-mgmt
    // table: WASM single-threaded interior mutability).
    use std::cell::Cell;
    use std::rc::Rc;
    let depth = Rc::new(Cell::new(0i32));

    // dragenter: bump depth, surface overlay if this is the first
    // enter event of the gesture and the drag carries files.
    let depth_enter = depth.clone();
    let on_enter = Closure::<dyn FnMut(_)>::new(move |ev: web_sys::DragEvent| {
        if !drag_carries_files(&ev) {
            return;
        }
        let next = depth_enter.get() + 1;
        depth_enter.set(next);
        if next == 1 {
            queue.drag_active.set(true);
        }
    });
    let _ = target.add_event_listener_with_callback("dragenter", on_enter.as_ref().unchecked_ref());

    // dragover: must call preventDefault so the browser permits a
    // subsequent drop on this target. Without this the drop event
    // never fires at the page level.
    let on_over = Closure::<dyn FnMut(_)>::new(move |ev: web_sys::DragEvent| {
        if drag_carries_files(&ev) {
            ev.prevent_default();
        }
    });
    let _ = target.add_event_listener_with_callback("dragover", on_over.as_ref().unchecked_ref());

    // dragleave: decrement depth, hide overlay when zero. When the
    // event's `related_target` is null, the cursor truly left the
    // window — Firefox can drop the final dragleave during nested
    // drags, so we treat that as a hard reset and force depth to 0
    // regardless. (Spec doesn't pin a "stuck overlay" recovery; this
    // is the cheapest one that doesn't ask the user to refresh.)
    let depth_leave = depth.clone();
    let on_leave = Closure::<dyn FnMut(_)>::new(move |ev: web_sys::DragEvent| {
        let leaving_window = ev.related_target().is_none();
        let next = if leaving_window {
            0
        } else {
            (depth_leave.get() - 1).max(0)
        };
        depth_leave.set(next);
        if next == 0 {
            queue.drag_active.set(false);
        }
    });
    let _ = target.add_event_listener_with_callback("dragleave", on_leave.as_ref().unchecked_ref());

    // drop: enqueue the dropped files, reset depth, surface dialog.
    let depth_drop = depth.clone();
    let on_drop = Closure::<dyn FnMut(_)>::new(move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        depth_drop.set(0);
        queue.drag_active.set(false);

        let Some(dt) = ev.data_transfer() else {
            return;
        };
        let Some(files) = dt.files() else {
            return;
        };
        let enqueued = enqueue_file_list(queue, handle.clone(), &files, |_, _| None);
        if enqueued > 0 {
            queue.open.set(true);
        }
    });
    let _ = target.add_event_listener_with_callback("drop", on_drop.as_ref().unchecked_ref());

    // Stash closures so they survive past this function and clean
    // them up when the owning component unmounts.
    let target_for_cleanup = target.clone();
    let enter_value = on_enter.into_js_value();
    let over_value = on_over.into_js_value();
    let leave_value = on_leave.into_js_value();
    let drop_value = on_drop.into_js_value();
    on_cleanup(move || {
        let _ = target_for_cleanup
            .remove_event_listener_with_callback("dragenter", enter_value.unchecked_ref());
        let _ = target_for_cleanup
            .remove_event_listener_with_callback("dragover", over_value.unchecked_ref());
        let _ = target_for_cleanup
            .remove_event_listener_with_callback("dragleave", leave_value.unchecked_ref());
        let _ = target_for_cleanup
            .remove_event_listener_with_callback("drop", drop_value.unchecked_ref());
    });
}

/// Returns `true` when the drag carries at least one file payload.
/// Filters out text-only drags (e.g. dragging selected text from
/// another tab) so the overlay only appears for actual file drops.
fn drag_carries_files(ev: &web_sys::DragEvent) -> bool {
    let Some(dt) = ev.data_transfer() else {
        return false;
    };
    let types = dt.types();
    for i in 0..types.length() {
        if let Some(t) = types.get(i).as_string() {
            if t == "Files" {
                return true;
            }
        }
    }
    false
}
