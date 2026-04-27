# Willow App Runtime — master design

**Date:** 2026-04-27
**Status:** draft, exploratory — no timeline, iterations expected
**Branch:** `claude/wasm-plugin-system-WyY1p`

## Purpose

This folder describes a long-horizon architectural direction for Willow:
becoming a small **kernel** that hosts **typed, capability-mediated,
content-addressed P2P apps**, of which today's chat product is one.

It is a *destination*, not a description of current state. The existing
codebase ships a chat-specific peer with a fixed `ServerState`, a Leptos web
UI bound to chat semantics, and worker binaries (`replay`, `storage`) that
materialize chat state. The architecture below treats chat as one application
among many, the UI as one app among many, and workers as commodity peer hosts
that don't know what application they're hosting.

This is a master spec. Child specs in this directory will refine specific
sub-systems as we iterate (kernel boundaries, WIT interface design, capability
model, app SDK, distribution, chat-server migration, etc). This document is
deliberately light on implementation detail — focus is on **what we are
building and why it is the right shape**, not on which files change.

## Reframe: from "WASM plugin system" to "P2P app runtime"

The conversation that produced this spec started as "a WASM plugin system for
Willow." That framing is too small. A plugin system implies a host application
with a fixed feature set, optionally extensible. What we want is the inverse:
a small kernel where **the application itself is a composition of typed,
sandboxed, content-addressed components**.

Chat is not a feature of Willow that has plugins. Chat is one app running on
Willow. Wikis are another. So is a kanban board. So is whatever someone
builds in two years. The kernel does not know what any of them are.

That is not a plugin system. It is an **app runtime**.

## Core idea

Willow's kernel provides exactly what every P2P app needs and no more:

- **Identity & signatures.** Ed25519 keypairs. Private keys live only in the
  kernel; components describe events and the kernel signs.
- **Peer protocol.** iroh, gossip, blob fetch, topic membership.
- **Event/DAG primitives.** `Event { author, prev, deps, payload, sig }`,
  `EventDag<P>` generic over opaque payload bytes, `PendingBuffer`, sync
  summaries, HLC.
- **Component loader & capability arbiter.** Instantiates WASM components,
  brokers every inter-component call, enforces capability declarations.
- **Narrow native imports.** DOM, network egress, persistent storage —
  bound only to specific component classes that have the capability.

Everything else is a component. The chat semantics are a component. The UI is
a component. Themes, integrations, bridges, even a future "the server has
roles" feature — components.

A **peer** is a process running the kernel plus whatever components it has
chosen to instantiate based on what topics it has joined and what
participation modes it has elected. There is no client/server distinction in
the kernel. A laptop running a UI, a worker running headless, an MCP agent,
and a future native desktop client are all just "the kernel + a different mix
of components."

## Runtime profiles for components

Different components have fundamentally different needs. The kernel
distinguishes three profiles, with very different host imports and execution
policies:

| Profile | Determinism | Imports | Where it runs | Examples |
|---|---|---|---|---|
| **State / `apply`** | **Required** — bit-identical across peers | `host.log`, plus the **deterministic helper set**: signature verification, payload-MAC verification, content hashing, key installation from a sealed key-distribution payload, HLC extraction | Every peer materializing the topic | chat-server-state apply, wiki-state apply |
| **State / `propose`** | Not required (runs once, on the authoring peer) | `host.hlc`, `host.random`, `host.seal` (capability-gated), `host.log` | The peer that originates the event | chat-server-state propose |
| **Interaction** | Not required | `host.broadcast`, `host.subscribe`, `host.kv`, `host.user-prompt`, UI app's `ui:*` | Any peer with a UI / agent host | chat-server-interaction, wiki-interaction |
| **Behavior** | Not required | + `host.http`, `host.timer`, `host.identity` (own keypair, gated) | Designated peer(s) | bridges, automod, archivers, bots |

