# Prior-Art Coverage Survey — Willow Specs
**Date:** 2026-05-29
**Scope:** All 29 design specs in `docs/specs/`
**Method:** Per-spec survey of design decisions and external lineage, followed by adversarial citation fact-checking (web-verified identifiers + code-verified Willow-side claims)

---

## Summary

Willow's prior-art convention is a `## Prior Art` table — `| System | Key idea adopted / how Willow diverges |` — tying each design decision to the external work it draws from or rejects. Before this survey only **1 of 29 specs** (the per-author Merkle-DAG spec) carried one.

This survey scored every spec, identified **17 strong candidates** whose decisions have rich, citable external lineage, and added a verified `## Prior Art` section to each. The remaining 12 are process/testing/docs or purely-internal specs with no meaningful external prior art (listed below).

After drafting, all 17 sections went through an adversarial review: **148 external citations** web-verified (RFC/NIP/MSC/BIP/BEP/XEP/CIP numbers, paper authors/venues/years, library facts) and **188 Willow-side claims** checked against the worktree code. Findings — 5 major, 24 minor, 4 nit — were triaged; every issue *inside an added Prior Art section* was fixed before merge (see "Review outcome").

## Specs that received a Prior Art section

| Spec | Anchor topic of the prior art |
|---|---|
| `2026-03-24-async-client-ui-refactor-design.md` | TEA / Redux-Flux / CQRS / actor-handle / SolidJS signals |
| `2026-03-26-screen-sharing-call-page-design.md` | WebRTC, perfect negotiation, full-mesh vs SFU (Jitsi/LiveKit/Element Call) |
| `2026-03-27-shareable-join-links-design.md` | matrix.to locators, NIP-21/19, Magic Wormhole, capability URLs |
| `2026-03-27-worker-nodes-design.md` | Nostr relays, SSB pubs, IPFS pinning, Dynamo anti-entropy |
| `2026-03-29-agentic-peer-api-design.md` | MCP, JSON-RPC, Matrix appservices, OAuth scopes, least privilege |
| `2026-03-29-iroh-migration-design.md` | iroh/n0, libp2p, HyParView+Plumtree, QUIC, BLAKE3, pkarr |
| `2026-03-31-actor-system-library-design.md` | Actor model (Hewitt 1973), Erlang/OTP, actix/ractor/kameo/xtra |
| `2026-04-12-state-authority-and-mutations.md` | Object-capabilities, Macaroons, SPKI/SDSI, Certificate Transparency |
| `2026-04-24-bech32-identifiers.md` | BIP-173/350, NIP-19, Base58Check, multiformats, StrKey, CIP-19 |
| `2026-04-24-epoch-key-rotation.md` | MLS/TreeKEM, HPKE, HKDF, Double Ratchet, Sender Keys, Megolm |
| `2026-04-24-error-prefixes.md` | NIP-01/42, Matrix errors, gRPC status, HTTP 429, WebSocket close codes |
| `2026-04-24-history-sync-eose.md` | Nostr EOSE, Matrix /sync, WebDAV sync-token, IMAP CONDSTORE, git fetch |
| `2026-04-24-negentropy-sync.md` | Negentropy/RBSR, SSB, Automerge, git pack, version-vector anti-entropy |
| `2026-04-24-outbox-relay-discovery.md` | NIP-65, pkarr/BEP44, Matrix well-known, RFC 8615, did:plc |
| `2026-04-24-relay-capability-doc.md` | NIP-11, Matrix versions/capabilities, XEP-0115/0390, NodeInfo, OIDC discovery |
| `2026-04-25-llm-agent-ux-spec-design.md` | Discord/Slack/Matrix bots, MCP, SSE streaming, protobuf/Cap'n Proto evolution |
| `2026-04-26-state-management-model-design.md` | Actor model, CSP/Hoare, Erlang gen_server, Akka, Ryhl "Actors with Tokio" |

## Not candidates (no Prior Art section added)

Process, testing, docs-infrastructure, and purely-internal specs with no external lineage worth citing:

- `2026-03-24-multi-peer-e2e-tests-design.md` — internal test harness design.
- `2026-03-25-ux-navigation-improvements-design.md` — product UX, no external protocol lineage.
- `2026-04-01-per-author-merkle-dag-state-design.md` — **already has** a Prior Art section (the convention's origin).
- `2026-04-12-willow-channel-removal.md` — internal refactor (crate deletion + type consolidation).
- `2026-04-13-test-architecture.md` — internal test-tier policy.
- `2026-04-21-e2e-test-architecture-design.md` — internal test-tier policy.
- `2026-04-27-event-based-waits-design.md` — internal async-test helper.
- `2026-05-07-docs-organization-design.md` — this docs system itself.
- `2026-05-21-pinned-message-metadata-design.md` — small internal metadata addition.

## Review outcome

The adversarial pass confirmed the citations are overwhelmingly accurate. Issues found and resolved in the added sections:

**Major (4 in-scope, all fixed):**
- `shareable-join-links`: fabricated type name `JoinLinkPayload` → corrected to the real `JoinToken` (`crates/client/src/ops.rs:28`).
- `iroh-migration`: overclaimed "no Willow-invented abstraction; uses iroh's `Hash`" → corrected to note the `BlobHash` wrapper (`crates/network/src/traits.rs:14-20`, needed because `iroh_blobs` is not WASM-compatible) and that `Bytes` is `bytes::Bytes`.
- `negentropy-sync`: wrong code ref `dag.rs:146-158` → `dag.rs:197-218` (the actual seq + prev-hash checks); matching body refs corrected for consistency.
- `relay-capability-doc`: false "v1.10" attribution for Matrix's `/versions` vs `/capabilities` split (both endpoints exist since r0.5.0, ~2019) → corrected in both the table row and the Future-work bullet (the latter was a pre-existing error in the spec body).

**Minor / nit (in-scope, fixed):** Discord expiry floor (30 min, not 1h); SSB pubs-vs-rooms conflation; Meyer RBSR date (arXiv 2212.13567, Dec 2022 / SRDS 2023); Akka "Lightbend" company-vs-framework wording; kameo "supervision" qualified (Willow keeps single-actor `RestartPolicy`); MLS forward-secrecy ("full FS", reserving "weak FS" for Willow); TreeKEM "key schedule" → "ratchet tree" (RFC 9420 §7); SSB Pubs "superseded" → "complemented" by Rooms; CIP-19 HRPs defined in CIP-5; SSB-EBT authorship disambiguation.

**Out of scope (not fixed here):** The review also flagged ~14 stale `file:line` references in the **bodies** of `history-sync-eose`, `relay-capability-doc`, and `llm-agent-ux-spec` (code drifted since those specs were written). These predate this work and are tracked as a follow-up doc-hygiene pass, not folded into this prior-art change.
