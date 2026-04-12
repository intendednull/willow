//! Actor context — provides access to the actor's own address, system, and utilities.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_core::Stream;

use crate::actor::{Actor, Handler, Message, StreamHandler};
use crate::addr::Addr;
use crate::envelope::{self, BoxEnvelope};
use crate::runtime::{self, Sender};
use crate::system::SystemHandle;

/// Provided to handlers — gives access to the actor's own address and system.
pub struct Context<A: Actor> {
    addr: Addr<A>,
    tx: Sender<BoxEnvelope<A>>,
    system: SystemHandle,
    stop: Arc<AtomicBool>,
}

impl<A: Actor> Context<A> {
    pub(crate) fn new(
        addr: Addr<A>,
        tx: Sender<BoxEnvelope<A>>,
        system: SystemHandle,
        stop: Arc<AtomicBool>,
    ) -> Self {
        Self {
            addr,
            tx,
            system,
            stop,
        }
    }

    /// Get this actor's own address (for self-sends or passing to children).
    pub fn address(&self) -> Addr<A> {
        self.addr.clone()
    }

    /// Request a graceful stop after the current message finishes.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Access the actor system.
    pub fn system(&self) -> &SystemHandle {
        &self.system
    }

    /// Spawn a new actor on the system.
    pub fn spawn<C: Actor>(&self, child: C) -> Addr<C> {
        self.system.spawn(child)
    }

    /// Attach a stream to this actor. Items are delivered as messages.
    pub fn add_stream<S, St>(&self, stream: St)
    where
        A: StreamHandler<S>,
        S: Send + 'static,
        St: Stream<Item = S> + Send + 'static,
    {
        let tx = self.tx.clone();
        runtime::spawn(async move {
            use std::pin::pin;
            let mut stream = pin!(stream);
            loop {
                let item = std::future::poll_fn(|cx| Stream::poll_next(stream.as_mut(), cx)).await;
                match item {
                    Some(item) => {
                        let envelope = envelope::envelope_stream_item::<A, S>(item);
                        if tx.send(envelope).is_err() {
                            break;
                        }
                    }
                    None => {
                        let envelope = envelope::envelope_stream_finished::<A, S>();
                        tx.send(envelope).ok();
                        break;
                    }
                }
            }
        });
    }

    /// Start a periodic interval. Delivers messages to the actor on each tick.
    ///
    /// Returns a handle that can cancel the interval.
    pub fn run_interval<M>(
        &self,
        duration: Duration,
        msg_factory: impl Fn() -> M + Send + 'static,
    ) -> IntervalHandle
    where
        A: Handler<M>,
        M: Message<Result = ()>,
    {
        let tx = self.tx.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = cancelled.clone();

        runtime::spawn(async move {
            loop {
                runtime::sleep(duration).await;

                if cancelled_clone.load(Ordering::Relaxed) || tx.is_closed() {
                    break;
                }

                let msg = msg_factory();
                let envelope = envelope::envelope_send::<A, M>(msg);
                if tx.send(envelope).is_err() {
                    break;
                }
            }
        });

        IntervalHandle { cancelled }
    }

    /// Schedule a one-shot delayed message. Returns a handle that can cancel delivery.
    pub fn run_after<M>(&self, delay: Duration, msg: M) -> TimerHandle
    where
        A: Handler<M>,
        M: Message<Result = ()>,
    {
        let tx = self.tx.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = cancelled.clone();

        runtime::spawn(async move {
            runtime::sleep(delay).await;

            if cancelled_clone.load(Ordering::Relaxed) || tx.is_closed() {
                return;
            }

            let envelope = envelope::envelope_send::<A, M>(msg);
            tx.send(envelope).ok();
        });

        TimerHandle { cancelled }
    }
}

/// Handle to a one-shot timer. Drop or call `cancel()` to prevent delivery.
pub struct TimerHandle {
    cancelled: Arc<AtomicBool>,
}

impl TimerHandle {
    /// Cancel the timer, preventing the message from being delivered.
    pub fn cancel(self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Drop for TimerHandle {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// Handle to a running interval. Drop or call `cancel()` to stop it.
pub struct IntervalHandle {
    cancelled: Arc<AtomicBool>,
}

impl IntervalHandle {
    /// Cancel the interval.
    pub fn cancel(self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Drop for IntervalHandle {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