All four are loaded by the same kernel through the same WIT-typed interface.
The difference is *which host imports each profile is permitted to bind* and
*which fuel/time policy applies*. State components have two entry points:
**`apply`** runs everywhere, deterministically, with no non-deterministic
imports; **`propose`** runs only on the originating peer to construct an
event payload (it is allowed to consult the clock, generate randomness, and
seal content), after which the kernel signs and broadcasts. Determinism is
enforced on the `apply` path by the absence of any non-deterministic host
import — there is nothing to call.

## Apps as bundles of components

An **app** is the user-facing distribution unit. A bundle on iroh-blobs,
hash-pinned and signed by the author:

```
chat-server/                            (the bundle)
├── manifest.toml                       (version, hashes, capabilities, interfaces)
├── state.wasm                          (deterministic; required by any materializing peer)
├── interaction.wasm                    (typed view + commands; loaded if peer has a UI)
├── behavior-discord-bridge.wasm        (optional; loaded by peers offering this capability)
└── schema.wit                          (interface contract, used by tooling)
```

Apps can ship state-only (a pure semantics package), interaction-only
(an alternative UI for someone else's state app), state+interaction (the
common case), or any combination. A peer fetches the bundle by hash and
instantiates only the components it needs for what it intends to do.

## UI is an app

The Leptos web client is, in this model, **the default UI app**. It exports
a set of `ui:*` interfaces — `ui:panel`, `ui:list`, `ui:message`, `ui:form`,
`ui:menu`, etc. — that other apps' interaction components import.

The honest framing: a real UI on any platform requires a broad and unstable
capability surface (DOM + focus/IME, clipboard, file pickers, navigation,
viewport/media queries, push, IndexedDB, service workers, drag-and-drop on
web; the equivalent set on each native platform). The kernel does not try
to abstract that surface. **The default UI app is privileged to bind a
broad, browser-shaped capability surface**; it is shipped in-tree as one
app, but it is not architecturally identical to a third-party
interaction-only utility — its capability set is much larger, and it is
trusted by the user accordingly.

The `ui:*` WIT interfaces are therefore **an interaction contract for
app-to-UI integration**, not a portable UI substrate. They define how an
interaction component declares the views it wants rendered, the commands
it accepts, and the contextual integration points it offers — not how
those views are painted. Each UI app implements the contract in its own
idiom; reusing one set of interaction components across UIs is the goal,
but each UI app is a substantial standalone project, not a recompile.

Plausible UI apps over time:

- **`willow-ui-tui`** — terminal, ratatui rendering, the chat-shaped
  subset of `ui:*`.
- **`willow-ui-mcp`** — agent host, structured-data rendering for an LLM.
- **`willow-ui-mobile-native`** — Compose / SwiftUI shell, far-future.
- **`willow-ui-dioxus`** — once Dioxus Blitz is mature, candidate
  replacement for Leptos.

App authors target the WIT contract. Their interaction components work
against any UI app that exports the interfaces they import. UI apps that
do not export an interface (e.g. a TUI without `ui:rich-card`) cause
graceful degradation, not breakage.

## Inter-component composition

Components compose by importing each other's exposed interfaces, mediated by
the kernel:

- A translation utility component imports `ui:context-menu` and `chat:message`
  to add "translate this message" entries on chat surfaces. The kernel wires
  the imports if the user has granted the capability.
- An emoji-picker utility exports `emoji:pick`; chat-interaction imports it
  if the user has installed the picker.
- A theme-pack imports `ui:theme` and provides token overrides.

Cross-component calls always go through the kernel. The kernel is the
capability arbiter, the call broker, and the resource-handle resolver. There
is no direct memory-shared linkage between components; every interaction is
typed, bounded, and refusable.

## Determinism, in detail (for state-`apply`)

The deterministic constraint applies only to the `apply` entry point of a
state component — the function every peer runs against every event. The
`propose` entry point runs once on the originating peer and is intentionally
non-deterministic; its output is an event payload that the kernel then signs
and broadcasts, after which all peers (including the originator) replay
through `apply`.

State-`apply` is a pure function of its inputs *and the kernel-side
deterministic helper set*. The kernel passes the event author, the event
hash, the HLC encoded in the event, and the event payload. The component
mutates state held in its linear memory and optionally emits a snapshot.

