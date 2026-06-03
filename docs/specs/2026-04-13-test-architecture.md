# Test Architecture Specification — Willow

**Date**: 2026-04-13
**Status**: Superseded by [docs/specs/2026-04-21-e2e-test-architecture-design.md](2026-04-21-e2e-test-architecture-design.md)

---

## 1. Testing Philosophy

### Purpose

Tests in Willow exist to enforce three guarantees:

1. **Correctness of security-critical invariants** — Signatures must not be forgeable. Invalid auth tags must be rejected. Unauthorized events must never reach state. These are properties where a runtime failure is a security breach, not just a bug.

2. **Determinism of the state machine** — The same sequence of events must always produce the same state, on any machine, at any time. Divergence here means peers disagree on reality. Tests make this guarantee visible and non-negotiable.

3. **Regression prevention at protocol boundaries** — Wire format changes, permission model changes, and HLC semantics must be caught before they break running deployments. Tests are the contract between versions.

Everything else is secondary. Tests are not written to satisfy coverage tools, to demonstrate that Rust compiles, or to document already-obvious behavior.

### What Tests Are NOT For

- Proving that your feature "works" — the compiler and type system already enforce a great deal. Tests fill the gap the compiler cannot fill.
- Coverage theater — a test that calls a function and asserts nothing is worse than no test. It adds maintenance burden while providing false assurance.
- Testing the test framework — do not write tests to confirm that `assert_eq!` behaves correctly or that `Vec::push` appends.

### Cost/Benefit Calculus

**High investment warranted:**
- Security invariants (identity, crypto): a missed edge case is a vulnerability
- State machine logic (dag.insert, materialize, permission checks): determinism errors corrupt shared state across all peers
- Wire format roundtrips: protocol breaks are unrecoverable for deployed nodes

**Moderate investment:**
- Client integration paths: catch misconfiguration of correct components
- UI components: verify that signals fire and DOM updates correctly

**Low investment:**
- Adapter/glue code with no internal logic
- Error message formatting
- Any behavior fully determined by the type system

The rule of thumb: investment in a test should be proportional to the cost of discovering the failure in production (data corruption, security breach, network partition that never heals) versus the cost of catching it in a test (a few minutes of test runtime and CI).

---

## 2. The Willow Test Pyramid

```
        /‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
       /       E2E (Playwright)        \     ← Fewest, slowest, highest cost
      /‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
     /     Browser (wasm-pack/Firefox)   \
    /‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
   /    Integration (client, MemNetwork)   \
  /‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
 /       Library Unit (identity, crypto,     \
/        transport, messaging, HLC)           \
/‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
/       State Machine (pure, zero I/O)         \  ← Most, fastest, lowest cost
\‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾/
```

### Level 1 — State Machine Tests (willow-state)

**Location**: `crates/state/src/tests.rs`
**Runner**: `just test-state` (`cargo test -p willow-state`)
**Speed**: Instant. No I/O, no async runtime, no network.

**What it tests:**
- Every `EventKind` variant: application, deduplication, rejection
- Permission enforcement for every privilege-gated operation
- `dag.insert()` determinism: same input always produces identical DAG
- `materialize()` convergence: same DAG always produces same state
- Parent hash verification: events with wrong `prev` are rejected by `dag.insert()`
- Event replay from genesis produces identical state
- Permission grant → operation → permission revoke chain
- Owner/admin implicit permissions

**What it does NOT test:**
- Networking, I/O, storage, async runtimes
- Wire serialization (that belongs in willow-transport tests)
- UI rendering

**When to use this level**: Any change to `EventKind`, `dag.insert()`, `materialize()`, `apply_incremental()`, permission tables, or `Snapshot.hash` computation. This is the primary regression layer for state correctness.

### Level 2 — Library Unit Tests

**Location**: `#[cfg(test)] mod tests` in each crate's source files
**Runner**: `cargo test -p willow-<crate>`
**Speed**: Fast. May use real crypto and serialization, but no networking.

**What it tests:**
- willow-identity: Ed25519 sign/verify, signature forgery resistance, invalid key rejection
- willow-crypto: encrypt/decrypt roundtrip, tampered ciphertext rejection, key exchange protocol
- willow-transport: serialization roundtrip for every wire type, malformed input handling
- willow-messaging: HLC ordering under clock drift, message construction, store operations

**What it does NOT test:**
- Integration between crates
- Network behavior

