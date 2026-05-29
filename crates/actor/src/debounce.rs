//! Rate-limiting actors: [`Debounce<M>`] and [`Throttle<M>`].
//!
//! **Debounce**: forwards only the last message after a quiet period.
//! Resets the timer on each new message.
//!
//! **Throttle**: forwards at most one message per interval. The first
//! message is sent immediately; subsequent messages during cooldown are
//! queued (only the latest).

use std::future::Future;
use std::time::Duration;

use crate::actor::{Actor, Handler, Message};
use crate::addr::Recipient;
use crate::context::{Context, TimerHandle};

// ───── Debounce ───────────────────────────────────────────────────────────

/// Wrapper message for sending values to a [`Debounce`] or [`Throttle`] actor.
pub struct Enqueue<M>(pub M);

impl<M: Send + 'static> Message for Enqueue<M> {
    type Result = ();
}

struct Flush;
impl Message for Flush {
    type Result = ();
}

/// Debounce actor: forwards only the last message after a quiet period.
///
/// Send messages via [`Enqueue<M>`]. Each new message resets the timer.
/// The message is forwarded to the target only after `delay` has elapsed
/// with no new messages.
pub struct Debounce<M: Message<Result = ()> + Send + 'static> {
    target: Recipient<M>,
    delay: Duration,
    pending: Option<M>,
    timer: Option<TimerHandle>,
}

impl<M: Message<Result = ()> + Send + 'static> Debounce<M> {
    /// Create a new debounce actor.
    pub fn new(target: Recipient<M>, delay: Duration) -> Self {
        Self {
            target,
            delay,
            pending: None,
            timer: None,
        }
    }
}

impl<M: Message<Result = ()> + Send + 'static> Actor for Debounce<M> {}

impl<M: Message<Result = ()> + Send + 'static> Handler<Enqueue<M>> for Debounce<M> {
    fn handle(
        &mut self,
        msg: Enqueue<M>,
        ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.pending = Some(msg.0);
        // Cancel existing timer explicitly
        if let Some(t) = self.timer.take() {
            t.cancel();
        }
        // Start new timer
        self.timer = Some(ctx.run_after(self.delay, Flush));
        async {}
    }
}

impl<M: Message<Result = ()> + Send + 'static> Handler<Flush> for Debounce<M> {
    fn handle(&mut self, _msg: Flush, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        if let Some(pending) = self.pending.take() {
            self.target.do_send(pending).ok();
        }
        async {}
    }
}

// ───── Throttle ───────────────────────────────────────────────────────────

struct CooldownExpired;
impl Message for CooldownExpired {
    type Result = ();
}

/// Throttle actor: forwards at most one message per interval.
///
/// Send messages via [`Enqueue<M>`]. The first message is forwarded
/// immediately. Subsequent messages during cooldown are queued (only the
/// latest is kept). When cooldown expires, the queued message is forwarded.
pub struct Throttle<M: Message<Result = ()> + Send + 'static> {
    target: Recipient<M>,
    interval: Duration,
    pending: Option<M>,
    cooling_down: bool,
    _timer: Option<TimerHandle>,
}

impl<M: Message<Result = ()> + Send + 'static> Throttle<M> {
    /// Create a new throttle actor.
    pub fn new(target: Recipient<M>, interval: Duration) -> Self {
        Self {
            target,
            interval,
            pending: None,
            cooling_down: false,
            _timer: None,
        }
    }
}

impl<M: Message<Result = ()> + Send + 'static> Actor for Throttle<M> {}

impl<M: Message<Result = ()> + Send + 'static> Handler<Enqueue<M>> for Throttle<M> {
    fn handle(
        &mut self,
        msg: Enqueue<M>,
        ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        if !self.cooling_down {
            self.target.do_send(msg.0).ok();
            self.cooling_down = true;
            self._timer = Some(ctx.run_after(self.interval, CooldownExpired));
        } else {
            self.pending = Some(msg.0);
        }
        async {}
    }
}

