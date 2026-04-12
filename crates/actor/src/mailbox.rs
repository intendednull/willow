//! Actor mailbox — the receive loop that drives an actor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing::trace;

use crate::actor::Actor;
use crate::context::Context;
use crate::envelope::BoxEnvelope;
use crate::runtime::{OneshotTx, Receiver};

/// Run the actor's mailbox loop.
///
/// 1. Call `actor.started()`
/// 2. `recv().await` — blocks until a message arrives
/// 3. Execute the envelope
/// 4. `try_recv()` drain loop — process all queued messages
/// 5. Call `actor.idle()`
/// 6. Check stop flag — if set, exit
/// 7. Go to step 2
///
/// On exit, calls `actor.stopped()` then signals `done`.
pub async fn run_mailbox<A: Actor>(
    mut actor: A,
    mut ctx: Context<A>,
    mut rx: Receiver<BoxEnvelope<A>>,
    stop: Arc<AtomicBool>,
    done: OneshotTx<()>,
) {
    actor.started(&mut ctx).await;
    trace!("actor started");

    loop {
        // Wait for at least one message.
        let Some(envelope) = rx.recv().await else {
            // Channel closed — all senders dropped.
            break;
        };

        // Process the first message.
        envelope(&mut actor, &mut ctx).await;

        // Drain all immediately-available messages.
        while let Some(envelope) = rx.try_recv() {
            envelope(&mut actor, &mut ctx).await;
        }

        // Notify the actor that the queue is empty.
        actor.idle(&mut ctx).await;

        // Check stop flag.
        if stop.load(Ordering::Relaxed) {
            break;
        }
    }

    actor.stopped().await;
    trace!("actor stopped");
    done.send(()).ok();
}