**When to use this level**: Adding or changing a function in a library crate. Every public function with non-trivial logic must have at least one test at this level.

### Level 3 — Integration Tests

**Location**: `crates/client/src/lib.rs` test module; integration test files in crates with complex cross-crate wiring
**Runner**: `just test-client`
**Speed**: Moderate. Uses `MemNetwork` test double; no real sockets.
**Tooling**: `MemNetwork` from `willow-network` (test-utils feature), `test_client()` helper

**What it tests:**
- Client API methods: message send, channel create, trust, kick, invite
- State transitions triggered by client API calls
- Event store persistence and retrieval
- MemNetwork: multiple clients exchanging events, gossip propagation
- Replay node: sync request handling, delta delivery, snapshot fallback
- Storage node: event persistence, paginated history, deduplication

**What it does NOT test:**
- Real QUIC transport behavior
- Browser DOM
- OS networking stack

**When to use this level**: Adding a client API method, changing how the client wires state + network + storage together, or testing replay/storage-level semantics. Use `MemNetwork` — not a real iroh node — for any test that doesn't specifically require QUIC or NAT traversal.

**Limitation of MemNetwork**: It elides real connection establishment, QUIC reliability, and NAT traversal. A test passing against `MemNetwork` guarantees protocol logic, not transport reliability.

### Level 4 — Browser Tests

**Location**: `crates/web/tests/browser.rs`
**Runner**: `just test-browser` (wasm-pack + headless Firefox + geckodriver)
**Speed**: Slow (requires browser process). Run in CI, not during local iteration.

**What it tests:**
- Leptos component rendering in real DOM
- Signal reactivity: state change triggers correct re-render
- Event handling: user input events update signals
- Component-level behavior: sidebar, message list, channel list, settings, member list

**What it does NOT test:**
- P2P sync between peers
- State machine logic (already covered at Level 1)
- Network behavior

**When to use this level**: Adding a new UI component or changing signal topology. Use `mount_test()` + `tick().await` to render and flush effects.

**Important**: A browser test that just mounts a component without asserting on DOM content is worthless. Every browser test must assert on rendered output or emitted signals.

**WASM-specific requirements**:
- `just check-wasm` must pass after adding any new dependency — run it before committing
- No `std::time::SystemTime` in library crates; use `js_sys::Date::now()` on WASM
- `getrandom` must have the `wasm_js` feature enabled for WASM targets

### Level 5 — E2E Tests (Playwright)

**Location**: `e2e/*.spec.ts`
**Runner**: `just test-e2e-*`
**Speed**: Slowest. Requires full dev stack running.

**What it tests:**
- Multi-peer message sync: peer A sends, peer B receives
- Permission enforcement in a real session: kick, trust, role grant
- Mobile UI: touch interactions, sidebar swipe behavior
- Bootstrap connectivity: peer connects via relay and joins gossip mesh

**What it does NOT test:**
- Behavior already proven at lower levels — do not re-test state machine logic or crypto in E2E
- Unit-level scenarios that can be isolated without two real browser instances

**When to use this level**: Behavior that requires two or more peers in real browsers with real networking — P2P convergence, relay bootstrap, session establishment. These tests are expensive; write them only when a lower level cannot cover the requirement.

---

## 3. Per-Crate Test Requirements

### willow-state — Minimum Bar

This crate is the most critical. Every `EventKind` and every permission must be covered.

**Required tests:**

**EventKind coverage** — for every variant of `EventKind` (all 22), there must be tests for:
- Happy path: authorized author applies event, state updates correctly
- Duplicate: applying the same event twice produces identical state (idempotency via `apply_incremental`)
- Unauthorized: event applied by a peer without the required permission is rejected by `dag.insert()`
- Malformed parent hash: event with wrong `prev` hash is rejected by `dag.insert()`

**Actual EventKind variants** (22 total):

