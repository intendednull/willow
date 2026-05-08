# Files and inline attachments — file cards, images, voice notes, upload

**Parent:** [README.md](README.md)
**Status:** implementing (phase 3b PR #633 — `docs/plans/2026-05-08-ui-phase-3b-files-inline.md`)
**Dependencies:** [`foundation.md`](foundation.md),
[`layout-primitives.md`](layout-primitives.md),
[`message-row.md`](message-row.md),
[`composer.md`](composer.md)

## Purpose

Attachments enter the conversation through the composer's attach button
(as well as drag-and-drop and paste on desktop). They render as inline
artefacts inside a message row. This spec owns the visual / behavioural
surface for every attachment form (file card, image, voice note), the
upload dialog invoked from the compose surface, and the drag-and-drop /
paste-to-upload inputs. Row positioning inside the message body is
owned by [`message-row.md`](message-row.md).

## Scope

**In scope.** File attachment card (filename, size, mime icon,
download, large-file warning). Inline image attachment (lazy reveal,
tap-to-enlarge, alt text, size-based degradation). Voice note
attachment (waveform + playback). Upload dialog invoked from the
compose surface's attach button (picker, progress, cancel). File
constraints (max size, allowed types, chunking). Inline rendering
rules (when an image becomes a thumb vs a full card). Desktop drag-
and-drop zones and paste-to-upload.

**Out of scope (hand-off).** Row layout and positioning within the
body column → [`message-row.md`](message-row.md). Compose surface, its
attach *button placement* (position in the compose row), and the
composer keybinding flow → [`composer.md`](composer.md). Ephemeral
expiration of attachments → [`ephemeral-channels.md`](ephemeral-channels.md).

## Inline attachments — rendering

Each is a direct child of the message row's content column, preceded
by `margin-top: 8px`.

### Image

- **Desktop.** `max-width: 380px`, radius 10 px, `loading="lazy"`.
- **Mobile.** `max-width: 280px`, radius 10 px, `loading="lazy"`.
- Mono caption below: `filename · size · e2e encrypted` in `--ink-3`.
- `<img loading="lazy" decoding="async">` wrapped in an anchor that
  opens full-size in a new tab.
- Failed loads fall back to the file-card variant.
- Tap-to-enlarge on mobile opens the same full-size view.

### File card

- **Container.** `FileCard` on `--bg-2`, `--line` border, radius 10 px,
  `10px 12px` padding. `max-width: 420px` desktop. On mobile, file
  card `max-width: 100%` within the content column.
- **Contents.** Mime icon (24 × 24, `--ink-2`), filename (body L,
  `--ink-0`, truncated), size + mime hint (`--ink-3`), `download`
  IconBtn.
- **Large-file warning.** Files above 10 MB get a
  `large · downloads on click` warning badge in `--amber`.

### Voice note

- Inline-flex card on `--bg-2`, `--line` border, radius 10 px, same
  `10px 12px` padding as the file card, `max-width: 420px` desktop.
- Contents: play / pause IconBtn (32 × 32, `--moss-2` foreground),
  waveform strip (fixed height 24 px, `--moss-1` bars on `--bg-1`;
  progress fill `--moss-2`), elapsed / total timer
  (`font-mono`, 11 px, `--ink-3`, format `mm:ss / mm:ss`).
- Playback persists across other audio (one voice note at a time;
  starting another pauses the previous).

## Inline rendering rules

- **Image above 4 MB** degrades to a file card instead of inline, to
  avoid silently spending mobile bandwidth.
- **File above 10 MB** shows the `large · downloads on click` warning
  badge.
- Images are always inline unless they hit the 4 MB rule or their
  mime type is unknown; unknown mime → file card.
- Voice notes always render as the voice-note card (never a plain
  file card), regardless of size.

## Upload dialog

Invoked from the compose surface's attach IconBtn (`plus`); composer
placement lives in [`composer.md`](composer.md). Also used as the
destination for drag-and-drop and paste-to-upload.

- Modal sheet above the composer. `--bg-1` on `--line`, radius 12 px,
  `--shadow-2`.
- **Picker row.** Browse button (`--moss-1` border, `--ink-0` text,
  label `choose files`) + inline hint `or drop files here`. Dropping
  onto the picker row highlights with `--moss-2` border.
- **Per-file row while uploading.** Icon + filename + size + progress
  bar (`--moss-2` fill on `--bg-2` track, 4 px tall) + cancel IconBtn
  (`x`). Complete rows flip to a `check` glyph.
- **Footer actions.** `cancel all` (`--ink-2`), flex spacer,
  `attach to message` (disabled until at least one upload completes;
  `--moss-1` bg, `--moss-4` fg).
- Attachments queue into the composer's pending attachment list on
  confirm; they are sent as part of the next outgoing message (message
  send and queueing owned by [`composer.md`](composer.md)).

## Drag-and-drop (desktop)

- A page-level drop listener displays a full-viewport overlay when
  a drag enters the window: `color-mix(in oklab, var(--moss-2) 18%, transparent)`
  tint + centered dashed `--moss-2` panel containing a 32 px `upload`
  icon and the label `drop to attach`.
- Dropping anywhere in the window opens the upload dialog with the
  dropped files already enqueued.
- Drag-leave / drop clears the overlay.

## Paste-to-upload (desktop)

- When the composer textarea has focus and a paste event contains
  files or images, the files enter the upload dialog (same queue as
  drag-and-drop) instead of inserting text.
- Pasting a plain image from the clipboard names the file
  `pasted-{YYYY-MM-DD-HH-mm-ss}.png` before it enters the queue.

## File constraints

- **Max size per file.** 25 MB in v1 (see Open questions).
- **Allowed types.** Any mime type is accepted; only images with mime
  prefix `image/` render inline (subject to the 4 MB rule).
- **Chunking.** Files chunk on the wire per the networking crate's
  blob protocol; the UI shows a single progress bar per file. Chunk
  count is not surfaced in the dialog.

## Copy (exact strings)

Lowercase unless proper noun. This is the source of truth for
translation work for the strings owned here.

### File card + image caption
- Image caption: `filename · size · e2e encrypted`
- Large-file warning badge: `large · downloads on click`

### Upload dialog
- Browse button: `choose files`
- Drop hint: `or drop files here`
- Footer cancel: `cancel all`
- Footer confirm: `attach to message`

### Drag overlay
- Label: `drop to attach`

## Data dependencies

### Existing

File attachments travel through the existing blob transport; the
message envelope carries an attachment list (filename, size, mime,
blob hash).

### Fields requiring extension

- **Image dimensions (`width`, `height`).** To render a correct-ratio
  placeholder while bytes stream, the attachment metadata needs
  `width` and `height`. Propose extending the messaging schema.

## Edge cases

- **Huge attachment.** Files > 10 MB get the `large · downloads on
  click` warning badge. Images > 4 MB render as file cards instead of
  inline, to avoid silently spending mobile bandwidth.
- **Mime mismatch.** Filename extension and mime disagree → show the
  mime icon (server of truth), keep the filename as-is.
- **Zero-byte file.** Upload is rejected before send with
  `empty file — nothing to attach` (not a sent message; dialog-local).
- **Clipboard with both text and image.** Image wins and routes to the
  upload dialog; text paste is dropped (user can paste again after
  dismissing the dialog).

## Accessibility

### ARIA labels

| Element | Label |
|---|---|
| attach button | `attach file` (owned by [`composer.md`](composer.md)) |
| file download | `download {filename}` |
| voice note play | `play voice note` / `pause voice note` |
| upload cancel (per file) | `cancel upload of {filename}` |
| upload cancel (all) | `cancel all uploads` |
| upload confirm | `attach to message` |
| drag overlay | `drop to attach` |

### Keyboard path

- From the compose surface, `Tab` reaches the attach button; `Enter`
  opens the upload dialog.
- Inside the dialog: `Tab` cycles through rows and footer actions;
  `Escape` calls `cancel all`; `Enter` triggers the focused button.
- Voice note cards: `Space` toggles play / pause when the card is
  focused.

### Motion

Progress bars update linearly; reduced motion shows the bar without
easing. Drag overlay crossfades in under reduced motion.

## Acceptance criteria

- [x] Inline images render at `max-width: 380 / 280 px` with
      `loading="lazy"` and caption `filename · size · e2e encrypted`.
      *(`<AttachmentImage>` — phase 3b T5.)*
- [x] Images above 4 MB degrade to a file card; files above 10 MB
      show the `large · downloads on click` warning badge.
      *(`attachment::pick` decision table + `<AttachmentFileCard>` —
      phase 3b T3 + T4.)*
- [x] File cards render with mime icon, filename, size, download
      IconBtn, and respect `max-width: 420px` desktop / `100%` mobile.
      *(`<AttachmentFileCard>` — phase 3b T4.)*
- [ ] Voice notes render the waveform + play / pause + mm:ss timer
      card, and starting one pauses any other. *(Placeholder ships in
      phase 3b; full surface in T6.)*
- [ ] Upload dialog opens from the composer attach button with a
      picker row, per-file progress + cancel, and the footer actions
      in §Copy. *(Phase 3b ships a single-file direct upload via the
      paperclip; full dialog is T8.)*
- [ ] Drag-and-drop anywhere in the desktop window opens the upload
      dialog with dropped files enqueued; the overlay uses the copy
      in §Copy. *(T10.)*
- [ ] Pasting files or an image into the composer routes them to the
      upload dialog instead of inserting text. *(T12.)*
- [ ] Every interactive element has an ARIA label per §Accessibility.
      *(`download {filename}` + `attach file` shipped in phase 3b;
      voice-note + upload-cancel + drag-overlay labels land with
      T6 / T8 / T10.)*

## Open questions

- ~~**Image dimensions in the envelope.**~~ **Resolved (phase 3b T1).**
  `Content::File` gained optional `width: Option<u32>` /
  `height: Option<u32>` (`#[serde(default)]`, capped at
  `MAX_DIMENSION_PX = 16384` with a render-clamp warning). The web
  layer extracts dimensions via the browser `Image` API at upload
  time and stamps them onto the wire; receivers use them to reserve
  correct-aspect layout space while bytes stream.
- **Max file size.** 25 MB in v1 (enforced in `<FileShareButton>`).
  The blob transport itself can handle larger payloads — the cap is
  a protect-from-accidents guard until the upload dialog (T8) lands
  with real progress UI. Revisit once we see real usage and
  blob-transport cost.
