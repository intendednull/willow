//! Actor system — owns the runtime and tracks all top-level actors.
//!
//! The system itself is an actor (`SystemActor`), eliminating the need
//! for a mutex on the actor registry. FIFO message ordering guarantees
//! all `Register` messages are processed before a `Shutdown` message.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::actor::Actor;
use crate::addr::Addr;
use crate::context::Context;
use crate::envelope::BoxEnvelope;
use crate::mailbox;
use crate::runtime::{self, OneshotRx, Sender, DEFAULT_MAILBOX_CAPACITY};

// ───── SystemActor (internal) ──────────────────────────────────────────

struct ActorEntry {
    /// Sets the stop flag on the actor's mailbox.
    signal_stop: Box<dyn Fn() + Send>,
    /// Signals when the actor's mailbox loop exits.
    done_rx: Option<OneshotRx<()>>,
}

/// Internal actor that owns the actor registry. No mutex needed —
/// all access is serialized through the mailbox.
struct SystemActor {
    entries: Vec<ActorEntry>,
}

impl crate::actor::Actor for SystemActor {}

/// Register a new actor for shutdown tracking.
struct Register(ActorEntry);
impl crate::actor::Message for Register {
    type Result = ();
}

impl crate::actor::Handler<Register> for SystemActor {
    fn handle(
        &mut self,
        msg: Register,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        self.entries.push(msg.0);
        async {}
    }
}

/// Shutdown: stop all tracked actors and return their done receivers.
struct Shutdown;
impl crate::actor::Message for Shutdown {
    type Result = Vec<OneshotRx<()>>;
}

impl crate::actor::Handler<Shutdown> for SystemActor {
    fn handle(
        &mut self,
        _msg: Shutdown,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = Vec<OneshotRx<()>>> + Send {
        let entries = std::mem::take(&mut self.entries);
        let mut done_rxs = Vec::new();
        for mut entry in entries {
            (entry.signal_stop)();
            if let Some(rx) = entry.done_rx.take() {
                done_rxs.push(rx);
            }
        }
        async move { done_rxs }
    }
}

// ───── Public API ──────────────────────────────────────────────────────

/// The actor system — tracks all top-level actors.
///
/// Dropping the `System` without calling `shutdown()` will stop
/// the system actor, but tracked actors will continue running
/// until their addresses are dropped.
pub struct System {
    handle: SystemHandle,
    /// The system actor's own channel sender — kept alive so
    /// the system actor's mailbox doesn't close prematurely.
    _system_tx: Sender<BoxEnvelope<SystemActor>>,
}

/// Cheap cloneable handle into the system. Holds the address
/// of the internal `SystemActor` — no locks.
#[derive(Clone)]
pub struct SystemHandle {
    system_addr: Addr<SystemActor>,
}

impl System {
    /// Create a new actor system.
    ///
    /// Bootstraps an internal `SystemActor` that owns the actor
    /// registry. Requires an async runtime to be available
    /// (tokio on native, wasm-bindgen-futures on WASM).
    pub fn new() -> Self {
        // Bootstrap: spawn the SystemActor directly (it can't
        // register itself — it IS the registry).
        let (tx, rx) = runtime::channel(DEFAULT_MAILBOX_CAPACITY);
        let addr = Addr::new(tx.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, _done_rx) = runtime::oneshot();

        let handle = SystemHandle {
            system_addr: addr.clone(),
        };

        let ctx = Context::new(addr, tx.clone(), handle.clone(), stop.clone());

        runtime::spawn(mailbox::run_mailbox(
            SystemActor {
                entries: Vec::new(),
            },
            ctx,
            rx,
            stop,
            done_tx,
        ));

        Self {
            handle,
            _system_tx: tx,
        }
    }

    /// Spawn a top-level actor and return its address.
    pub fn spawn<A: Actor>(&self, actor: A) -> Addr<A> {
        self.handle.spawn(actor)
    }

    /// Get a handle that can be passed to other contexts.
    pub fn handle(&self) -> SystemHandle {
        self.handle.clone()
    }

    /// Shut down all actors gracefully.
    ///
    /// Sends `Shutdown` to the system actor (which stops all tracked
    /// actors), then awaits their completion. FIFO ordering guarantees
    /// all prior `Register` messages are processed first.
    pub async fn shutdown(self) {
        let done_rxs = match self.handle.system_addr.ask(Shutdown).await {
            Ok(rxs) => rxs,
            Err(_) => return, // System actor already dead
        };

        for rx in done_rxs {
            let _ = rx.recv().await;
        }
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemHandle {
    /// Spawn a supervised actor with a restart policy.
    pub fn spawn_supervised<A: Actor + Clone>(
        &self,
        actor: A,
        policy: crate::supervisor::RestartPolicy,
    ) -> Addr<A> {
        crate::supervisor::spawn_supervised(actor, policy, self.clone())
    }

    /// Spawn a top-level actor and return its address.
    pub fn spawn<A: Actor>(&self, actor: A) -> Addr<A> {
        self.spawn_with_capacity(actor, DEFAULT_MAILBOX_CAPACITY)
    }

    /// Spawn a top-level actor with a custom mailbox capacity.
    ///
    /// Use this when an actor has different backpressure needs than the
    /// default. Also useful in tests to verify bounded mailbox behavior
    /// with a small capacity.
    pub fn spawn_with_capacity<A: Actor>(&self, actor: A, capacity: usize) -> Addr<A> {
        let (tx, rx) = runtime::channel(capacity);
        let addr = Addr::new(tx.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = runtime::oneshot();

        let ctx = Context::new(addr.clone(), tx.clone(), self.clone(), stop.clone());

        runtime::spawn(mailbox::run_mailbox(actor, ctx, rx, stop.clone(), done_tx));

        // Create a stop signal that sets the flag AND sends a no-op envelope
        // to wake up the mailbox if it's blocked on recv(). If the mailbox
        // is full the noop is dropped, but the stop flag is still set — the
        // actor will notice it on the next message or idle cycle.
        let signal_stop = {
            let stop = stop.clone();
            let tx = tx;
            Box::new(move || {
                stop.store(true, Ordering::SeqCst);
                let noop: BoxEnvelope<A> = Box::new(|_actor, _ctx| Box::pin(async {}));
                let _ = tx.send(noop);
            }) as Box<dyn Fn() + Send>
        };

        // Register for shutdown tracking (fire-and-forget).
        // FIFO ordering guarantees this is processed before any
        // subsequent Shutdown message.
        let _ = self.system_addr.do_send(Register(ActorEntry {
            signal_stop,
            done_rx: Some(done_rx),
        }));

        addr
    }
}