| EventKind | Required scenario |
|---|---|
| CreateServer | Genesis event establishes server, subsequent CreateServer is rejected (DuplicateGenesis) |
| Propose | Admin proposes action; non-admin proposal is rejected (PermissionDenied) |
| Vote | Admin votes on a known proposal; vote on unknown proposal hash is rejected (MissingGovernanceDep) |
| GrantPermission | Granted permission enables previously-rejected event; non-admin grant rejected |
| RevokePermission | Revoked permission re-enables rejection; non-admin revoke rejected |
| CreateChannel | Owner creates, member without ManageChannels is rejected |
| DeleteChannel | Channel deleted, subsequent events targeting it are rejected |
| RenameChannel | Rename visible in state, old name absent |
| CreateRole | Role appears in state |
| DeleteRole | Role removed from state |
| SetPermission | Permission set on role, reflected in state |
| AssignRole | Role assigned to member |
| Message | Message appears in channel state; member without SendMessages is rejected |
| EditMessage | Edit reflected in state; non-author edit rejected |
| DeleteMessage | Soft-delete reflected in state (body replaced, deleted=true) |
| Reaction | Reaction appears on message in state |
| SetProfile | Display name updated in state |
| RotateChannelKey | Encrypted key material stored per recipient |
| PinMessage | Message appears in channel pin list |
| UnpinMessage | Message removed from pin list |
| RenameServer | Server name updated in state |
| SetServerDescription | Description updated in state |

**Governance subsystem** — required scenarios:
- Propose by non-admin is rejected with PermissionDenied
- Vote on unknown proposal hash is rejected with MissingGovernanceDep
- Majority vote threshold: insufficient yes-votes do not trigger action; crossing threshold applies action
- Unanimous threshold: all admins must vote yes; one no-vote blocks
- Last admin cannot be kicked or have admin revoked (last-admin guard: `RevokeAdmin` and `KickMember` proposals must not leave the admin set empty)
- Vote on an already-applied proposal is safe (idempotent second application)

**Equivocation and sequence gaps** — required scenarios:
- Equivocating event: same author, same seq, different `prev` hash → rejected by `dag.insert()` with `InsertError::PrevMismatch`
- Gap in sequence number: author submits seq N+2 before N+1 → rejected by `dag.insert()` with `InsertError::SeqGap` until the gap is filled
- `PrevMismatch` events must be dropped (not buffered) because the prev they reference will never become the head

**Convergence** — required scenarios:
- Two peers diverge (different channels created independently), `materialize()` on each peer's DAG produces identical state given the same set of events
- Three-way: A, B, C each make concurrent mutations; all pairs converge identically when they have the same events
- Concurrent conflicting mutations on same field: deterministic winner (topological sort by `EventHash`)

**Permission chain** — required scenarios:
- Admin grants ManageChannels to peer A; peer A successfully creates channel
- Admin revokes ManageChannels from peer A; peer A's next channel create is rejected
- Admin has all non-admin permissions implicitly (`has_permission()` returns true for all `Permission` variants)
- Permission escalation: non-admin cannot emit `GrantPermission` or `RevokePermission`

**State hash / Snapshot** — required scenarios:
- Identical event sequence produces identical `Snapshot.hash` on separate state instances
- One differing event produces a different `Snapshot.hash`
- Empty DAG has no snapshot; non-empty DAG produces stable, defined hash

**Pending buffer** — required scenarios:
- Events arriving before their chain predecessors are buffered (not dropped)
- Buffered events are applied when predecessors arrive (cascade resolution)
- Buffer cap enforced: `PendingBuffer::with_capacity(N)` never exceeds N events
- Deep pending chains resolve without stack overflow (iterative queue, not recursion)

**Event replay from genesis** — required:
- Store N events, replay them in order into a fresh DAG, `materialize()` the result; state is identical to the original

### willow-identity — Minimum Bar

Security-critical. Every failure mode must be covered.

**Required tests:**
- Sign a message, verify with the same key: succeeds
- Sign a message, verify with a different public key: fails
- Sign a message, tamper with one byte of the signature: fails
- Sign a message, tamper with one byte of the message body: fails
- Keypair serialization roundtrip: serialize to bytes, deserialize, sign and verify still works
- Public key serialization roundtrip: same key before and after round-trip
- Zero-length message signing: succeeds (edge case, not a crash)
- Signature over event hash verifies correctly; tampering with any field of the signable content causes `Event::verify()` to return false
- Profile construction and retrieval: display name, avatar hash survive roundtrip
- Key file created with mode 0600 (Unix): load succeeds
- Loading a key file with mode 0644 returns `IdentityError::InsecurePermissions`
- `Identity` is `ZeroizeOnDrop`: secret key material is zeroed on drop (structural guarantee, verifiable by type annotation)

**Explicitly NOT required:**
- Testing that Ed25519 itself is mathematically correct (that is the job of the `ed25519-dalek` maintainers)