The rule is **`apply` may import only host functions whose output is a
pure function of their inputs** — not "no imports." Useful kernel-side
helpers `apply` legitimately needs are deterministic by construction:

- `host.verify-signature(pubkey, msg, sig)` — Ed25519 verification.
- `host.verify-payload-mac(envelope, key-handle)` — authenticity check
  on a sealed payload without revealing plaintext, proving "some holder
  of the key bound to this handle sealed this." Note this proves *key
  possession*, not *author identity* — author identity comes from the
  outer Ed25519 signature on the event itself. The exact set of envelope
  formats the helper accepts (current `seal_content`, future MLS
  Welcome / Commit, etc.) is the crypto-and-key-custody child spec's
  responsibility; per the seal-gift-wrap deferral spec, MLS *application*
  messages do not flow through the DAG and are therefore not what
  `apply` verifies.
- `host.hash(bytes)` — blake3 / sha256.
- `host.install-key(handle, sealed-distribution-blob) -> ()` — register
  that a key-distribution blob exists for this app handle. The kernel
  records the (handle, blob) pair under the app's namespace on every
  peer; whether *this* peer can actually unwrap the blob with its own
  X25519 key is recorded **only in kernel-local custody, never visible
  to `apply`**. From `apply`'s point of view, the call is a pure
  recording of "this app declares handle H, gated by this distribution
  blob," with no return value to branch on. State-`apply` is therefore
  bit-identical across peers regardless of who can decrypt. The
  *interaction* profile asks the kernel separately (`host.can-open(handle)`
  or by attempting `host.open` and getting an error) whether this peer
  can use the key to read messages. There is no observable peer-local
  return on the `apply` path.
- `host.now-hlc-from-event(event)` — extract HLC bytes from the event
  envelope (no wall clock).

What `apply` continues to be denied:

- No wall clock. No randomness. No network, no filesystem, no environment.
- No threads.
- A deterministic fuel budget (instruction count, not wall time). Running
  out of fuel terminates uniformly across peers.
- Spec-deterministic floats (the WASM spec pins these), with a strong
  recommendation to ban them in v1 anyway to avoid review pain.

The determinism proof is therefore: **every host import bound to `apply`
returns a pure function of its inputs given the event payload alone**.
There are no peer-local return values; whether-this-peer-can-decrypt is
an interaction-profile concern, not an `apply` concern. The exact list
of deterministic helpers belongs in the crypto-and-key-custody child
spec; the master spec commits to the shape.

The kernel verifies cross-peer convergence by hashing a **canonical
state digest** the app exports — *not* a hash of WASM linear memory,
which would diverge trivially across peers due to allocator behavior,
struct field padding, or `HashMap` iteration order. Apps export a
`state-digest()` function (or equivalent) that returns canonical bytes
under a deterministic encoding (postcard with sorted collections is the
existing-codebase precedent); the kernel hashes the result and gossips
the hash. The exact encoding rules belong in the
determinism-enforcement child spec; the master commitment is that
convergence is checked against an app-canonical digest, not memory
bytes. Mismatches surface as bugs (or, if signed, as proof of a
malicious or buggy component).

## Crypto and key custody

Encryption is load-bearing today (`willow-crypto`, `seal_content`,
the epoch-rotation spec, the deferred MLS-over-Willow spec) and the runtime
has to place it explicitly, not silently. The chosen split:

- **Private signing keys live only in the kernel.** No component sees them.
  Components describe events; the kernel signs.
- **Symmetric channel/group keys, ratchets, and MLS group state are kernel-custodied as well**, but on behalf of an app instance. Apps refer to them by app-declared key handles (opaque IDs).
- **The kernel exposes typed crypto host imports** bound to key handles:
  - `host.seal(handle, plaintext)` on state-`propose` and behavior
    profiles only — produces ciphertext under the named key.
  - `host.open(handle, ciphertext)` on interaction profile only —
    decrypts for display.
  - `host.verify-payload-mac(envelope, key-handle)` and
    `host.install-key(handle, sealed-distribution-blob) -> ()` on
    state-`apply` — deterministic helpers (see "Determinism, in detail").
    State-`apply` never sees plaintext message content, never sees a
    return value indicating local decryption capability; it records that
    the handle exists and lets the kernel custody the per-peer
    decryptability privately. Whether *this* peer can actually use a
    handle is a separate interaction-profile query.
