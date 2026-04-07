//! Unified DAG + materialized state container.
//!
//! [`ManagedDag`] holds an [`EventDag`], its [`ServerState`], and a
//! [`PendingBuffer`] together. All mutations go through
//! [`insert_and_apply`](ManagedDag::insert_and_apply) which atomically
//! inserts into the DAG and applies to state, making it structurally
//! impossible for the two to diverge.

use willow_identity::Identity;

use crate::dag::{EventDag, InsertError};
use crate::event::{Event, EventKind};
use crate::hash::EventHash;
use crate::materialize::{apply_incremental, ApplyResult};
use crate::server::ServerState;
use crate::sync::PendingBuffer;

/// A DAG paired with its materialized state and pending buffer.
///
/// All mutations go through [`insert_and_apply`](Self::insert_and_apply)
/// which guarantees the `ServerState` is always consistent with the DAG
/// contents. This eliminates the crash-safety window where an event is in
/// the DAG but not in state.
#[derive(Clone)]
pub struct ManagedDag {
    dag: EventDag,
    state: ServerState,
    pending: PendingBuffer,
    synced: bool,
}

/// Result of inserting an event into the managed DAG.
#[derive(Debug)]
pub struct InsertOutcome {
    /// The event that was applied (if insertion succeeded).
    pub applied: Option<Event>,
    /// Additional events that were resolved from the pending buffer
    /// and recursively applied.
    pub resolved: Vec<Event>,
    /// The apply result for the primary event (if it was inserted).
    pub apply_result: Option<ApplyResult>,
}

impl ManagedDag {
    /// Create a new ManagedDag by seeding genesis.
    ///
    /// The genesis `CreateServer` event is immediately inserted and applied,
    /// so `state()` is ready to use after construction.
    pub fn new(identity: &Identity, server_name: &str, max_pending: usize) -> Self {
        let mut dag = EventDag::new();
        let genesis = dag.create_event(
            identity,
            EventKind::CreateServer {
                name: server_name.into(),
            },
            vec![],
            0,
        );
        dag.insert(genesis.clone())
            .expect("genesis insert must succeed");
        let state = crate::materialize::materialize(&dag);
        Self {
            dag,
            state,
            pending: PendingBuffer::with_capacity(max_pending),
            synced: true,
        }
    }

    /// Create an empty ManagedDag (no genesis yet).
    ///
    /// Used when the DAG will be populated via sync. The `synced` flag
    /// starts as `false` and should be set after genesis arrives.
    /// State is initialized with placeholder values that will be
    /// overwritten when genesis is received via `insert_and_apply`.
    pub fn empty(max_pending: usize) -> Self {
        // Use a dummy genesis author — will be replaced when actual genesis arrives.
        let dummy_author = willow_identity::EndpointId::from_bytes(&[0u8; 32]).unwrap();
        Self {
            dag: EventDag::new(),
            state: ServerState::new("", "", dummy_author),
            pending: PendingBuffer::with_capacity(max_pending),
            synced: false,
        }
    }

    /// Insert an event and atomically apply it to state.
    ///
    /// On success, resolves any pending events whose predecessor is now
    /// satisfied, recursively inserting and applying them.
    ///
    /// On chain gap or missing genesis, the event is buffered in the
    /// pending buffer (which self-enforces its capacity limit).
    pub fn insert_and_apply(&mut self, event: Event) -> Result<InsertOutcome, InsertError> {
        match self.dag.insert(event.clone()) {
            Ok(()) => {
                // For genesis events, re-initialize state properly
                // since apply_incremental treats CreateServer as a no-op.
                let apply_result = if matches!(event.kind, EventKind::CreateServer { .. }) {
                    self.state = crate::materialize::materialize(&self.dag);
                    ApplyResult::Applied
                } else {
                    apply_incremental(&mut self.state, &event)
                };

                let mut resolved_from_hash = self.pending.resolve(&event.hash);
                if matches!(event.kind, EventKind::CreateServer { .. }) {
                    resolved_from_hash.extend(self.pending.resolve(&EventHash::ZERO));
                    // Mark as synced once genesis is received.
                    if !self.synced {
                        self.synced = true;
                    }
                }

                // Recursively apply resolved events.
                let mut all_resolved = Vec::new();
                for r in resolved_from_hash {
                    match self.insert_and_apply(r.clone()) {
                        Ok(outcome) => {
                            all_resolved.push(r);
                            all_resolved.extend(outcome.resolved);
                        }
                        Err(_) => {
                            // Resolved event failed insertion (e.g. duplicate,
                            // equivocation) — skip it.
                        }
                    }
                }

                Ok(InsertOutcome {
                    applied: Some(event),
                    resolved: all_resolved,
                    apply_result: Some(apply_result),
                })
            }
            Err(InsertError::SeqGap { .. }) | Err(InsertError::NotGenesis) => {
                self.pending.buffer_for_prev(event.prev, event);
                Ok(InsertOutcome {
                    applied: None,
                    resolved: vec![],
                    apply_result: None,
                })
            }
            Err(InsertError::Duplicate) => Ok(InsertOutcome {
                applied: None,
                resolved: vec![],
                apply_result: None,
            }),
            Err(e) => Err(e),
        }
    }

    /// Create a local event and atomically insert + apply it.
    ///
    /// Computes cross-author causal dependencies from the current DAG heads.
    pub fn create_and_insert(
        &mut self,
        identity: &Identity,
        kind: EventKind,
        timestamp_ms: u64,
    ) -> Result<Event, InsertError> {
        if !self.synced {
            return Err(InsertError::NotGenesis);
        }

        let my_id = identity.endpoint_id();
        let mut deps: Vec<EventHash> = self
            .dag
            .authors()
            .filter(|a| **a != my_id)
            .filter_map(|a| self.dag.head(a).copied())
            .collect();

        // Vote events must causally depend on the proposal.
        if let EventKind::Vote { ref proposal, .. } = kind {
            if !deps.contains(proposal) {
                deps.push(*proposal);
            }
        }

        let event = self.dag.create_event(identity, kind, deps, timestamp_ms);
        self.insert_and_apply(event.clone())?;
        Ok(event)
    }

    /// Read-only access to the DAG.
    pub fn dag(&self) -> &EventDag {
        &self.dag
    }

    /// Read-only access to the materialized state.
    pub fn state(&self) -> &ServerState {
        &self.state
    }

    /// Read-only access to the pending buffer.
    pub fn pending(&self) -> &PendingBuffer {
        &self.pending
    }

    /// Whether the DAG has been populated with genesis.
    pub fn is_synced(&self) -> bool {
        self.synced
    }

    /// Manually set the synced flag.
    pub fn set_synced(&mut self, synced: bool) {
        self.synced = synced;
    }
}

/// Default creates an empty ManagedDag with a 5000-event pending buffer.
impl Default for ManagedDag {
    fn default() -> Self {
        Self::empty(5_000)
    }
}