### willow-crypto — Minimum Bar

Security-critical. Correctness here prevents data exposure.

**Required tests:**
- Encrypt then decrypt with same key: plaintext recovered exactly
- Encrypt then decrypt with wrong key: returns error (not wrong plaintext — actual rejection)
- Tamper with one byte of ciphertext after encryption: auth tag fails, returns error
- Tamper with one byte of the auth tag directly: returns error
- Zero-length plaintext: encrypt/decrypt succeeds
- Large plaintext (64 KB+): encrypt/decrypt succeeds, no truncation
- X25519 key exchange: two parties perform exchange, arrive at the same shared secret
- X25519 key exchange with mismatched keys: shared secrets differ
- Key serialization roundtrip: derived key survives serialize/deserialize
- Key ratchet forward secrecy: old epoch key cannot decrypt ciphertext encrypted with a new epoch key
- Ratchet counter increments monotonically: `derive_message_key` at counter N is consistent across calls with the same ratchet state at N
- Ratchet counter bounds check: requesting a key at an out-of-range counter returns an error (DoS mitigation)

**Explicitly NOT required:**
- Testing ChaCha20-Poly1305 mathematical properties (that is the job of the `chacha20poly1305` crate)

### willow-transport — Minimum Bar

Protocol compatibility depends on this. Every wire type needs a roundtrip test.

**Required tests:**
- For every type that derives `Serialize + Deserialize` and appears on the wire:
  - Serialize to bytes, deserialize back, value equals original
  - This includes: all `EventKind` variants (even if they have no fields), envelope types, message types, relay framing types
- Malformed input: feed truncated bytes, extra bytes, or random bytes to the deserializer and verify it returns an error rather than panicking or producing garbage
- Version/magic byte: if the protocol includes a version prefix, an unknown version returns an error
- Maximum-size message: a message near the protocol size limit serializes and deserializes without error
- Empty/minimal message: smallest valid message roundtrips correctly

### willow-messaging — Minimum Bar

**Required tests:**
- HLC `now()`: timestamp is >= wall clock
- HLC `receive(remote_ts)`: after receiving a future timestamp, subsequent `now()` is >= that timestamp
- HLC ordering: given two messages where A is created after B's timestamp is received, A > B in total order
- HLC with zero drift: clock never goes backward
- Message construction: `Message::text()`, other constructors produce valid messages
- Message store append: stored message is retrievable by ID
- Message store ordering: messages retrieved in HLC order, not insertion order

### willow-network — Minimum Bar

The `MemNetwork` test double handles Level 3. The production `IrohNetwork` is not directly unit-tested (it wraps iroh). Focus on the trait contract.

**Required tests (against MemNetwork, `test-utils` feature):**
- Two `MemNetwork` instances exchange a broadcast message via gossip
- Subscriber receives message sent before it subscribed (history delivery)
- Unsubscribed client does not receive messages
- `TopicId` registry: same logical topic always produces same `TopicId` (blake3 determinism)
- `BlobStore` put/get roundtrip

### willow-relay — Minimum Bar

The relay is a NAT traversal and gossip bootstrap shim. It wraps `iroh-relay` for QUIC hole-punching, runs a bootstrap gossip node that subscribes to system topics (`_willow_server_ops`, `_willow_workers`, `_willow_profiles`), and serves a lightweight HTTP endpoint that returns the bootstrap node's `EndpointId`. It does NOT store events and does NOT bridge TCP↔WebSocket at the application level.

**Required tests:**
- `topic_str_is_valid`: accepts valid ASCII alphanumeric/underscore/slash/colon/dot/dash strings; rejects empty, too-long, and non-ASCII strings
- Bootstrap HTTP handler: sends a well-formed HTTP/1.1 200 response containing the node's EndpointId string and closes the connection within `BOOTSTRAP_IO_TIMEOUT`
- Bootstrap handler: a slow client (no read within timeout) triggers a timeout error, connection dropped
- Bootstrap handler: concurrent connection cap (`MAX_CONCURRENT_PROXY_CONNECTIONS`) — connections beyond the cap are dropped immediately
- Topic announce listener: valid `TopicAnnounce` message causes subscription to the announced topic
- Topic announce listener: invalid topic string (fails `topic_str_is_valid`) is rejected, no subscription created
- Topic announce listener: subscription cap (`MAX_TOPICS`) enforced — topics beyond the cap are silently dropped