- **Key generation and rotation events are app-defined.** A chat-server-style
  app defines its own `RotateChannelKeyV2`-equivalent events; the state-`apply`
  function records the new key handle in materialized state; the kernel
  binds the new handle to the underlying key material it just generated on
  behalf of the propose call. The kernel does *not* know what
  "channel" or "epoch" mean; it only knows about handles and the
  permissions to seal/open under them.
- **MLS group state**, when we adopt MLS, lives on the kernel side of the
  boundary as a typed capability surface (`host.mls`) bound to an app's
  group handle. The app emits MLS Welcome / Commit / Application events
  through ordinary state propose; the kernel-side MLS engine processes
  them under the requesting peer's identity.

The principle is consistent: **secrets do not enter component memory in
their raw form**. Components hold handles; the kernel custodies bytes. An
app-defined permission that gates `Rotate*` events is enforced by the
app's pre-check function (which shares its decision logic with apply, see
the capability model section); the kernel only enforces that the seal/open
call presented an authorized handle.

The exact `host.seal` / `host.open` / `host.mls` interface, key-derivation
strategy, and persistence story belong in a child spec dedicated to crypto
boundaries. What this section commits to is the placement: **encryption is
a kernel capability bound to opaque key handles**, not an app concern.

## Capability model

Every component runs sandboxed by default. It can:

- Make outbound calls only to interfaces its manifest declares as imports.
- Receive inbound calls only on interfaces its manifest declares as exports.
- Use host imports only as listed in the manifest's `capabilities` block.

The user is the trust root for their own peer. Installing an app prompts a
capability summary the kernel can render: "this app wants to broadcast
events on topic X, store ≤ 1 MB locally, send HTTP requests to discord.com."
Granted capabilities are bound at instantiate time; they cannot escalate
later without re-prompting.

**`ui:*` calls that proxy privileged platform surfaces are
capability-checked per call, not just per import-binding.** Clipboard
writes, file pickers, top-level navigation, push-notification
registration, and similar — each call is gated by the *calling
component's* manifest, not the UI app's broad surface. This prevents a
malicious or compromised interaction component composed inside the UI
app from socially-engineering the UI into doing things the calling
component was never granted. The UI app is in the TCB for its own
chrome and its own DOM; it is not in the TCB for arbitrary callers'
intents.

State-`apply` is bound only to the deterministic helper set
(see "Determinism, in detail" for the full list). There is no
non-deterministic capability to grant and no information leak surface;
resource consumption — handle namespace, key-store size, fuel — is bounded
by per-instance caps defined in the worker child spec. State-`propose`
has the small set listed in the runtime-profiles table (`host.hlc`,
`host.random`, capability-gated `host.seal`); these are bound only when a
peer is actually originating an event, never during replay.

## What stays the same about Willow

- Event-sourced per-author Merkle DAG with prev/deps causal links.
- Identity rooted in Ed25519 signatures.
- iroh for transport (gossip + blob fetch).
- Relays remain dumb topic-bridges; they do not materialize state.
- Workers (`replay`, `storage`) remain peers, just generalized to host
  arbitrary state components instead of being chat-specific.
- The dual-target (native + WASM) compilation discipline is *intended* to
  survive at the *kernel* layer — the kernel compiles to both targets, the
  native build using wasmtime and the web build using a jco-transpiled
  host. Concrete kernel subsystems that have historically been native-only
  (the MLS engine when adopted, persistent key storage, full-fat blob
  store) may require platform-specific backends behind a stable
  kernel-internal trait; cataloguing those backends and confirming each
  one survives jco transpilation is part of the crypto-and-key-custody
  child spec. For *application code*, the discipline is replaced: an app
  component is built once to wasm and is loaded by whichever kernel a
  peer is running.
