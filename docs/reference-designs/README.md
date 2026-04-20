# Reference designs

Archived design bundles from Claude Design (claude.ai/design) used as inputs for
UI specs and plans. Each bundle is a gzipped tar archive containing the original
HTML/CSS/JSX prototype, a `README.md` from the design tool, and any chat
transcripts.

Keep bundles immutable — never edit in place. If the design evolves, save a new
dated bundle alongside the old one so historical intent is preserved.

## Layout inside each bundle

```
willow/
├── README.md               handoff notes from the design tool
├── chats/                  conversation transcripts (intent lives here)
├── project/
│   ├── ARCHITECTURE.md     design-side file organization rules
│   ├── Willow.html         desktop prototype entry
│   ├── Willow Mobile.html  mobile prototype entry
│   ├── Willow Review.html  side-by-side review canvas
│   ├── src/                desktop JSX
│   │   ├── shared/         cross-surface atoms (security-critical)
│   │   └── mobile/         mobile JSX + iOS/Android frames
│   ├── scraps/             informal sketches
│   └── uploads/            user-supplied images
```

## Inventory

| File | Date | Source | Notes |
|------|------|--------|-------|
| `2026-04-19-willow-design-bundle.tar.gz` | 2026-04-19 | Claude Design handoff `zzFiuoEoUKLDFdbOOJhyuA` | First mobile-first pass. Drives `docs/specs/2026-04-19-ui-design/`. |

## Extracting a bundle

```bash
mkdir -p /tmp/willow-design && \
  tar -xzf docs/reference-designs/2026-04-19-willow-design-bundle.tar.gz -C /tmp/willow-design
```

## Adding a new snapshot

1. Download the bundle (gzipped tar) from the design tool.
2. Save as `YYYY-MM-DD-<topic>-design-bundle.tar.gz`.
3. Append a row to the inventory above with source reference and one-line note on what changed.
4. If the snapshot drives new specs, link the spec folder in the inventory note.
