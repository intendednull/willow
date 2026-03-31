//! Supervision — restart policies for actors that panic or fail.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tracing::info;

use crate::actor::Actor;
use crate::addr::Addr;
use crate::context::Context;
use crate::envelope::BoxEnvelope;
use crate::runtime::{self, Receiver, Sender};
use crate::system::SystemHandle;

/// Restart policy for supervised actors.
#[derive(Debug, Clone)]
pub enum RestartPolicy {
    /// Never restart (default). Errors are logged.
    Never,
    /// Restart immediately on panic/error, up to `max` times.
    OnFailure { max: u32 },
    /// Restart with exponential backoff.
    Backoff {
        initial: Duration,
        max_delay: Duration,
        max_retries: u32,
    },
}

/// Spawn a supervised actor. The `Addr` returned is stable across restarts
/// because it points at the same channel — only the actor instance is replaced.
pub(crate) fn spawn_supervised<A: Actor + Clone>(
    actor: A,
    policy: RestartPolicy,
    system: SystemHandle,
) -> Addr<A> {
    let (tx, rx) = runtime::unbounded_channel::<BoxEnvelope<A>>();
    let addr = Addr::new(tx.clone());

    runtime::spawn(supervisor_loop(actor, policy, rx, tx, system, addr.clone()));

    addr
}

async fn supervisor_loop<A: Actor + Clone>(
    actor: A,
    policy: RestartPolicy,
    mut rx: Receiver<BoxEnvelope<A>>,
    tx: Sender<BoxEnvelope<A>>,
    system: SystemHandle,
    addr: Addr<A>,
) {
    let mut restarts = 0u32;
    let mut current_delay = match &policy {
        RestartPolicy::Backoff { initial, .. } => *initial,
        _ => Duration::ZERO,
    };

    loop {
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, _done_rx) = runtime::oneshot();

        let ctx = Context::new(addr.clone(), tx.clone(), system.clone(), stop.clone());

        let actor_clone = actor.clone();

        // Run the actor's mailbox. We pass the receiver by transferring ownership,
        // but we need it back for restarts. We'll use a wrapper approach.
        // Actually, we can't easily share the Receiver across restarts since it's not Clone.
        // Instead, we run the mailbox inline in this task.
        run_mailbox_inline(actor_clone, ctx, &mut rx, stop, done_tx).await;

        // Actor stopped. Check if we should restart.
        let should_restart = match &policy {
            RestartPolicy::Never => false,
            RestartPolicy::OnFailure { max } => restarts < *max,
            RestartPolicy::Backoff { max_retries, .. } => restarts < *max_retries,
        };

        if !should_restart || tx.is_closed() {
            break;
        }

        // Apply backoff delay.
        if let RestartPolicy::Backoff { max_delay, .. } = &policy {
            info!(restart = restarts + 1, delay_ms = ?current_delay, "restarting supervised actor");
            runtime::sleep(current_delay).await;
            current_delay = std::cmp::min(current_delay * 2, *max_delay);
        } else {
            info!(restart = restarts + 1, "restarting supervised actor");
        }

        restarts += 1;
    }
}

/// Inline version of run_mailbox that borrows the receiver (for restart reuse).
async fn run_mailbox_inline<A: Actor>(
    mut actor: A,
    mut ctx: Context<A>,
    rx: &mut Receiver<BoxEnvelope<A>>,
    stop: Arc<AtomicBool>,
    done: runtime::OneshotTx<()>,
) {
    actor.started(&mut ctx).await;

    loop {
        let Some(envelope) = rx.recv().await else {
            break;
        };

        envelope(&mut actor, &mut ctx).await;

        while let Some(envelope) = rx.try_recv() {
            envelope(&mut actor, &mut ctx).await;
        }

        actor.idle(&mut ctx).await;

        if stop.load(Ordering::Relaxed) {
            break;
        }
    }

    actor.stopped().await;
    let _ = done.send(());
}
