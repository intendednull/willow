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

## Three runtime profiles for components

Different components have fundamentally different needs. The kernel
distinguishes three profiles, with very different host imports and execution
policies:

| Profile | Determinism | Imports | Where it runs | Examples |
|---|---|---|---|---|
| **State** | **Required** — bit-identical across peers | `host.log` only | Every peer materializing the topic | chat-server-state, wiki-state, polls-state |
| **Interaction** | Not required | `host.broadcast`, `host.subscribe`, `host.kv`, `host.user-prompt`, UI app's `ui:*` | Any peer with a UI / agent host | chat-server-interaction, wiki-interaction |
| **Behavior** | Not required | + `host.http`, `host.timer`, `host.identity` (own keypair, gated) | Designated peer(s) | bridges, automod, archivers, bots |

All three are loaded by the same kernel through the same WIT-typed interface.
The difference is *which host imports each profile is permitted to bind* and
*which fuel/time policy applies*. Determinism is enforced for state
components by the absence of any non-deterministic host import — there is
nothing to call.

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
`ui:menu`, etc. — that other apps' interaction components import. It is not
privileged in the kernel; it is bound to DOM imports as a capability,
shipped in-tree for convenience, but architecturally indistinguishable from
a third-party UI.

Other UI apps are possible at different levels of effort:

- **`willow-ui-tui`** — terminal, ratatui rendering, same WIT contract.
- **`willow-ui-mcp`** — agent host, structured-data rendering for an LLM.
- **`willow-ui-mobile-native`** — Compose / SwiftUI shell, future.
- **`willow-ui-dioxus`** — once Dioxus Blitz is mature, replaces Leptos.

App authors target the WIT contract, not a specific UI. Their interaction
components work against any UI app that exports the interfaces they import.

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

## Determinism, in detail (for state components)

State components are pure functions of their inputs. The kernel passes the
event author, the event hash, the HLC, and the event payload. The component
returns a mutated state (held in linear memory) and optionally a snapshot.

To preserve cross-peer determinism, state components have:

- No wall clock. HLC bytes only.
- No randomness. Hash-derived if needed.
- No network, no filesystem, no environment access.
- No threads.
- A deterministic fuel budget (instruction count, not wall time). Running out
  of fuel terminates uniformly across peers.
- Spec-deterministic floats (the WASM spec pins these), with a strong
  recommendation to ban them in v1 anyway to avoid review pain.

The kernel verifies cross-peer convergence by hashing snapshots and gossiping
state hashes. Mismatches surface as bugs (or, if signed, as proof of a
malicious or buggy component).

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

State components have a deliberately *empty* capability surface beyond
`host.log`. There is nothing to grant; nothing to leak.

## What stays the same about Willow

- Event-sourced per-author Merkle DAG with prev/deps causal links.
- Identity rooted in Ed25519 signatures.
- iroh for transport (gossip + blob fetch).
- Relays remain dumb topic-bridges; they do not materialize state.
- Workers (`replay`, `storage`) remain peers, just generalized to host
  arbitrary state components instead of being chat-specific.
- The dual-target (native + WASM) compilation discipline maps directly to
  the runtime's two backends.
- The existing capability/permission ideas from `willow-state` generalize:
  each app defines its own permission set, the kernel does not.

## What changes about Willow

- `willow-state` splits. The kernel half (`Event`, `EventDag<P>`,
  `PendingBuffer`, sync, HLC) stays. The chat half (`EventKind`,
  `ServerState`, `apply_event`, `required_permission`) becomes the
  `chat-server` app, eventually shipped in-tree at `crates/apps/chat-server/`.
- `willow-web` becomes the default UI app, shipped in-tree at
  `crates/apps/ui-leptos/`. Its bindings to chat semantics route through
  the kernel and the chat-server interaction component, not through direct
  Rust imports.
- `replay` and `storage` workers become generic peer hosts that load
  state components for any topic they are subscribed to.
- A new top-level crate `willow-kernel` (or similar) gathers what the kernel
  contains. A new `willow-app-sdk` crate is what app authors use.

These are *consequences* of the design, not v1 work items. Migration is its
own multi-spec effort and will be planned separately.

## ABI commitments

We commit to **WIT-shaped semantics** as the eventual interface ABI. We have
not yet committed to a v1 implementation path. Two candidates:

- **(A) Full WebAssembly Component Model from day one.** wit-bindgen,
  wasmtime native, jco-transpiled glue + core wasm in browser. Ecosystem-aligned.
  Cost: heavier toolchain, browser CM is still maturing, ~350 KB JS shim
  floor in browser, no async on the browser side.
- **(B) Extism for v1, WIT-shaped subset.** Ship faster on a simpler runtime;
  every component call has a WIT-expressible signature; migrate to full
  Component Model when browser tooling is mature. Cost: known migration
  later. Reward: faster v1, real-world component authoring before the ABI is
  locked.

Tentative lean: (B). Decision will be settled in a child spec on ABI &
runtime backends.

## Constraints we accept

- **All cross-component calls go through the kernel.** Runtime composition
  in WASM is host-mediated; this aligns with our capability model anyway.
- **Coarse-grained interfaces only.** No tight inner-loop callbacks across
  component boundaries. Interaction components return whole view models per
  state change; behavior components observe and emit in batches.
- **Sync ABI at v1.** Browser jco does not support async. State components
  are sync by definition; the rest fit.
- **Opaque IDs, not typed resource handles, between components.** Until
  wit-bindgen unifies imported and exported resource types, components pass
  string/u64 IDs and the kernel resolves them.
- **Two runtime backends in the kernel.** wasmtime native, jco-transpiled
  web. Same host interface so app authors target one ABI.
- **Deterministic-by-omission for state.** No host imports = no
  non-determinism. We do not implement runtime checks for nondeterminism
  because the absence of imports is the proof.

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
   emit events that propagate to other peers.

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
- **MVP demo app** — what it is, what it proves, what it doesn't have to.
- **chat-server migration** (much later) — extracting today's `ServerState`
  into the `chat-server` app on top of the runtime.

## Open questions deferred to child specs

- v1 ABI path: full Component Model now, or Extism with WIT-shaped subset
  and migrate later?
- Topic root: how is the (state-component-hash, genesis-hash) tuple pinned —
  encoded in the topic ID directly, or in a `PinComponent` event?
- Behavior component identity: own keypair, granted permissions via the
  state component's permission system (i.e. "bot user")?
- Cross-app authority composition: out of scope for v1, but what shape
  should the v2 hooks take?
- Resource limits: per-instance fuel and memory budgets — what defaults?
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
