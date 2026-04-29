// e2e/test-hooks.ts
//
// JS-side wrapper for window.__willow + the __willowEvent push stream
// installed by crates/web (--features test-hooks). See:
//   docs/specs/2026-04-27-event-based-waits-design.md
//
// Types here mirror the Rust WireEvent / SnapshotDto / ChannelDto shapes.
// Keep in sync with crates/web/src/test_hooks/{wire,snapshot}.rs.

import type { Page, BrowserContext } from '@playwright/test';

// ── Mirror of crates/web/src/test_hooks/wire.rs::WireEvent ─────────────

export type ClientEvent =
  | { kind: 'SyncCompleted'; opsApplied: number }
  | { kind: 'MessageReceived'; channel: string; messageId: string; isLocal: boolean }
  | { kind: 'PeerConnected'; peerId: string }
  | { kind: 'PeerDisconnected'; peerId: string }
  | { kind: 'ChannelCreated'; name: string }
  | { kind: 'ChannelDeleted'; name: string }
  | { kind: 'PeerTrusted'; peerId: string }
  | { kind: 'PeerUntrusted'; peerId: string }
  | { kind: 'ProfileUpdated'; peerId: string; displayName: string }
  | { kind: 'RoleCreated'; roleId: string; name: string };

// ── Mirror of crates/web/src/test_hooks/snapshot.rs ────────────────────

export interface AuthorHead {
  seq: number;
  /** 64-char lowercase hex (EventHash::Display). */
  hash: string;
}

export interface ChannelSummary {
  name: string;
  /** Mirror of willow_state::ChannelKind — serialized as the variant name. */
  kind: 'Text' | 'Voice';
}

export interface Snapshot {
  eventCount: number;
  /** Per-author DAG heads. Keys are EndpointId hex strings (BTreeMap → sorted). */
  heads: Record<string, AuthorHead>;
  /** Hex hash of most recently applied event, or null if the DAG is empty. */
  lastEvent: string | null;
  channels: ChannelSummary[];
}

// ── Internal: window.__willow surface ──────────────────────────────────

/** Shape installed at `window.__willow` by crates/web/src/test_hooks/mod.rs. */
interface WillowTestHooksJS {
  snapshot(): Promise<Snapshot>;
  heads(): Promise<Record<string, AuthorHead>>;
  event_count(): Promise<number>;
  last_event(): Promise<string | null>;
}

/** Sentinel: queue + Page + label. Returned by the fixture, not exported as a type. */
type PeerInternals = {
  page: Page;
  label: string;
  queue: ClientEvent[];
};

// Stub — runtime classes/fixtures land in later tasks.
export {};