- The existing capability/permission ideas from `willow-state` generalize,
  with one new responsibility: each app defines its own permission set, but
  also supplies the *pre-check* code that gates event creation. Today's
  centralized `required_permission()` table runs in trusted in-process Rust;
  under the runtime the kernel calls into the app's state component to ask
  "may this author emit this event under the current state?" before signing.
  This shifts a precise, audit-friendly responsibility onto app authors,
  but the runtime makes drift impossible by construction: **pre-check is
  not "shared logic by convention" — it is mechanically the same WASM
  function as `apply`'s authority verdict, called by the kernel in
  dry-run mode against a hypothetical post-state.** Apps export one
  authority predicate; the kernel calls it once before signing on the
  originator (with the proposed event applied to a scratch copy of state)
  and again on every peer during real `apply`. Compare-acceptance is
  enforced because it is the same export. Pre-check therefore runs under
  the state-`apply` runtime profile — same deterministic helper set, same
  fuel posture, same denied non-deterministic imports. The exact dry-run
  protocol (scratch state ownership, rollback semantics) is deferred to
  the chat-server-migration / WIT-interfaces child spec; the master spec
  commits to the *property* that pre-check and apply cannot diverge
  because they are not separate code paths.

## Runtime and actors

Willow's existing actor framework (`willow-actor`) and the
`docs/specs/2026-04-26-state-management-model-design.md` discipline — all
shared mutable state in lib crates lives inside an actor — do not go away.
The runtime sits *underneath* that model, not in place of it.

The intended mapping:

- **On any one peer, each component instance is owned by exactly one
  actor.** The actor's mailbox serializes calls into the component's WASM
  instance. Component instances are the unit of *typed sandboxing*; actors
  remain the unit of *concurrency*. Different peers materializing the same
  topic each instantiate the same component code in their own actor; the
  runtime makes no claim about cross-peer actor topology — that is
  emergent from the gossip protocol, not coordinated by the kernel.
- The kernel itself is composed of actors: a loader actor, a per-topic
  state-materialization actor (which owns one state component instance and
  calls `apply` on each event), interaction actors per active interaction
  component, behavior actors per behavior instance.
- Lock-vs-actor decisions in *kernel code* still follow the existing
  decision tree. Components never see locks; they see only the actor's
  mailbox semantics, surfaced as synchronous WIT calls into and out of the
  instance.
- Persistence is owned by the host's actors, not by components. A state
  component returns updated state in its linear memory; the kernel-side
  materialization actor decides when to snapshot, when to write to the
  storage backend, and how to coordinate with sync.

This means: the actor framework is one of the things that stays. The
runtime adds a layer above it for typed sandboxing, content-addressed
distribution, and capability arbitration. It does not replace the
host-side concurrency model.

## What changes about Willow

These are *consequences* of the design, named at the level of
responsibility rather than file layout. Exact crate boundaries, names,
and migration mechanics are child-spec concerns.

- **`willow-state` splits.** A payload-agnostic kernel half (events,
  DAG, sync primitives, HLC) stays as kernel. The chat-specific half
  (`EventKind`, `ServerState`, `apply_event`, `required_permission`)
  becomes the `chat-server` app.
- **The web client becomes the default UI app.** Its bindings to chat
  semantics route through the kernel and the chat-server interaction
  component rather than through direct Rust imports of chat types.
- **Workers become generic peer hosts** that load state components for
  any topic they are subscribed to.
- **Worker trust model shifts.** Today's workers run trusted in-tree
  Rust; under the runtime, a worker subscribed to N topics may be
  executing N distinct, third-party-authored, attacker-influenceable
  WASM state components simultaneously. DoS resistance, fuel scheduling,
  per-instance memory caps, fair-share between topics, and operator-level
  deny-lists are load-bearing operational concerns, not bandwidth/latency
  tuning. Operators must be able to constrain which apps a worker will host.
- **A kernel crate emerges** gathering the privileged subsystems described
  above; an app-SDK crate emerges as the authoring surface for app
  components.

