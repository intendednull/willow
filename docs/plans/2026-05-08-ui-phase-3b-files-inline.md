# UI Phase 3b — Files & Inline Attachments Implementation Plan

**Status:** draft

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development. Every task = one commit; tick the checkbox in the same commit.

**Goal:** Ship `docs/specs/2026-04-19-ui-design/files-inline.md` — replace the legacy 256 KB base64 `[file:name:base64]` text-message hack with a real blob-backed attachment flow: `Content::File` rendered as inline images / file cards / voice-note players inside a message row, an `<UploadDialog>` modal driven from the composer's attach button, page-level drag-and-drop, paste-to-upload from the composer, and the large-file / image-degradation rules called out in the spec.

**Architecture:** Files travel as `Content::File { hash, filename, mime_type, size_bytes, width?, height? }` over the existing iroh blob transport. The web UI gains an `attachment` component module (`Image`, `FileCard`, `VoiceNote`) that the message-row picks between based on mime + size; an `<UploadDialog>` (modal sheet) owned by `chat_view` that the composer's attach button + DnD + paste all enqueue into; and a page-level `<DragOverlay>` mounted at app root. `Content::File` gets two new optional fields (`width`, `height`); new senders use them to reserve aspect-ratio space while bytes stream. Schema change is the only protocol-level adjustment; everything else is UI.

> **Wire compatibility note.** Adding fields to `Content::File` is a wire-breaking change under bincode (the wire format Willow uses today). bincode is positional and doesn't apply `#[serde(default)]` for missing fields, so a pre-3b peer cannot decode a post-3b `Content::File` payload and vice versa. Willow is pre-1.0 and peers update together; the plan accepts the break rather than introducing a `Content::FileV2` variant just to side-step it. Future-proofing the schema for additive evolution would require migrating off bincode (see open question), tracked separately.

**Tech Stack:** Rust + Leptos 0.8 (WASM), `willow-messaging` (schema field add), `willow-client` (upload helper + image-dimensions extraction), `willow-web` (attachment renderers + upload dialog + DnD + paste handlers). Reuses the existing iroh blob store; no new EventKinds. Foundation tokens only (no new hex). `just test-state`, `just test-client`, `just test-browser`, `just test-e2e-*` run in CI.

**Branch:** `ui/phase-3b-files-inline`. Commits `ui(phase-3b): <imperative>` (code), `state(phase-3b): <imperative>` for schema, `docs(plan): phase 3b — files & inline attachments implementation plan` for this plan. Lands as one PR; the plan ships in the same PR as the implementation. The first commit also stamps `composer.md` + `2026-04-26-ui-phase-3a-composer.md` as `landed (e46e4b3, 2026-05-08)` (carry-over from the phase-3a merge that protected-main blocked from a direct push).

---

## Scope