impl<M: Message<Result = ()> + Send + 'static> Handler<CooldownExpired> for Throttle<M> {
    fn handle(
        &mut self,
        _msg: CooldownExpired,
        ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.cooling_down = false;
        // Cancel existing timer explicitly
        if let Some(t) = self._timer.take() {
            t.cancel();
        }
        if let Some(pending) = self.pending.take() {
            self.target.do_send(pending).ok();
            self.cooling_down = true;
            self._timer = Some(ctx.run_after(self.interval, CooldownExpired));
        }
        async {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{runtime, System};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    // Poll `condition` every 10 ms until it returns true or `timeout_ms`
    // elapses. Panics with `msg` on timeout.
    //
    // Use this for POSITIVE "X eventually happens" assertions in tests that
    // depend on async delivery — actor timers and message routing can be
    // slower than the test's wall clock under parallel suite load.
    async fn wait_for(condition: impl Fn() -> bool, timeout_ms: u64, msg: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if condition() {
                return;
            }
            if std::time::Instant::now() >= deadline {
                panic!("wait_for timeout after {timeout_ms}ms: {msg}");
            }
            runtime::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[derive(Clone)]
    struct Ping(u32);
    impl Message for Ping {
        type Result = ();
    }

    struct PingCollector {
        count: Arc<AtomicU32>,
        last: Arc<std::sync::Mutex<Option<u32>>>,
    }

    impl Actor for PingCollector {}

    impl Handler<Ping> for PingCollector {
        fn handle(
            &mut self,
            msg: Ping,
            _ctx: &mut Context<Self>,
        ) -> impl Future<Output = ()> + Send {
            self.count.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().unwrap() = Some(msg.0);
            async {}
        }
    }

    fn setup_collector(
        system: &System,
    ) -> (
        crate::Addr<PingCollector>,
        Arc<AtomicU32>,
        Arc<std::sync::Mutex<Option<u32>>>,
    ) {
        let count = Arc::new(AtomicU32::new(0));
        let last = Arc::new(std::sync::Mutex::new(None));
        let addr = system.spawn(PingCollector {
            count: count.clone(),
            last: last.clone(),
        });
        (addr, count, last)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn debounce_single_message() {
        let system = System::new();
        let (collector, count, last) = setup_collector(&system);
        let debounce = system.spawn(Debounce::new(collector.into(), Duration::from_millis(50)));

        debounce.do_send(Enqueue(Ping(1))).unwrap();

        // Wait for the debounce flush to arrive — actor timer + message
        // delivery can be slower than fixed sleeps under parallel suite load.
        wait_for(
            || count.load(Ordering::SeqCst) >= 1,
            2000,
            "debounce should have forwarded exactly one message",
        )
        .await;

        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(*last.lock().unwrap(), Some(1));
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn debounce_rapid_messages() {
        let system = System::new();
        let (collector, count, last) = setup_collector(&system);
        let debounce = system.spawn(Debounce::new(collector.into(), Duration::from_millis(50)));

        for i in 0..5 {
            debounce.do_send(Enqueue(Ping(i))).unwrap();
        }

        // Wait for the single coalesced flush to arrive.
        wait_for(
            || count.load(Ordering::SeqCst) >= 1,
            2000,
            "debounce should have forwarded exactly one coalesced message",
        )
        .await;

        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(*last.lock().unwrap(), Some(4));
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn debounce_timer_reset() {
        let system = System::new();
        let (collector, count, last) = setup_collector(&system);
        let debounce = system.spawn(Debounce::new(collector.into(), Duration::from_millis(60)));

        debounce.do_send(Enqueue(Ping(1))).unwrap();
        // Sleep for half the debounce window, then reset the timer with Ping(2).
        runtime::sleep(Duration::from_millis(30)).await;
        debounce.do_send(Enqueue(Ping(2))).unwrap();
        runtime::sleep(Duration::from_millis(30)).await;
        // Debounce window restarted from Ping(2); 30ms << 60ms — must not fire.
        // This is an intentional timing invariant: fixed sleep is correct here
        // because we're asserting a "not yet" condition within a known timed window.
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "timer should not have fired yet"
        );

        // Wait for the eventual flush via condition-based polling.
        wait_for(
            || count.load(Ordering::SeqCst) >= 1,
            2000,
            "debounce should have forwarded Ping(2) after quiet period",
        )
        .await;

        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(*last.lock().unwrap(), Some(2));
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn debounce_separate_bursts() {
        let system = System::new();
        let (collector, count, _) = setup_collector(&system);
        let debounce = system.spawn(Debounce::new(collector.into(), Duration::from_millis(30)));

        // First burst — wait for it to flush before sending the second burst,
        // so the two bursts are truly separate and each yields exactly one flush.
        debounce.do_send(Enqueue(Ping(1))).unwrap();
        debounce.do_send(Enqueue(Ping(2))).unwrap();
        wait_for(
            || count.load(Ordering::SeqCst) >= 1,
            2000,
            "first burst should have flushed",
        )
        .await;

        // Second burst — wait for its flush too.
        debounce.do_send(Enqueue(Ping(3))).unwrap();
        debounce.do_send(Enqueue(Ping(4))).unwrap();
        wait_for(
            || count.load(Ordering::SeqCst) >= 2,
            2000,
            "second burst should have flushed",
        )
        .await;

        assert_eq!(count.load(Ordering::SeqCst), 2);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn throttle_immediate_first() {
        let system = System::new();
        let (collector, count, last) = setup_collector(&system);
        let throttle = system.spawn(Throttle::new(collector.into(), Duration::from_millis(100)));

        throttle.do_send(Enqueue(Ping(1))).unwrap();

        // Throttle forwards the first message immediately (before any cooldown
        // fires). Wait for it to arrive rather than guessing a fixed sleep.
        wait_for(
            || count.load(Ordering::SeqCst) >= 1,
            2000,
            "throttle should forward the first message immediately",
        )
        .await;

        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(*last.lock().unwrap(), Some(1));
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn throttle_rate_limited() {
        let system = System::new();
        let (collector, count, _) = setup_collector(&system);
        let throttle = system.spawn(Throttle::new(collector.into(), Duration::from_millis(100)));

        throttle.do_send(Enqueue(Ping(1))).unwrap();
        // First message is forwarded immediately. Wait for it so the cooldown
        // window is known to have started before we queue more messages.
        wait_for(
            || count.load(Ordering::SeqCst) >= 1,
            2000,
            "throttle should forward first message",
        )
        .await;

        throttle.do_send(Enqueue(Ping(2))).unwrap();
        throttle.do_send(Enqueue(Ping(3))).unwrap();

        // Cooldown is 100ms. Sleep for 20ms — well inside the cooldown window —
        // and assert count is still 1. This is an intentional timing invariant:
        // the cooldown MUST suppress extra messages during this window. A fixed
        // sleep is correct here because we're asserting a "not yet" condition
        // within a known timed window (20ms << 100ms cooldown).
        runtime::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "throttle should suppress messages during cooldown"
        );
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn throttle_pending_forwarded() {
        let system = System::new();
        let (collector, count, last) = setup_collector(&system);
        let throttle = system.spawn(Throttle::new(collector.into(), Duration::from_millis(50)));

        throttle.do_send(Enqueue(Ping(1))).unwrap();
        // Wait for first forward so we know the cooldown has started before
        // queuing Ping(2).
        wait_for(
            || count.load(Ordering::SeqCst) >= 1,
            2000,
            "throttle should forward first message",
        )
        .await;

        throttle.do_send(Enqueue(Ping(2))).unwrap();

        // After cooldown expires, pending Ping(2) should be forwarded.
        wait_for(
            || count.load(Ordering::SeqCst) >= 2,
            2000,
            "throttle should forward pending message after cooldown",
        )
        .await;

        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(*last.lock().unwrap(), Some(2));
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn throttle_only_latest_pending() {
        let system = System::new();
        let (collector, count, last) = setup_collector(&system);
        let throttle = system.spawn(Throttle::new(collector.into(), Duration::from_millis(50)));

        throttle.do_send(Enqueue(Ping(1))).unwrap();
        // Wait for first forward so cooldown is active before queuing.
        wait_for(
            || count.load(Ordering::SeqCst) >= 1,
            2000,
            "throttle should forward first message",
        )
        .await;

        throttle.do_send(Enqueue(Ping(2))).unwrap();
        throttle.do_send(Enqueue(Ping(3))).unwrap();

        // After cooldown, only the last pending (Ping(3)) should be forwarded.
        wait_for(
            || count.load(Ordering::SeqCst) >= 2,
            2000,
            "throttle should forward only latest pending message after cooldown",
        )
        .await;

        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(*last.lock().unwrap(), Some(3));
        system.shutdown().await;
    }
}