### willow-replay — Minimum Bar

The replay node maintains per-server in-memory `EventDag` instances and responds to sync requests with event deltas or snapshots. It buffers out-of-order events and evicts the least-recently-used server when `MAX_SERVERS` is exceeded.

**Required tests:**
- Sync request with empty heads returns all events (new peer receives everything)
- Sync request with known heads returns only newer events (delta delivery)
- Events ingested are returned in subsequent sync requests
- LRU eviction: inserting more than `MAX_SERVERS` unique servers evicts the least-recently-used
- Snapshot response: peer that is behind (lower seq) receives a `WorkerResponse::Snapshot`
- Out-of-order events buffered and resolved when predecessors arrive
- Deep chains resolve without stack overflow (iterative queue, not recursion)
- Duplicate events are deduplicated (second ingest of same hash is a no-op)
- `WorkerRequest::History` is denied (replay nodes do not serve history)

### willow-storage — Minimum Bar

The storage node persists events to SQLite indefinitely and serves paginated history and sync requests. It uses WAL journaling, `synchronous = FULL`, and foreign-key enforcement for durability.

**Required tests:**
- Store and retrieve event by hash: round-trip equality (not just non-zero length)
- Deduplication by event hash: storing the same event twice results in one stored copy
- Pagination with cursor: `History` request with a `before` cursor returns the correct page and `has_more` flag
- Per-server isolation: events stored for server A are not returned when querying server B
- Per-channel filtering: `History` with `channel = Some("general")` excludes events from other channels
- `WorkerRequest::Sync` with empty heads returns all events for that server
- `WorkerRequest::Sync` with known heads returns only newer events (delta)
- Durability pragmas: `PRAGMA journal_mode` = `WAL` (on disk), `PRAGMA synchronous` = `FULL`, `PRAGMA foreign_keys` = `ON` — verified via `PRAGMA` query after open
- Schema migrations are idempotent: opening the same database twice does not fail or duplicate tables

### willow-worker — Minimum Bar

**Location**: `crates/worker/tests/integration.rs`
**Tests**: Actor orchestration, heartbeat/departure, event forwarding

**Required tests:**
- `StateActor` with a real `WorkerRole`: receives an event, forwards it to the role, role state reflects the change
- Pre-buffer race condition: event arrives before actor is ready; event is processed after actor becomes ready
- Heartbeat/departure: actor sends periodic heartbeat; departure is signalled on drop
- Event forwarding: `NetworkActor` receives gossip event, forwards to `StateActor`
- `WorkerRequest` round-trip: request sent to actor, response returned to caller

**Currently missing (known gaps):**
- `SyncActor` broadcast verification: events received from one worker are broadcast to peers
- Two-worker convergence: worker A and worker B exchange events until both have identical state

### willow-client — Minimum Bar

**Required tests:**
- `send_message`: event reaches state, state accessor returns message
- `create_channel`: channel appears in state
- `trust_peer`: peer gains appropriate permission
- `kick_member`: peer is removed from member list (requires governance vote path)
- `revoke_invite`: invite no longer valid
- Event store: events persist across client reconstruction
- Bridge conversion: all bridge event variants convert to/from state events without loss

---

## 4. Test Quality Standards

### High-Value vs. Low-Value Tests

**HIGH VALUE:**
- Tests that can catch a regression introduced by changing one line of logic
- Tests that verify rejection of invalid input (not just acceptance of valid input)
- Tests that verify correctness of security invariants (auth tag rejection, permission rejection)
- Tests that exercise state at the boundary between valid and invalid (off-by-one permission levels, events at the boundary of the duplicate window)
- Tests that verify convergence properties (merge tests, dedup tests)

**LOW VALUE:**
- Tests that call a constructor and assert the struct has the field values passed to the constructor
- Tests that call a happy path with valid input and assert success, when there are no code paths that could produce failure
- Tests that duplicate type-system guarantees (e.g., asserting that a field has the correct type)
- Tests that exist purely to hit a coverage threshold

### Negative Assertions: How to Assert Something Did NOT Happen

Many correctness properties are negative: an unauthorized event must not change state, a broker channel must not emit an event, a rejected operation must not silently succeed.

**Check state is unchanged after a rejected event:**
```rust
let state_before = materialize(&dag);
let result = dag.insert(unauthorized_event);
assert!(result.is_err());
let state_after = materialize(&dag);
assert_eq!(state_before.channels, state_after.channels); // state is unchanged
```