Migration from today's codebase to this layout is its own multi-spec
effort and will be planned separately.

## ABI commitments

We commit to **WIT-shaped semantics** as the eventual interface ABI. We have
not yet committed to a v1 implementation path. Two candidates:

- **(A) Full WebAssembly Component Model from day one.** wit-bindgen,
  wasmtime native, jco-transpiled glue + core wasm in browser. Ecosystem-aligned.
  Cost: heavier toolchain, browser CM is still maturing, ~350 KB JS shim
  floor in browser, no async on the browser side.
- **(B) Extism for v1, WIT-shaped where possible.** Ship faster on a simpler
  runtime. Every *host-call* signature is chosen to be WIT-expressible
  (records, variants, lists, strings, integers). Cross-component composition
  in v1 is **kernel-brokered RPC by opaque ID only** — Extism has no notion
  of imported/exported resource handles, borrowed lifetimes, world
  composition, or futures/streams, and we do not pretend it does. Migration
  to full Component Model later is a real refactor for app authors (resource
  handles replace ID lookups, imported interfaces replace kernel-broker calls,
  borrows replace clone-and-pass), not a regenerate-bindings event.

The migration story is therefore: (a) *host-side* signatures we design today
will translate mechanically; (b) *cross-component composition* will be
rewritten when we move to Component Model; (c) any v1 plugin author should
expect to update their code at the migration boundary, but not redesign their
state machine or domain model.

Tentative lean: (B). Decision will be settled in a child spec on ABI &
runtime backends, including an explicit table of which v1 conveniences will
require app-author refactor at migration time.

## Constraints we accept

- **All cross-component calls go through the kernel.** Runtime composition
  in WASM is host-mediated; this aligns with our capability model anyway.
- **Coarse-grained interfaces.** No tight inner-loop callbacks across
  component boundaries. Interaction components return view models in
  per-surface units (e.g. one channel timeline, one member list, one
  composer state) — not per-element callbacks, but also not "the whole
  app's view." Returns are version-tagged so the host can skip
  recomposition on no-op state changes; large lists (timelines, member
  rosters) are paged. Behavior components observe and emit in batches.
  Exact diffing/paging strategy is for the WIT-interfaces child spec.
- **Sync ABI at v1, with kernel-side async bridged via tokens.** Browser
  jco does not support async. State `apply` is sync by definition.
  Kernel calls that wrap inherently async surfaces (gossip broadcast,
  blob fetch, HTTP, persistent KV, timers) follow a *submit-and-poll*
  pattern: the component calls a sync host function that returns a
  `request-token`, then the kernel later re-enters the component (via
  an exported `on-completion(token, result)` handler in the appropriate
  profile) when the operation finishes. This keeps the WIT surface sync
  while preserving back-pressure: a slow blob fetch does not stall the
  component's actor mailbox, because the originating call returned
  immediately. The ergonomic cost is real — apps cannot use familiar
  `async`/`await` flow control, and SDK macros are expected to hide the
  token-juggling for common patterns. Exact handler-method shape is for
  the WIT-interfaces child spec.
- **Pre-check fails closed.** When the kernel's dry-run pre-check panics,
  exhausts fuel, traps, or loops up to the deterministic budget, the
  user-action that triggered it is rejected and the event is *not*
  signed. Failing open (admitting an event that every peer rejects at
  `apply`) is forbidden because rejected events accumulate in the
  per-author DAG and cannot be removed without breaking the chain — the
  exact failure mode the existing authority spec was designed to make
  impossible. Adversarial app components that always-fail pre-check
  produce a self-DoS of the user's own ability to act in that app, which
  is detectable and recoverable by uninstalling the app.