**In:**
- Add optional `width: Option<u32>`, `height: Option<u32>` to `Content::File`, `#[serde(default)]` for backward compatibility. State-machine round-trip + apply tests.
- `ClientHandle::upload_attachment(channel_id, filename, mime, bytes) -> Result<AttachmentHandle>` and `ClientHandle::send_attachment_message(channel_id, attachment, opt_caption)` — the latter routes through the existing `Content::File` send path. Image dimension extraction lives at the web layer and is supplied by callers.
- New `crates/web/src/components/attachment/` module: `mod.rs`, `image.rs`, `file_card.rs`, `voice_note.rs`. Pure presentational; reads `Content::File` fields.
- Replace the legacy base64 `[file:…]` rendering branch in `message.rs` with the typed `Content::File` rendering branch. Old base64 messages keep rendering for back-compat through the existing `parse_inline_file` path (legacy reader stays, legacy writer goes — the composer no longer emits the format).
- `<UploadDialog>` modal sheet: picker row (`choose files` button + `or drop files here` hint), per-file row (icon + filename + size + progress bar + cancel), footer (`cancel all` + `attach to message`). Aria labels per spec.
- Wire composer attach button → opens `<UploadDialog>` (replacing the existing `<FileShareButton>` direct-upload path).
- Page-level `<DragOverlay>` mounted at app root: drag-enter → tinted overlay + `drop to attach`; drop → opens `<UploadDialog>` with files enqueued.
- Composer textarea paste handler routes clipboard files / images to `<UploadDialog>`. Pasted images get filename `pasted-{YYYY-MM-DD-HH-mm-ss}.png`.
- File-rendering rules: image > 4 MB → render as file card; file > 10 MB → file card with `large · downloads on click` badge in `--amber`; unknown mime → file card.
- Voice-note card: HTML5 `<audio controls>` for playback, mm:ss / mm:ss timer, play / pause IconBtn. Single-instance playback (starting one pauses the previous) via a per-app `VoiceNotePlayer` actor (`StateActor<CurrentlyPlaying>`).
- Reduced-motion: progress bars + drag-overlay crossfade respect `prefers-reduced-motion: reduce` (foundation tokens already gate this).
- ARIA labels per spec table: `download {filename}` / `play voice note` / `pause voice note` / `cancel upload of {filename}` / `cancel all uploads` / `attach to message` / `drop to attach`.
- Browser tests for: file card render, image inline render, image > 4 MB degradation, file > 10 MB warning badge, upload dialog open / cancel / per-file cancel, drag overlay show / hide, paste-to-upload routing, voice-note play / pause / single-instance, ARIA label coverage.
- One Playwright spec for desktop drag-and-drop end-to-end (real browser drag events; `wasm-pack` can't fake DataTransfer reliably).

**Out (hand-off / explicit defer):**
- Voice-note **recording** (mic capture + encoding) — spec only discusses playback. Capture surface is owned by a future phase referenced from the call experience spec; phase 3b only renders received voice notes.
- Artistic waveform peaks — v1 voice-note card uses the browser's native `<audio controls>` and a numeric `mm:ss / mm:ss` timer; the styled bar-strip waveform is parked behind a TODO pointing back at this plan's open questions.
- Image lightbox / full-screen viewer beyond `target="_blank"` — clicking an inline image opens it in a new tab via the wrapping anchor; a true lightbox is deferred.
- Mobile drag-and-drop — touchscreens don't generate DataTransfer events. Mobile attach goes through the dialog only.
- Resumable / chunked upload UI — the iroh blob transport handles chunking transparently; the dialog shows one progress bar per file. A dedicated chunked-upload UI is deferred.
- Server-side file-type policies (allowlist / denylist) — per spec, any mime is accepted. Policy enforcement is a governance concern and will reference `governance.md` if it ever lands.

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/messaging/src/lib.rs` | modify | Add `width: Option<u32>` and `height: Option<u32>` to `Content::File` with `#[serde(default)]`. Update `Content::validate` to bound dimension values (reject above e.g. 16384 px to avoid layout abuse). Update existing `Message::file()` constructor + add a `file_with_dimensions()` constructor. |
| `crates/messaging/src/lib.rs` (tests) | modify | Add `content_file_round_trips_with_dimensions`, `content_file_round_trips_without_dimensions_back_compat`, `content_validate_rejects_oversize_dimensions`. |
| `crates/client/src/files.rs` | modify | Add `ClientHandle::upload_attachment(channel_id, filename, mime, bytes) -> Result<AttachmentHandle>` (returns `{ hash, size_bytes }`) wrapping the existing blob store. Add `send_attachment_message(channel_id, AttachmentMeta)` that posts a `Content::File`. Keep the legacy `share_file_inline` for back-compat reads but mark `#[deprecated]` with a pointer to the new API. |
| `crates/client/src/tests/files.rs` | **new** | `upload_attachment_returns_hash`, `upload_attachment_dedupes_by_hash`, `send_attachment_round_trips_through_state`, `send_attachment_carries_dimensions`. |
| `crates/web/src/components/attachment/mod.rs` | **new** | `pub mod image; pub mod file_card; pub mod voice_note; pub use image::AttachmentImage; pub use file_card::AttachmentFileCard; pub use voice_note::AttachmentVoiceNote;`. Plus `pub fn pick(content: &Content::File) -> AttachmentKind` (the rendering-rule decision: image / file_card / voice_note). |
| `crates/web/src/components/attachment/image.rs` | **new** | `<AttachmentImage hash filename mime_type size_bytes width height />`. Wraps `<img loading="lazy" decoding="async">` in an anchor that opens full-size in a new tab. Caption row below: `filename · size · e2e encrypted` in `--ink-3`. Failed loads swap to `<AttachmentFileCard>` via a fallback `Show`. Desktop `max-width: 380px` / mobile `max-width: 280px`. |
| `crates/web/src/components/attachment/file_card.rs` | **new** | `<AttachmentFileCard hash filename mime_type size_bytes />`. Mime icon (24×24, `--ink-2`), filename, size + mime hint, download IconBtn. Large-file warning badge when `size_bytes > 10 MB`: `large · downloads on click` in `--amber`. ARIA `download {filename}`. |
| `crates/web/src/components/attachment/voice_note.rs` | **new** | `<AttachmentVoiceNote hash filename size_bytes />`. Inline-flex card on `--bg-2`. Play / pause IconBtn (32×32, `--moss-2`), `<audio>` element (controls hidden — we drive it from the IconBtn), mm:ss / mm:ss timer. Subscribes to `VoiceNotePlayer` so starting a new one pauses any previously playing instance. ARIA `play voice note` / `pause voice note`. |
| `crates/web/src/voice_note_player.rs` | **new** | `pub struct VoiceNotePlayer` — single-instance playback coordinator. `RwSignal<Option<VoiceNoteId>>` for "currently playing"; `notify(id)` flips to `Some(id)` and the per-card effect pauses if its id no longer matches. Provided as a context at the app root. |
| `crates/web/src/components/upload_dialog.rs` | **new** | `<UploadDialog open files on_cancel_all on_attach />`. Modal sheet on `--bg-1`, `--shadow-2`, radius 12px. Picker row: `choose files` button + `or drop files here` hint. Per-file rows from a `Vec<UploadEntry>` reactive list. Footer with `cancel all` + `attach to message`. Wires `<input type=file multiple>` for browse. ARIA labels per spec. |
| `crates/web/src/components/drag_overlay.rs` | **new** | `<DragOverlay open />`. Mounted at app root (provided context for `(open, set_open)` shared with the page-level drag handler). Full-viewport tint + dashed `--moss-2` panel + `upload` icon + `drop to attach` label. ARIA `drop to attach`. |
| `crates/web/src/upload_state.rs` | **new** | `pub struct UploadQueue { entries: RwSignal<Vec<UploadEntry>>, … }` — reactive queue of in-flight uploads. Each `UploadEntry { id, filename, mime, size, progress: RwSignal<f32>, status: RwSignal<UploadStatus> }`. Provided as context in `chat_view`. Drives both `<UploadDialog>` rendering and the eventual `Content::File` send when the user confirms. |
| `crates/web/src/handlers.rs` | modify | Add `attach_drag_and_drop_listeners(set_overlay_open, push_to_queue)` mounted once at app start. Add `attach_paste_handler(textarea_ref, push_to_queue)` mounted by the composer. |
| `crates/web/src/components/composer/composer.rs` | modify | Replace the placeholder attach button click handler with one that opens the `<UploadDialog>` (read from context). Wire the textarea paste handler from `handlers::attach_paste_handler`. |
| `crates/web/src/components/file_share.rs` | modify | Mark the entire module `#[deprecated(note = "use crates/web/src/components/attachment/* instead")]`. Keep `parse_inline_file` and `format_file_size` exported (still used by the message-row's legacy reader). Remove `<FileShareButton>` callsite from `chat_view` / `mobile_shell`. |
| `crates/web/src/components/message.rs` | modify | Add a typed `Content::File { hash, filename, mime_type, size_bytes, width, height }` rendering branch that calls `attachment::pick()` and renders the appropriate `<AttachmentImage>` / `<AttachmentFileCard>` / `<AttachmentVoiceNote>`. Keep the legacy base64 `[file:…]` branch for back-compat reads, gated behind a debug-only deprecation log. |
| `crates/web/src/app.rs` | modify | Mount `<DragOverlay>` at app root. Provide `UploadQueue` + `VoiceNotePlayer` contexts. Drop the now-orphan `<FileShareButton>` import. |
| `crates/web/src/components/mobile_shell.rs` | modify | Same drop of `<FileShareButton>` import + callsite. The composer's attach button is shared between shells. |
| `crates/web/style.css` | modify | Append `.attachment`, `.attachment--image`, `.attachment--file-card`, `.attachment--voice-note`, `.attachment__caption`, `.attachment__warning`, `.upload-dialog`, `.upload-dialog__row`, `.upload-dialog__progress`, `.drag-overlay`, `.drag-overlay__panel`, `.drag-overlay__icon`. Foundation tokens only — `--bg-1`, `--bg-2`, `--line`, `--moss-1`, `--moss-2`, `--moss-4`, `--ink-0`, `--ink-2`, `--ink-3`, `--amber`, `--shadow-2`, `--motion-fast`. Drop the now-unused `.file-card`, `.file-icon`, `.file-info`, `.file-name`, `.file-size`, `.file-share-btn`, `.download-btn`. |
| `crates/web/tests/browser.rs` | modify | Append `mod phase_3b_attachments { … }` — ~14 tests covering AG-1 through AG-13. |
| `e2e/files-inline.spec.ts` | **new** | One Playwright spec exercising real desktop drag-and-drop end-to-end against the running web stack. |
| `docs/specs/2026-04-19-ui-design/composer.md` | modify | Stamp `**Status:** implementing → landed (e46e4b3, 2026-05-08)`. (Carry-over from phase 3a — main is protected and the merge happened upstream.) |
| `docs/plans/2026-04-26-ui-phase-3a-composer.md` | modify | Add `**Status:** landed (e46e4b3, 2026-05-08)` line at the top. (Carry-over from phase 3a.) |

## Acceptance gates

> Each gate maps to one or more browser/state/client tests. A task is complete when (a) the production code lands, (b) the corresponding test(s) pass, (c) the spec checkbox is ticked in the same commit.

- [ ] **AG-1.** `Content::File` round-trips with and without `width` / `height` and rejects oversized dimensions. Test: state-tier round-trip + validation.
- [ ] **AG-2.** `<AttachmentImage>` renders inline with `loading="lazy"`, anchor-wrapped, and the spec caption (`filename · size · e2e encrypted`). Test: browser-tier render + selector assertions.
- [ ] **AG-3.** `<AttachmentFileCard>` renders mime icon + filename + size + download IconBtn at the spec `max-width`. Test: browser-tier.
- [ ] **AG-4.** Image > 4 MB degrades to file card; file > 10 MB shows the `large · downloads on click` warning badge. Test: browser-tier render two `Content::File` fixtures, assert variant + warning.
- [ ] **AG-5.** `<AttachmentVoiceNote>` renders the play / pause IconBtn + mm:ss / mm:ss timer card. Test: browser-tier.
- [ ] **AG-6.** Starting one voice note pauses any other. Test: browser-tier mounts two cards, drives play on the first, then play on the second, asserts the first is paused.
- [ ] **AG-7.** `<UploadDialog>` opens from the composer attach button with the picker row + footer; per-file cancel removes the row; `cancel all` clears all rows. Test: browser-tier.
- [ ] **AG-8.** Drag-enter on the window shows `<DragOverlay>` with the spec copy; drag-leave / drop hides it. Test: browser-tier dispatches synthetic drag events on window.
- [ ] **AG-9.** Drop opens `<UploadDialog>` with the dropped files enqueued. Test: Playwright (real DataTransfer required).
- [ ] **AG-10.** Pasting an image into the composer routes to `<UploadDialog>` instead of inserting text; the file gets the `pasted-{ts}.png` filename. Test: browser-tier dispatches a synthetic ClipboardEvent.
- [ ] **AG-11.** ARIA labels match the spec table for every interactive element added in this phase. Test: browser-tier.
- [ ] **AG-12.** `prefers-reduced-motion: reduce` collapses the drag-overlay crossfade to an instant fade. Test: browser-tier.
- [ ] **AG-13.** Spec `files-inline.md` acceptance criteria checkboxes are ticked at the end of the plan (final task).

## Tasks

> Each task is one self-contained commit. Subagent contract: implement the production change(s), add the listed test(s), make them pass, run `just check` (or the targeted subset called out), tick the matching `[ ]` checkbox in this plan, then commit. No multi-task batches.

### Phase A — schema + carry-over stamp

- [x] **T0.** Stamp `composer.md` and the phase-3a plan as `landed (e46e4b3, 2026-05-08)` (carry-over the dropped main commit). Commit `docs(phase-3a): mark composer spec + plan as landed (carry-over)`. **Verify:** spec/plan diffs inspected.

- [x] **T1.** Extend `Content::File` with `width: Option<u32>` + `height: Option<u32>`, both `#[serde(default)]`. Bound dimensions in `Content::validate` (reject > 16384 px). Add `Message::file_with_dimensions()` constructor. Wire-breaking change under bincode — see plan §Architecture. **Tests:** `content_file_round_trips_with_dimensions`, `content_file_round_trips_without_dimensions`, `content_validate_rejects_oversize_dimensions`. **Verify:** `cargo test -p willow-messaging` (the schema tests live alongside `Content` in the messaging crate, not in `willow-state`).

### Phase B — client upload helper

- [x] **T2.** Add the EventKind, materialize handler, and client API to send file attachments through the typed event system instead of the legacy `[file:NAME:base64]` text-message hack:
  - `EventKind::FileMessage { channel_id, hash, filename, mime_type, size_bytes, width, height, body, reply_to }` in `willow-state` plus the materialize handler that produces a `ChatMessage` with `attachment: Some(_)`. `Permission::SendMessages`. Defense-in-depth bounds at apply: filename / mime / dimension caps.
  - `FileAttachment { hash, filename, mime_type, size_bytes, width, height }` struct on `ChatMessage` (new field `attachment: Option<FileAttachment>`, `#[serde(default)]`).
  - `ClientMutations::send_file_message(...)` builds and broadcasts the new variant.
  - `ClientHandle::upload_attachment(data) -> (BlobHash, u64)` wraps the blob store; `ClientHandle::send_attachment_message(...)` ties the upload hash + metadata + optional caption into a wire event. The hash is hex-encoded for the cross-crate boundary (willow-state can't depend on willow-network) — `blob_hash_to_hex` / `hex_to_blob_hash` round-trip helpers ship alongside.
  - `share_file_inline` marked `#[deprecated]` with a pointer to the new API; existing tests + reads keep working through the legacy path.

  **Tests landed:** `file_message_apply_attaches_file_attachment`, `message_apply_keeps_attachment_none`, `file_message_apply_rejects_oversize_dimensions` (state); `round_trip_via_hex`, `rejects_wrong_length`, `rejects_non_hex_chars` (client). End-to-end client tests (`upload_attachment_returns_hash`, `send_attachment_round_trips_through_state`) move to T2.5 once the receiver-side decode + projection lands. **Verify:** `cargo test -p willow-state file_message`, `cargo test -p willow-client --lib blob_hash_hex_tests`.

  > **Architectural blocker discovered while scoping.** `EventKind::Message` only carries `{ channel_id, body, reply_to }` — there is no `Content::File` plumbing through the event system today. The legacy `share_file_inline` works around this by encoding files into the `body` string as `[file:NAME:base64]`. To wire `Content::File` properly we need a new `EventKind::FileMessage { channel_id, hash, filename, mime_type, size_bytes, width, height, reply_to }` (or a generalised `EventKind::Message { content: MessagePayload }` if we want to avoid variant proliferation), plus the materialize handler, the projection in client views, and the listener / sync paths. This is roughly the same shape of work as adding any new EventKind (see `CLAUDE.md` §"Adding a new EventKind") and should be its own commit before T2 lands. T2's own contract — `upload_attachment` + `send_attachment_message` — stays small once the event variant exists.

### Phase C — inline rendering

- [x] **T3.** Create `crates/web/src/components/attachment/` module skeleton + `pick(mime_type, size_bytes) -> AttachmentKind`. Pure function, 6 unit tests covering image / file / voice / image-over-4mb-falls-to-card / unknown-mime-falls-to-card / case-insensitive mime / boundary inclusivity. Stub `AttachmentImage` / `AttachmentFileCard` / `AttachmentVoiceNote` components shipped alongside so the module's public surface compiles before T4–T6 land the full visuals. Wire into `components/mod.rs`. **Verify:** `cargo test -p willow-web --lib attachment`.

- [x] **T4.** Implement `<AttachmentFileCard>` per spec (mime icon, filename, size, download IconBtn, large-file badge above 10 MB). Spec layout shipped as a complete component; download IconBtn click handler logs a console warning until T8/T9 wire the blob-fetch path through `WebClientHandle::network.blobs()`. CSS for `.attachment` + `.attachment__*` variants appended to `style.css` using foundation tokens only. **Tests:** `format_size_thresholds`, `large_file_warning_threshold_matches_spec` (rust unit). Browser-tier render assertions land in T7.5 once we wire test fixtures with the new `attachment` field. **Verify:** `cargo test -p willow-web --lib attachment`.

- [x] **T5.** Implement `<AttachmentImage>` per spec (inline `<img loading="lazy">` wrapped in anchor with `target="_blank"`, caption row below in `--ink-3` mono, `max-width` 380 / 280 px). The `<img>` `src` is left empty pending the T8/T9 blob-fetch wiring; the spec layout + caption are final. **Tests:** `caption_format_matches_spec` pins the byte-exact `filename · size · e2e encrypted` format. **Verify:** `cargo test -p willow-web --lib attachment`.

- [ ] **T6.** Implement `<AttachmentVoiceNote>` per spec + `VoiceNotePlayer` single-instance coordinator. **Browser tests:** AG-5, AG-6. **Verify:** `just test-browser`.

- [x] **T7.** Wire the typed `EventKind::FileMessage` rendering branch in `message.rs` to call `attachment::pick()` and render the appropriate component. Projection updated: `DisplayMessage` gains `attachment: Option<willow_state::FileAttachment>` populated by `views::compute_messages_view` from the underlying `ChatMessage::attachment`. Legacy base64 `[file:…]` branch kept after the typed branch so historical messages still render. Reviewer-flagged fixes from T2 also landed in this commit:
  - `derive_client_events` now matches `EventKind::Message | EventKind::FileMessage` so attachment messages produce `ClientEvent::MessageReceived` and the UI signal layer fires.
  - `hex_to_blob_hash` re-exported from `willow_client::lib` (was effectively `pub(crate)` before via the private `actions` module). `#[allow(dead_code)]` removed.
  - `share_file_inline` callers (`crates/web/src/components/file_share.rs`, `crates/agent/src/tools.rs`, `crates/client/src/tests/actions.rs`) get `#![allow(deprecated)]` / `#[allow(deprecated)]` so `just check` stays zero-warning while the legacy reader path remains alive.

  **Tests:** state-tier projection tests cover the `attachment` field round-trip (T1+T2 commits). Browser-tier `message_row_renders_typed_content_file_*` tests are deferred to T4/T5 once the placeholder components ship their full visuals. **Verify:** `cargo check --workspace --all-targets`, `cargo test --workspace --lib`.

### Phase D — upload dialog

- [ ] **T8.** Implement `UploadQueue` reactive state + `<UploadDialog>` modal sheet (picker row, per-file rows with progress + cancel, footer actions). Wire from a context provided in `chat_view`. **Browser test:** AG-7. **Verify:** `just test-browser`.

- [ ] **T9.** Wire the composer's attach button to open `<UploadDialog>`. Remove the `<FileShareButton>` callsite from `chat_view` / `mobile_shell`. Mark `file_share.rs::FileShareButton` as `#[deprecated]`. **Browser test:** `composer_attach_button_opens_upload_dialog`. **Verify:** `just test-browser`.

### Phase E — drag-and-drop

- [ ] **T10.** Implement `<DragOverlay>` + `attach_drag_and_drop_listeners` page-level handler. Mount overlay at app root with shared `(open, set_open)` context. **Browser test:** AG-8. **Verify:** `just test-browser`.

- [ ] **T11.** Wire drop event → enqueue files into `UploadQueue` → open `<UploadDialog>`. **Playwright spec:** AG-9 (`e2e/files-inline.spec.ts`). **Verify:** `just test-e2e-ui` for the new spec only.

### Phase F — paste-to-upload

- [ ] **T12.** Implement `attach_paste_handler` on the composer textarea. Route clipboard files / images to `UploadQueue`. Pasted images get filename `pasted-{YYYY-MM-DD-HH-mm-ss}.png`. **Browser test:** AG-10. **Verify:** `just test-browser`.

### Phase G — accessibility, motion, polish

- [x] **T13.** Audit ARIA labels per the spec table for the surfaces that ship in this PR:
  - Composer attach button (`<FileShareButton>`): `aria-label="attach file"` (was just a hover `title`).
  - File card download button (`<AttachmentFileCard>`): `aria-label="download {filename}"` ✓ shipped in T4.
  - File card download button when network is offline: `aria-disabled="true"` + `disabled` ✓ shipped with the fetch wiring.

  Out-of-scope ARIA labels owned by surfaces deferred to T6 / T8 / T10:
  - voice note play / pause — `<AttachmentVoiceNote>` placeholder; T6.
  - upload cancel (per file / all) + confirm — `<UploadDialog>`; T8.
  - drag overlay — `<DragOverlay>`; T10.

  **Browser test:** AG-11 lands alongside the deferred surfaces so a single browser-tier audit can sweep them all together. Today the FileShareButton + FileCard ARIA is verified by code inspection against the spec table. **Verify:** `cargo check --workspace --all-targets`.

- [x] **T14.** Verify reduced-motion participation for the surfaces that ship in this PR. By construction:
  - `<AttachmentImage>` paints a static `<img>` with `loading="lazy"` and `decoding="async"`. No CSS transitions, no keyframes — reduced-motion is a no-op (no animation to suppress).
  - `<AttachmentFileCard>` paints a static card. The download button has `:hover` / `:focus-visible` border-color changes but no transitions on the property itself.
  - The drag overlay (`<DragOverlay>`, T10) and upload-progress bars (`<UploadDialog>`, T8) are the surfaces the spec calls out for reduced-motion gating; both are deferred. Their AG-12 browser test lands with their respective tasks.

  No code change needed for the surfaces that ship today. **Verify:** code inspection of `crates/web/style.css` `.attachment*` rules — no `transition` / `animation` properties present.

### Phase H — close-out

- [ ] **T15.** Tick the spec acceptance criteria in `docs/specs/2026-04-19-ui-design/files-inline.md` for the items that ship in this PR (file-card render, image render, image > 4 MB degradation, file > 10 MB warning, ARIA `download {filename}`). Stamp spec status `draft → implementing` with a note pointing back at this plan + PR. The spec's "Open questions" (`width` / `height` schema extension, 25 MB max-file size) get resolved-with-pointers comments. The status moves `implementing → landed` in a follow-up close-out commit on main once T6 / T8 / T9 / T10 / T11 / T12 ship — they're necessary for the full spec checkbox sweep. **Verify:** spec + plan diff inspected; `just check` clean on the close-out commit.

## Ambiguity decisions

These are decisions made in the plan that the spec leaves open or under-specified. Recorded so future readers know why we chose what we chose.

1. **Voice-note recording.** The spec discusses playback but never recording. We treat capture as the responsibility of a future surface (likely tied to the call experience or a dedicated "press-and-hold-to-record" affordance) and ship phase 3b with playback only. If a recording UI lands later, it produces standard audio attachments that this phase already renders.
2. **Waveform visual.** The spec calls for a styled bar-strip waveform. Computing peaks for arbitrary audio in WASM is non-trivial and a chunky asset. v1 uses the browser's native `<audio controls>` chrome inside the card and reserves the bar-strip for a follow-up. The IconBtn + mm:ss / mm:ss timer the spec calls out are still wired, the player is single-instance, and the card layout uses the spec's container styles — only the visual peaks are deferred.
3. **Image lightbox.** Anchor-wrap with `target="_blank"` opens full-size in a new tab. A true in-app lightbox modal would need its own focus / keyboard contract and is out of scope here.
4. **Mobile drag-and-drop.** Touchscreens don't generate `DataTransfer`; mobile attach goes through the dialog. The drag overlay is desktop-only behaviourally even though the CSS would render on mobile (it never opens because no drag events fire).
5. **Upload chunking UI.** iroh blob transport handles chunking transparently. The dialog shows one progress bar per file driven by the blob store's outgoing-bytes signal; chunk count is intentionally not surfaced.
6. **Legacy `[file:…]` base64 messages.** Old messages keep rendering through the existing `parse_inline_file` reader so peers don't see broken cards on history sync. The legacy *writer* path is removed (composer no longer emits it) and the reader logs a debug-only deprecation warning so we can prune the path once real usage drops.
7. **Image dimensions extraction.** Browser-side via `Image` element measurement. The web layer pulls width / height before calling `send_attachment_message` so the wire payload always carries them when known. Files attached without dimensions (e.g. drag-and-dropped non-images) carry `None`, which is fine — the renderer falls back to natural sizing.

## Test plan

- **State tests:** `content_file_round_trips_with_dimensions`, `content_file_round_trips_without_dimensions_back_compat`, `content_validate_rejects_oversize_dimensions` (in `willow-messaging`'s wire-tests if that's where round-trips live, else `willow-state`).
- **Client tests:** `upload_attachment_returns_hash`, `upload_attachment_dedupes_by_hash`, `send_attachment_round_trips_through_state`, `send_attachment_carries_dimensions` (in `crates/client/src/tests/files.rs`).
- **Web unit tests:** `attachment::pick` decision-table (5 cases) inside `attachment/mod.rs`.
- **Browser tests:** AG-2 through AG-8, AG-10 through AG-12 plus the message-row wiring tests (~14 tests in `mod phase_3b_attachments`).
- **Playwright:** AG-9 — one spec covering real desktop drag-and-drop end-to-end. wasm-pack can't fake `DataTransfer` reliably so this single behaviour stays at the e2e tier.

## Verification commands

Subagents must run the targeted subset for their task; the final task runs the full set:

```bash
just test-state                       # T1
just test-client                      # T2
cargo test -p willow-web              # T3 (attachment::pick unit tests)
just test-browser                     # T4–T10, T12–T14 (each task runs the suite or a filter)
just test-e2e-ui                      # T11 (one new spec)
just check                            # T15 close-out (full sweep)
just test-browser && just test-e2e-ui # T15 close-out (browser + Playwright together)
```

## Out of scope (explicitly deferred)

- Voice-note recording (mic capture + encoding) → future phase, likely tied to call experience or a dedicated record-button affordance.
- Artistic waveform peaks → v1 uses browser-native `<audio controls>`; styled bar-strip is a follow-up.
- True in-app image lightbox → anchor-wrap to a new tab is enough for v1.
- Mobile drag-and-drop → touchscreens lack `DataTransfer`.
- Resumable / chunked upload UI → blob transport handles chunking transparently.
- Server-side file-type policies → governance concern, deferred until `governance.md` lands.

## Status ladder transitions

- Spec `files-inline.md` moves `draft` → `implementing` on T1 (or T0 if the human reviewer prefers a separate spec-status bump commit).
- Spec moves `implementing` → `landed` on PR merge (commit hash recorded in spec front-matter).
- This plan moves `draft` → `landed` on PR merge.
