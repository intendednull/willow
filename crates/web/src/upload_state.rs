//! Upload-queue context for the `<UploadDialog>`.
//!
//! Phase 3b shipped `<FileShareButton>` as a single-file path
//! straight to `upload_attachment` + `send_attachment_message`. The
//! spec (`docs/specs/2026-04-19-ui-design/files-inline.md`
//! §Upload dialog) calls for a modal sheet with a multi-file queue,
//! per-file progress + cancel, and a footer that batch-sends on
//! confirm. This module owns the session-scoped queue state — a
//! Leptos context shared between the composer attach button (which
//! flips the dialog open), the dialog itself (which renders + drives
//! the queue), and any future drag-overlay or paste handler that
//! enqueues files.

use leptos::prelude::*;
use willow_network::BlobHash;

/// Per-file upload progression state.
///
/// Today the underlying iroh blob store doesn't surface incremental
/// progress events — `upload_attachment` resolves once the whole
/// blob lands. The dialog renders the row as `Uploading` until the
/// future resolves, then flips to `Done(_)` or `Failed(_)`. A future
/// blob-store-progress hook can swap `Uploading` for a 0..1 ratio
/// without changing this module's surface.
#[derive(Debug, Clone, PartialEq)]
pub enum UploadStatus {
    /// Upload in flight; no progress signal yet.
    Uploading,
    /// Bytes landed in the blob store; ready to attach. Carries the
    /// content-addressed hash for the eventual `FileMessage` send.
    Done(BlobHash),
    /// Upload failed; error string surfaces in the row.
    Failed(String),
}

/// One entry in the upload queue. The `status` signal is independent
/// per entry so the dialog can re-render a single row without
/// touching the rest of the queue.
#[derive(Debug, Clone)]
pub struct UploadEntry {
    /// Stable session-local id used for `<For>` keying + cancel.
    pub id: String,
    pub filename: String,
    pub mime: String,
    pub size: u64,
    pub status: RwSignal<UploadStatus>,
}

/// Session-scoped upload queue + dialog visibility. Provided once at
/// the app-shell layer and consumed by the composer attach button +
/// the `<UploadDialog>` itself.
#[derive(Clone, Copy)]
pub struct UploadQueue {
    /// Whether the `<UploadDialog>` is currently mounted.
    pub open: RwSignal<bool>,
    /// Pending + completed entries, oldest at the front.
    pub entries: RwSignal<Vec<UploadEntry>>,
}

impl UploadQueue {
    pub fn new() -> Self {
        Self {
            open: RwSignal::new(false),
            entries: RwSignal::new(Vec::new()),
        }
    }

    /// Push a fresh entry into the queue. Returns the new entry's
    /// id so the caller can spawn the upload future and update its
    /// status when bytes resolve.
    pub fn push(
        &self,
        filename: String,
        mime: String,
        size: u64,
    ) -> (String, RwSignal<UploadStatus>) {
        let id = next_entry_id();
        let status = RwSignal::new(UploadStatus::Uploading);
        let entry = UploadEntry {
            id: id.clone(),
            filename,
            mime,
            size,
            status,
        };
        self.entries.update(|v| v.push(entry));
        (id, status)
    }

    /// Remove an entry by id. Used by the per-row cancel button.
    pub fn remove(&self, id: &str) {
        self.entries.update(|v| v.retain(|e| e.id != id));
    }

    /// Clear the queue + close the dialog. Used by the footer
    /// `cancel all` action.
    pub fn cancel_all(&self) {
        self.entries.update(|v| v.clear());
        self.open.set(false);
    }
}

impl Default for UploadQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Read the queue from context, providing a fresh one if absent
/// (e.g. unit-test mounts that don't construct the full app shell).
/// In production the app shell provides exactly one queue so the
/// composer button + the dialog see the same state.
pub fn use_upload_queue() -> UploadQueue {
    use_context::<UploadQueue>().unwrap_or_else(|| {
        let queue = UploadQueue::new();
        provide_context(queue);
        queue
    })
}

/// Monotonic counter for entry ids. Session-scoped and best-effort
/// — collisions are impossible in practice (uploads are user-driven
/// and can't fire faster than the counter increments).
fn next_entry_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("upload-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_entry_id_is_unique_within_session() {
        // Same module → same atomic counter; ids are guaranteed to
        // diverge across calls.
        let a = next_entry_id();
        let b = next_entry_id();
        assert_ne!(a, b);
        assert!(a.starts_with("upload-"));
    }
}