- **Behavior identity is per-(peer, behavior-instance).** When a peer
  enables a behavior, the kernel generates and custodies a fresh Ed25519
  keypair scoped to that peer and that instance. Events authored through
  `host.broadcast` are signed under that identity, not the user's. The
  runtime does *not* migrate behavior keypairs between peers; cross-peer
  behavior continuity is an app-level concern. Apps that need a stable
  "bot identity" across peers define an in-band registration event
  mapping a peer-side behavior keypair to an app-level role
  (the "bot user" pattern), enforced by the app's own pre-check.
  Behavior components never see private keys; key custody is
  identical to the user-identity custody story. **This is structurally
  the same problem as multi-device user identity** (long-term identity,
  short-lived per-device signing key) which the seal-gift-wrap deferral
  spec calls out as non-negotiable: both should share a kernel-level
  mechanism rather than be invented twice.
- **Opaque IDs, not typed resource handles, between components.** Until
  wit-bindgen unifies imported and exported resource types, components pass
  string/u64 IDs and the kernel resolves them.
- **Two runtime backends in the kernel.** wasmtime native, jco-transpiled
  web. Same host interface so app authors target one ABI.
- **Relays are gossip-driven, not state-driven.** The relay never inspects
  app payloads, never materializes state, and never runs WASM. Topic
  discovery at the relay remains a transport-layer concern. App-defined
  topic-ID rotation (as used today by the epoch-rotation spec, where
  future topic IDs are intentionally unpredictable to non-members) must
  bridge a relay across rotations *without* publishing rotated IDs on a
  public channel — naive public discovery would defeat the rotation's
  unlinkability property. The likely shape is members announcing the
  next topic to the existing relay session before rotation, but the
  exact protocol is deferred to a relay-and-rotation child spec; the
  master-spec commitment is only that the kernel is not in this loop.
  Practical consequence: the in-flight epoch-rotation work
  (`docs/specs/2026-04-24-epoch-key-rotation.md`) needs to land in this
  new shape — the relay will no longer be told "this is a rotation
  event, here's the next topic id" by app code, because the relay no
  longer runs app code.
- **Deterministic-by-construction for state-`apply`.** The only host
  imports bound to `apply` are the deterministic helper set (signature
  verification, hash, payload-MAC verification, key installation, HLC
  extraction, log). Each is a pure function of its inputs given the
  event payload. Determinism is proven by the absence of any
  *non-deterministic* import — not by the absence of imports altogether.

## Lineage and influences

The design draws on a recognizable tradition of ambitious systems:

- **Holochain** — P2P apps as deterministic state machines (DNAs) plus UIs.
- **Urbit** — personal computer as peer, deterministic state, content-addressed.
- **AT Protocol** — apps composing on a shared identity + repo protocol.
- **Spritely / OCapN** — capability-secure distributed objects.
- **WebAssembly Component Model** — the typed composition substrate.
- **Erlang / OTP** — supervised lightweight processes, message passing.
- **Slack / Discord apps** — the user-facing "install an app, grant capabilities" model.

What is novel about Willow's combination is the marriage of (a) a
content-addressed, signature-rooted, gossip-synced DAG kernel with
(b) WASM Component Model semantics for typed composition, on (c) iroh as
the transport. None of the influences above ship all three together.

## MVP, in spirit (not in detail)

The smallest end-to-end demonstration that the runtime is real:

1. The kernel can load and instantiate a WASM state component from a bundle
   fetched via iroh-blobs.
2. The component applies events deterministically; multiple peers running
   the same component bytes converge to the same state hash.
3. A UI app can load an interaction component for that state, project a view,
   submit a command, observe the resulting state change.
4. A second app instance (different state component, different topic)
   coexists on the same peer; events do not cross.
5. Capability declarations actually gate access — a component cannot import
   an interface its manifest does not declare.
6. A behavior component can run on a designated peer, observe events, and
   log them. Emitting events under a kernel-custodied behavior identity is
   the next milestone after MVP, blocked on the capability model + identity
   custody child specs landing first.

What demo app proves this is an open child-spec question. Candidates: a tiny
shared-counter app (~50 lines of state, ~100 lines of interaction); a
single-channel chat that doesn't reuse `ServerState`; a real-time poll. The
toy app's job is to be irrelevant to chat — proving the kernel doesn't know
about chat — while still exercising the determinism + interaction loop.

## Child specs (planned)