**Check no event was emitted (empty broker channel):**
```rust
// After an operation that should produce no event:
assert!(broker_rx.try_recv().is_err(), "no event should have been emitted");
```

**Do NOT use `waitForTimeout(N)` then check absence**: polling with a sleep is unreliable — the event might arrive after the timeout. Use explicit synchronization barriers or check the channel immediately after the synchronous operation completes.

### Assertion Standards

Every test must contain at least one assertion that can fail. The following patterns are prohibited:

```rust
// PROHIBITED: calls function but asserts nothing meaningful
#[test]
fn test_apply_event() {
    let state = State::new();
    let event = make_event(...);
    apply_incremental(&mut state, &event); // "at least it didn't panic"
}
```

```rust
// REQUIRED: asserts on the observable effect of the operation
#[test]
fn apply_create_channel_adds_channel_to_state() {
    let mut dag = EventDag::new();
    // ... insert genesis and create_channel events ...
    let state = materialize(&dag);
    assert!(state.channels.contains_key("general"));
}
```

For rejection tests, assert on the **error kind**, not just that an error occurred:

```rust
// BETTER: verify the specific failure mode
let result = dag.insert(unauthorized_event);
assert!(matches!(result, Err(InsertError::PermissionDenied(..))));
```

### Naming Conventions

Test function names must describe the **behavior under test**, not the function being called.

| Bad | Good |
|---|---|
| `test_apply` | `apply_create_channel_adds_channel_to_state` |
| `test_sign` | `sign_with_wrong_key_returns_error` |
| `test_encrypt_decrypt` | `tampered_ciphertext_is_rejected_with_auth_error` |
| `test_merge` | `concurrent_events_converge_to_identical_state` |
| `test_hlc` | `hlc_receive_future_timestamp_advances_local_clock` |

The pattern: `<subject>_<condition>_<expected_outcome>`.

### When to Use Test Doubles vs. Real Implementations

**Use real implementations when:**
- The behavior under test is the correctness of the implementation itself (crypto tests use real crypto)
- The test is at Level 1 or 2 (state machine and library unit tests have no expensive dependencies)
- The cost of using a real implementation is negligible (in-memory, no I/O)

**Use test doubles (`MemNetwork`) when:**
- The test is at Level 3 (integration) and real networking adds no correctness value
- The test would be flaky due to real network timing
- The test subject is the protocol logic, not transport reliability
- Parallelism: multiple integration tests must not fight over real ports

**Never use test doubles to avoid testing correctness.** If a test double would hide a real failure, it should not be used.

### MemNetwork: Appropriate Use and Limitations

`MemNetwork` is the right tool for:
- Testing that two clients exchange events via the protocol
- Testing gossip subscription semantics
- Testing replay history delivery logic
- Any test that needs multiple "peers" without real QUIC

`MemNetwork` is NOT a substitute for:
- Testing real connection establishment (E2E only)
- Testing NAT traversal behavior (E2E only)
- Testing behavior under real network delay or packet loss (not currently in scope)

When a test uses `MemNetwork` and passes, it proves the **protocol logic** is correct. It does not prove the transport layer is correct. Acknowledge this distinction when reading test results.

---

## 5. What NOT to Test

The following are explicit wastes of time in this codebase:

### Rust Compiler Guarantees
- Type compatibility: if the code compiles, types are correct
- Ownership and lifetimes: enforced at compile time
- `Send + Sync` bounds: enforced at compile time
- That a `Vec<T>` returned by a function contains `T` values

### Third-Party Library Correctness
- That `ed25519-dalek` correctly implements Ed25519
- That `chacha20poly1305` correctly implements ChaCha20-Poly1305
- That `iroh` establishes QUIC connections correctly
- That `sqlx` correctly executes SQL queries against SQLite
- That `serde_json` correctly encodes JSON

**Exception**: Test the *use* of these libraries at your crate's API boundary. If you call `ed25519-dalek` with the wrong key material, that is a bug in Willow, not in `ed25519-dalek`. Test your call sites, not their implementations.

### Trivial Constructors and Getters
Do not test:
```rust
let msg = Message::new(content, author);
assert_eq!(msg.author(), author); // the field literally came from the argument
```

