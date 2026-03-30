//! Actor system — owns the runtime and tracks all top-level actors.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::actor::Actor;
use crate::addr::Addr;
use crate::context::Context;
use crate::envelope::BoxEnvelope;
use crate::mailbox;
use crate::runtime::{self, OneshotRx};

/// The actor system — tracks all top-level actors.
pub struct System {
    handle: SystemHandle,
}

/// Cheap cloneable handle into the system.
#[derive(Clone)]
pub struct SystemHandle {
    inner: Arc<SystemInner>,
}

struct ActorEntry {
    /// Sets the stop flag on the actor's mailbox.
    signal_stop: Box<dyn Fn() + Send>,
    /// Signals when the actor's mailbox loop exits.
    done_rx: Option<OneshotRx<()>>,
}

struct SystemInner {
    actors: Mutex<Vec<ActorEntry>>,
}

impl System {
    /// Create a new actor system.
    pub fn new() -> Self {
        Self {
            handle: SystemHandle {
                inner: Arc::new(SystemInner {
                    actors: Mutex::new(Vec::new()),
                }),
            },
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
    /// Signals all actors to stop, then waits for them to finish.
    pub async fn shutdown(self) {
        let entries = {
            let mut actors = self.handle.inner.actors.lock().unwrap();
            std::mem::take(&mut *actors)
        };

        // Signal stop on all actors and collect done receivers.
        let mut done_rxs = Vec::new();
        for mut entry in entries {
            (entry.signal_stop)();
            if let Some(rx) = entry.done_rx.take() {
                done_rxs.push(rx);
            }
        }

        // Wait for all actors to finish.
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
        let (tx, rx) = runtime::unbounded_channel();
        let addr = Addr::new(tx.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = runtime::oneshot();

        let ctx = Context::new(addr.clone(), tx.clone(), self.clone(), stop.clone());

        runtime::spawn(mailbox::run_mailbox(actor, ctx, rx, stop.clone(), done_tx));

        // Create a stop signal that sets the flag AND sends a no-op envelope
        // to wake up the mailbox if it's blocked on recv().
        let signal_stop = {
            let stop = stop.clone();
            let tx = tx;
            Box::new(move || {
                stop.store(true, Ordering::SeqCst);
                // Send a no-op envelope to wake up the recv() call.
                let noop: BoxEnvelope<A> = Box::new(|_actor, _ctx| Box::pin(async {}));
                let _ = tx.send(noop);
            }) as Box<dyn Fn() + Send>
        };

        // Track for shutdown.
        {
            let mut actors = self.inner.actors.lock().unwrap();
            actors.push(ActorEntry {
                signal_stop,
                done_rx: Some(done_rx),
            });
        }

        addr
    }
}