To be written incrementally. Anticipated topics, in roughly the order they
become useful:

- **Kernel boundary** — what stays in `willow-kernel` vs becomes an app, exact
  trait surface, what's privileged.
- **ABI & runtime backends** — the (A) vs (B) decision; Extism integration
  details if (B); WIT-shaped contract design.
- **WIT interfaces** — `ui:*`, `state:*`, `behavior:*`, `host:*` interfaces;
  versioning policy; resource/handle conventions.
- **Capability model & install UX** — manifest format, default-deny, prompts,
  scoped grants.
- **Distribution, signing & versioning** — bundle format, hash chain,
  signatures, manifest evolution, multi-target artifacts (native + web
  transpiled).
- **App SDK ergonomics** — Rust macros, dual-build native/WASM, test harness,
  scaffolding CLI.
- **Determinism enforcement** — fuel policy, cross-impl verification,
  state-hash gossip.
- **State materialization on workers** — how `replay` and `storage` become
  generic; bandwidth/latency tradeoffs; snapshot custody.
- **Worker as untrusted-WASM execution host** — fuel scheduling, fair-share
  across topics, per-instance resource caps, operator deny-lists, abuse
  surfaces. Distinct from materialization, which is about correctness;
  this is about operating workers safely at scale.
- **Relay and topic-ID rotation** — how a relay continues to bridge a
  topic across an app-driven rotation without the kernel knowing what an
  epoch is and without leaking rotation linkability to the public
  network. Likely member-announced via the pre-rotation session.
- **Crypto and key custody boundaries** — the `host.seal` / `host.open` /
  `host.mls` interface, key-derivation strategy, persistence story, app
  ↔ kernel responsibility split for rotation.
- **Runtime and actor coexistence** — exact actor topology, mailbox
  semantics across the WIT boundary, lock/actor decision tree updates.
- **MVP demo app** — what it is, what it proves, what it doesn't have to.
- **chat-server migration** (much later) — extracting today's `ServerState`
  into the `chat-server` app on top of the runtime.

## Open questions deferred to child specs

- v1 ABI path: full Component Model now, or Extism with WIT-shaped subset
  and migrate later?
- Topic root: how is the (state-component-hash, genesis-hash) tuple pinned —
  encoded in the topic ID directly, or in a `PinComponent` event?
- Cross-app authority composition: out of scope for v1, but what shape
  should the v2 hooks take?
- Resource limits: per-instance fuel and memory budgets — what defaults?
- Worker capability advertisement: parallel to the existing relay
  capability document, should workers advertise which app-component
  hashes they host, so peers can discover "a worker that materializes
  my chat-server app" without out-of-band config? Or stays operator-config?
- Pre-check fuel budget: pre-check runs under the `apply` profile, but
  it runs *only on the originating peer*; does it share `apply`'s
  per-event fuel cap, or is it budgeted separately? (The polarity is
  master-level: pre-check fails closed — see "Constraints we accept".)
- Handle namespace ownership: two apps installing keys under the same
  opaque handle on one peer — collision, namespacing per-app instance,
  or kernel-arbitrated allocation?
- Snapshot portability across component-version upgrades: when an app's
  state component is updated, do existing snapshots remain valid? What
  is the migration story?
- Multi-peer behavior coordination: when two peers run instances of the
  same behavior for redundancy, dedup of emitted events and leader
  election are app-level concerns; the runtime offers no kernel-level
  coordination primitive. Apps that need single-emitter semantics
  implement leader election in their own state component. Should the
  runtime offer a shared primitive, or stay strict?
- Hot reload: deferred. Component update is restart for v1.

## Status

This spec is exploratory and will iterate. It is the agreed framing as of
2026-04-27 between the human author and an AI brainstorming session that
included four parallel research agents (WIT toolchain maturity, browser
Component Model state, Rust UI framework survey, Bevy specifics). The
decision to capture this as a runtime — not a plugin system — was made late
in that conversation and is the load-bearing reframing.

Nothing here is committed code. The first concrete step is whichever child
spec we write next.