Test constructors only when they contain non-trivial logic (normalization, validation, ID generation, timestamp assignment).

### Obvious Happy Paths Without Edge Cases
A function with a single code path that always succeeds does not need a test. A function with two code paths needs a test for both paths. The threshold for "needs a test" is: "would changing one line of this function's logic break a test?"

### Panic Behavior on Invalid State
Tests that deliberately put the system into impossible states and assert on panic behavior are not useful. The system should never be in impossible states; prevent that with proper type design and return `Result` instead of panicking.

---

## 6. Test Maintenance Rules

### Tests Must Track Behavior Changes

When behavior changes intentionally (a permission model update, a new `EventKind`, a protocol revision), the tests must be updated **in the same commit**. A PR that changes `dag.insert()` or `materialize()` without updating the tests for the affected `EventKind` is incomplete and must not be merged.

If a test fails after a behavioral change and the new behavior is correct, update the test to match the new specification. Do not delete the test — rewrite the assertion.

### Flaky Tests Are Bugs

A test that sometimes passes and sometimes fails is a bug in the test, the system under test, or both. Flaky tests must be diagnosed and fixed before being re-enabled. Acceptable root causes: real timing dependency (add explicit synchronization), non-deterministic ordering (use sorted comparison), or race condition in production code (fix the race).

Options that are NOT acceptable:
- Retry loops around the assertion
- `sleep()` to "give it time"
- Increasing a timeout until the flakiness is rare enough to ignore
- Marking the test `#[ignore]` without a tracking issue

### Test Names Describe Behavior

If a test name must change because the behavior it tests has been renamed or reorganized, rename it. Test names are documentation. A test named `test_apply_001` tells future readers nothing.

### Do Not Disable Tests to Unblock CI

A failing test is a failing system. If CI is blocked by a test failure, fix the system or explicitly revert the change that broke it. Disabling the test to make CI green hides a known defect.

---

## 7. Decision Guide: What Tests Do I Write?

```
I am adding/changing feature X
         │
         ▼
Does it change willow-state logic?
(EventKind, dag.insert(), materialize(), apply_incremental(), permissions, Snapshot.hash)
         │
    YES──┤──► Add/update tests in crates/state/src/tests.rs
         │    Cover: happy path, duplicate (idempotency), unauthorized, wrong parent hash
         │    Use: Level 1 (state machine tests)
         │
    NO───┤
         ▼
Does it change a library crate?
(identity, crypto, transport, messaging, HLC)
         │
    YES──┤──► Add/update tests in that crate's #[cfg(test)] mod tests
         │    Cover: valid input, invalid/malformed input, edge cases (empty, max size)
         │    For security crates: cover every rejection path
         │    Use: Level 2 (library unit tests)
         │
    NO───┤
         ▼
Does it change the client API or cross-crate integration?
(client methods, storage, event store, MemNetwork behavior, replay, worker)
         │
    YES──┤──► Add/update tests in crates/client/src/lib.rs test module
         │    Use MemNetwork and test_client() helpers
         │    Cover: operation succeeds, state reflects change, errors propagate correctly
         │    Use: Level 3 (integration tests)
         │
    NO───┤
         ▼
Does it change a UI component?
(Leptos signals, DOM rendering, event handlers)
         │
    YES──┤──► Add/update tests in crates/web/tests/browser.rs
         │    Use mount_test() + tick().await
         │    Assert on rendered DOM content, not just "no panic"
         │    Run: just check-wasm before committing
         │    Use: Level 4 (browser tests)
         │
    NO───┤
         ▼
Does it require two real peers or a real browser for meaningful verification?
(P2P sync, relay bootstrap, session handshake, mobile touch)
         │
    YES──┤──► Add/update tests in e2e/*.spec.ts
         │    Verify: peer A sends → peer B receives; state converges after partition
         │    Use: Level 5 (Playwright E2E)
         │
    NO───┤
         ▼
Does it change error message text, logging, or
something else fully determined by the type system?
         │
    YES──┤──► No test required.
         │
    NO───┘
         ▼
Write a Level 1 or 2 test if you can identify
a specific invariant the code must uphold.
If you cannot articulate the invariant, reconsider
whether the code needs a test.
```

### Quick Reference: Which Crate, Which Level

| Changed crate | Test level | Location |
|---|---|---|
| willow-state | Level 1 | `crates/state/src/tests.rs` |
| willow-identity | Level 2 | `crates/identity/src/lib.rs` |
| willow-crypto | Level 2 | `crates/crypto/src/lib.rs` |
| willow-transport | Level 2 | `crates/transport/src/lib.rs` |
| willow-messaging | Level 2 | `crates/messaging/src/lib.rs` |
| willow-network | Level 3 (MemNetwork) | `crates/network/src/mem.rs` tests |
| willow-relay | Level 3 | `crates/relay/src/lib.rs` tests |
| willow-replay | Level 3 | `crates/replay/src/role.rs` tests |
| willow-storage | Level 3 | `crates/storage/src/role.rs` tests |
| willow-worker | Level 3 | `crates/worker/tests/integration.rs` |
| willow-client | Level 3 | `crates/client/src/lib.rs` tests |
| willow-web | Level 4 | `crates/web/tests/browser.rs` |
| Multi-peer sync | Level 5 | `e2e/multi-peer-sync.spec.ts` |
| Permissions (session) | Level 5 | `e2e/permissions.spec.ts` |
| Mobile UI | Level 5 | `e2e/mobile.spec.ts` |

---

## Appendix: Required Invariants Summary

These are the properties that the test suite, taken as a whole, must guarantee. If any of these invariants is not covered by at least one test, the test suite is incomplete.

| # | Invariant | Covered at Level |
|---|---|---|
| I-1 | Same event sequence → same `Snapshot.hash`, always | 1 |
| I-2 | Unauthorized event → rejected by `dag.insert()` with `InsertError::PermissionDenied` | 1 |
| I-3 | Duplicate event → idempotent (`apply_incremental` returns `AlreadyApplied`, state unchanged) | 1 |
| I-4 | Wrong `prev` hash → rejected by `dag.insert()` with `InsertError::PrevMismatch` | 1 |
| I-5 | Divergent histories → `materialize()` on the merged DAG converges to identical state | 1 |
| I-6 | Genesis author is the initial admin; admins have all non-admin permissions implicitly | 1 |
| I-7 | Revoked permission re-enables rejection for affected operations | 1 |
| I-8 | All 22 EventKind variants: `dag.insert()` + `materialize()` produces correct state change | 1 |
| I-9 | Equivocating event (same seq, same author, different prev) → `InsertError::PrevMismatch`, dropped | 1 |
| I-10 | Sequence gap → `InsertError::SeqGap`, buffered until gap is filled | 1 |
| I-11 | Last-admin guard: `RevokeAdmin`/`KickMember` proposal that would empty admin set is rejected | 1 |
| I-12 | Governance: `Propose` by non-admin → `InsertError::PermissionDenied` | 1 |
| I-13 | Governance: `Vote` without proposal in deps → `InsertError::MissingGovernanceDep` | 1 |
| I-14 | Pending buffer cap: `PendingBuffer::with_capacity(N)` never exceeds N events | 1 |
| I-15 | Ed25519: tampered signature is rejected | 2 |
| I-16 | Ed25519: tampered message is rejected | 2 |
| I-17 | Ed25519: wrong public key rejects valid signature | 2 |
| I-18 | ChaCha20-Poly1305: tampered ciphertext returns auth error | 2 |
| I-19 | ChaCha20-Poly1305: wrong key returns error (not wrong plaintext) | 2 |
| I-20 | X25519: two parties derive same shared secret | 2 |
| I-21 | Key ratchet forward secrecy: old epoch key cannot decrypt ciphertext from new epoch | 2 |
| I-22 | All wire types: serialize → deserialize → equal to original | 2 |
| I-23 | Malformed wire input → error, not panic or garbage | 2 |
| I-24 | HLC: clock never goes backward | 2 |
| I-25 | HLC: receive(future_ts) advances local clock | 2 |
| I-26 | Two peers via MemNetwork exchange messages | 3 |
| I-27 | New peer receives full history from replay node via sync request | 3 |
| I-28 | Bootstrap endpoint returns peer ID and closes connection within configured timeout | 3 |
| I-29 | Client API: all operations reflected in state | 3 |
| I-30 | Storage: event store round-trip equality by hash | 3 |
| I-31 | Storage: deduplication by event hash | 3 |
| I-32 | Leptos components render without error | 4 |
| I-33 | Multi-peer: message sent by A is received by B | 5 |
| I-34 | Bootstrap: peer connects via relay node and joins gossip mesh within timeout | 5 |
