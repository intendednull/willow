//! # willow-actor
//!
//! Lightweight actor framework for Willow — dual-target (native + WASM).
//!
//! ## Core primitives
//!
//! [`Actor`], [`Handler`], [`StreamHandler`], typed [`Addr`] handles,
//! request-reply via [`ask()`](Addr::ask), supervision, intervals, and stream
//! attachment. Uses tokio on native, futures-channel + gloo-timers on WASM.
//!
//! ## State management
//!
//! - [`StateActor<S>`] — generic state container with `Arc`-based cheap reads
//!   and copy-on-write mutations
//! - [`StateRef<S>`] — type-erased, cloneable handle for composing observable actors
//! - [`DerivedActor`] — reactive derived values from one or more source actors
//!
//! ## Patterns
//!
//! - [`Broker<T>`] — topic-based pub/sub with auto-pruning of dead subscribers
//! - [`FsmActor<M>`] — typed finite state machine with transition validation
//! - [`Pool<A>`] — round-robin work distribution across actor clones
//! - [`StreamOutput<T>`] — actor-produced async streams via bounded channels
//! - [`Debounce<M>`] / [`Throttle<M>`] — rate-limiting actors

pub mod actor;
pub mod addr;
pub mod broker;
pub mod context;
pub mod debounce;
pub mod derived;
pub mod envelope;
pub mod error;
pub mod fsm;
pub mod mailbox;
pub mod pool;
pub mod runtime;
pub mod state;
pub mod stream;
pub mod supervisor;
pub mod system;

pub use actor::{Actor, Handler, Message, StreamHandler};
pub use addr::{Addr, AnyAddr, Recipient};
pub use broker::{
    Broker, BrokerAttach, BrokerSubscribe, BrokerUnsubscribe, Publish, SubscriptionId,
};
pub use context::{Context, IntervalHandle, TimerHandle};
pub use debounce::{Debounce, Enqueue, Throttle};
pub use derived::{derived, DeriveSource, DerivedActor};
pub use error::{AskError, SendError};
pub use fsm::{FsmActor, Input, StateMachine, TransitionResult};
pub use pool::Pool;
pub use state::{
    get, mutate, select, subscribe, Get, Mutate, Notify, Select, Set, StateActor, StateRef,
    Subscribe,
};
pub use stream::{OutputStream, StreamOutput, SubscribeStream};
pub use supervisor::RestartPolicy;
pub use system::{System, SystemHandle};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    // ───── Test actor: Counter ─────────────────────────────────────────────

    struct CounterActor {
        count: u32,
        idle_count: Arc<AtomicU32>,
    }

    struct Increment;
    impl Message for Increment {
        type Result = ();
    }

    struct GetCount;
    impl Message for GetCount {
        type Result = u32;
    }

    impl Handler<Increment> for CounterActor {
        fn handle(
            &mut self,
            _msg: Increment,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            self.count += 1;
            async {}
        }
    }

    impl Handler<GetCount> for CounterActor {
        fn handle(
            &mut self,
            _msg: GetCount,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = u32> + Send {
            let c = self.count;
            async move { c }
        }
    }

    impl CounterActor {
        fn new() -> Self {
            Self {
                count: 0,
                idle_count: Arc::new(AtomicU32::new(0)),
            }
        }

        fn new_with_idle(idle_count: Arc<AtomicU32>) -> Self {
            Self {
                count: 0,
                idle_count,
            }
        }
    }

    impl Actor for CounterActor {
        fn idle(
            &mut self,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            self.idle_count.fetch_add(1, Ordering::SeqCst);
            async {}
        }
    }

    // ───── Basic send/ask ──────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_and_ask() {
        let system = System::new();
        let addr = system.spawn(CounterActor::new());

        addr.do_send(Increment).unwrap();
        addr.do_send(Increment).unwrap();
        addr.do_send(Increment).unwrap();

        let count = addr.ask(GetCount).await.unwrap();
        assert_eq!(count, 3);

        system.shutdown().await;
    }

    // ───── Multi-actor communication ───────────────────────────────────────

    struct ForwarderActor {
        target: Addr<CounterActor>,
    }

    impl Actor for ForwarderActor {}

    struct Forward;
    impl Message for Forward {
        type Result = ();
    }

    impl Handler<Forward> for ForwarderActor {
        fn handle(
            &mut self,
            _msg: Forward,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            let target = self.target.clone();
            async move {
                let _ = target.do_send(Increment);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn multi_actor_communication() {
        let system = System::new();
        let counter = system.spawn(CounterActor::new());
        let forwarder = system.spawn(ForwarderActor {
            target: counter.clone(),
        });

        forwarder.do_send(Forward).unwrap();
        forwarder.do_send(Forward).unwrap();

        // Give time for messages to propagate.
        runtime::sleep(Duration::from_millis(50)).await;

        let count = counter.ask(GetCount).await.unwrap();
        assert_eq!(count, 2);

        system.shutdown().await;
    }

    // ───── StreamHandler ───────────────────────────────────────────────────

    struct StreamActor {
        items: Vec<u32>,
        finished: bool,
    }

    impl Actor for StreamActor {}

    struct GetItems;
    impl Message for GetItems {
        type Result = Vec<u32>;
    }

    impl Handler<GetItems> for StreamActor {
        fn handle(
            &mut self,
            _msg: GetItems,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = Vec<u32>> + Send {
            let items = self.items.clone();
            async move { items }
        }
    }

    struct IsFinished;
    impl Message for IsFinished {
        type Result = bool;
    }

    impl Handler<IsFinished> for StreamActor {
        fn handle(
            &mut self,
            _msg: IsFinished,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = bool> + Send {
            let f = self.finished;
            async move { f }
        }
    }

    impl StreamHandler<u32> for StreamActor {
        fn handle_stream_item(
            &mut self,
            item: u32,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            self.items.push(item);
            async {}
        }

        fn stream_finished(
            &mut self,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            self.finished = true;
            async {}
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_handler() {
        let system = System::new();

        // Create actor with a placeholder — we'll start the stream in started().
        struct StreamActorStarter {
            inner: StreamActor,
        }
        impl Actor for StreamActorStarter {
            fn started(
                &mut self,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                ctx.add_stream(futures::stream::iter(vec![1u32, 2, 3, 4, 5]));
                async {}
            }
        }
        impl StreamHandler<u32> for StreamActorStarter {
            fn handle_stream_item(
                &mut self,
                item: u32,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.inner.items.push(item);
                async {}
            }
            fn stream_finished(
                &mut self,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.inner.finished = true;
                async {}
            }
        }

        struct GetItems2;
        impl Message for GetItems2 {
            type Result = (Vec<u32>, bool);
        }
        impl Handler<GetItems2> for StreamActorStarter {
            fn handle(
                &mut self,
                _msg: GetItems2,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = (Vec<u32>, bool)> + Send {
                let items = self.inner.items.clone();
                let f = self.inner.finished;
                async move { (items, f) }
            }
        }

        let addr = system.spawn(StreamActorStarter {
            inner: StreamActor {
                items: vec![],
                finished: false,
            },
        });

        // Wait for stream to be fully processed.
        runtime::sleep(Duration::from_millis(100)).await;

        let (items, finished) = addr.ask(GetItems2).await.unwrap();
        assert_eq!(items, vec![1, 2, 3, 4, 5]);
        assert!(finished);

        system.shutdown().await;
    }

    // ───── Interval ────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn interval_fires() {
        let system = System::new();

        struct TickActor {
            ticks: u32,
            _interval: Option<IntervalHandle>,
        }
        impl Actor for TickActor {
            fn started(
                &mut self,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self._interval = Some(ctx.run_interval(Duration::from_millis(20), || Tick));
                async {}
            }
        }

        struct Tick;
        impl Message for Tick {
            type Result = ();
        }
        impl Handler<Tick> for TickActor {
            fn handle(
                &mut self,
                _msg: Tick,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.ticks += 1;
                async {}
            }
        }

        struct GetTicks;
        impl Message for GetTicks {
            type Result = u32;
        }
        impl Handler<GetTicks> for TickActor {
            fn handle(
                &mut self,
                _msg: GetTicks,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = u32> + Send {
                let t = self.ticks;
                async move { t }
            }
        }

        let addr = system.spawn(TickActor {
            ticks: 0,
            _interval: None,
        });

        runtime::sleep(Duration::from_millis(110)).await;

        let ticks = addr.ask(GetTicks).await.unwrap();
        assert!(ticks >= 3, "expected at least 3 ticks, got {ticks}");

        system.shutdown().await;
    }

    // ───── Idle batching ───────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn idle_batching() {
        let idle_count = Arc::new(AtomicU32::new(0));
        let system = System::new();
        let addr = system.spawn(CounterActor::new_with_idle(idle_count.clone()));

        // Send 10 messages quickly — they should be batched.
        for _ in 0..10 {
            addr.do_send(Increment).unwrap();
        }

        // Wait for processing.
        runtime::sleep(Duration::from_millis(50)).await;

        let count = addr.ask(GetCount).await.unwrap();
        assert_eq!(count, 10);

        // idle() should have been called far fewer times than 10.
        // (Ideally 1-2 times for the batch, plus 1 for the ask.)
        let idles = idle_count.load(Ordering::SeqCst);
        assert!(
            idles < 10,
            "expected idle to batch (called {idles} times, not 10)"
        );

        system.shutdown().await;
    }

    // ───── Shutdown stops all actors ───────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_stops_actors() {
        let system = System::new();
        let addr = system.spawn(CounterActor::new());

        assert!(addr.is_alive());
        system.shutdown().await;
        for _ in 0..10 {
            if !addr.is_alive() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert!(!addr.is_alive());
    }

    // ───── Recipient ───────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recipient_type_erased_send() {
        let system = System::new();
        let addr = system.spawn(CounterActor::new());

        let recipient: Recipient<Increment> = addr.clone().into();
        recipient.do_send(Increment).unwrap();
        recipient.do_send(Increment).unwrap();

        runtime::sleep(Duration::from_millis(50)).await;

        let count = addr.ask(GetCount).await.unwrap();
        assert_eq!(count, 2);

        system.shutdown().await;
    }

    // ───── Actor stops itself ──────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actor_self_stop() {
        struct SelfStopper;
        impl Actor for SelfStopper {}

        struct StopNow;
        impl Message for StopNow {
            type Result = ();
        }
        impl Handler<StopNow> for SelfStopper {
            fn handle(
                &mut self,
                _msg: StopNow,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                ctx.stop();
                async {}
            }
        }

        let system = System::new();
        let addr = system.spawn(SelfStopper);

        addr.do_send(StopNow).unwrap();

        runtime::sleep(Duration::from_millis(50)).await;
        assert!(!addr.is_alive());

        system.shutdown().await;
    }

    // ───── run_after / TimerHandle ───────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_after_fires() {
        struct TimerActor {
            fired: bool,
            _handle: Option<TimerHandle>,
        }
        impl Actor for TimerActor {
            fn started(
                &mut self,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self._handle = Some(ctx.run_after(Duration::from_millis(20), Fire));
                async {}
            }
        }
        struct Fire;
        impl Message for Fire {
            type Result = ();
        }
        impl Handler<Fire> for TimerActor {
            fn handle(
                &mut self,
                _msg: Fire,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.fired = true;
                async {}
            }
        }
        struct DidFire;
        impl Message for DidFire {
            type Result = bool;
        }
        impl Handler<DidFire> for TimerActor {
            fn handle(
                &mut self,
                _msg: DidFire,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = bool> + Send {
                let f = self.fired;
                async move { f }
            }
        }

        let system = System::new();
        let addr = system.spawn(TimerActor {
            fired: false,
            _handle: None,
        });
        runtime::sleep(Duration::from_millis(50)).await;
        assert!(addr.ask(DidFire).await.unwrap());
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_after_cancel_prevents_delivery() {
        struct TimerActor {
            fired: bool,
            handle: Option<TimerHandle>,
        }
        impl Actor for TimerActor {
            fn started(
                &mut self,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.handle = Some(ctx.run_after(Duration::from_millis(20), Fire));
                // Cancel immediately
                self.handle.take();
                async {}
            }
        }
        struct Fire;
        impl Message for Fire {
            type Result = ();
        }
        impl Handler<Fire> for TimerActor {
            fn handle(
                &mut self,
                _msg: Fire,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.fired = true;
                async {}
            }
        }
        struct DidFire;
        impl Message for DidFire {
            type Result = bool;
        }
        impl Handler<DidFire> for TimerActor {
            fn handle(
                &mut self,
                _msg: DidFire,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = bool> + Send {
                let f = self.fired;
                async move { f }
            }
        }

        let system = System::new();
        let addr = system.spawn(TimerActor {
            fired: false,
            handle: None,
        });
        runtime::sleep(Duration::from_millis(50)).await;
        assert!(!addr.ask(DidFire).await.unwrap());
        system.shutdown().await;
    }

    // ───── bounded_channel ────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bounded_channel_respects_capacity() {
        let (tx, _rx) = runtime::bounded_channel::<u32>(2);
        assert!(tx.try_send(1).is_ok());
        assert!(tx.try_send(2).is_ok());
        // Channel full — should fail
        assert!(tx.try_send(3).is_err());
    }

    // ───── Bounded mailbox (issue #78) ───────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mailbox_drops_messages_when_capacity_exceeded() {
        // Issue #78: Actor mailboxes must be bounded to prevent OOM DoS.
        // With unbounded channels, send() would never return Err due to
        // capacity — only if the receiver is closed. With bounded channels,
        // send() returns Err when the mailbox is full.

        // Test directly at the channel level — this is deterministic.
        let (tx, _rx) = runtime::channel::<u32>(4);
        assert!(tx.send(1).is_ok());
        assert!(tx.send(2).is_ok());
        assert!(tx.send(3).is_ok());
        assert!(tx.send(4).is_ok());
        // Channel is full — this should fail.
        assert!(
            tx.send(5).is_err(),
            "expected mailbox full error, but send succeeded (unbounded!)"
        );

        // Now test via actor spawn_with_capacity — use a slow actor to
        // ensure we can fill the mailbox.
        struct SlowActor;
        impl Actor for SlowActor {}

        struct SlowMsg;
        impl Message for SlowMsg {
            type Result = ();
        }
        impl Handler<SlowMsg> for SlowActor {
            async fn handle(&mut self, _msg: SlowMsg, _ctx: &mut Context<Self>) {
                runtime::sleep(Duration::from_millis(200)).await;
            }
        }

        let system = System::new();
        let addr = system.handle().spawn_with_capacity(SlowActor, 4);

        // Send one message to trigger processing (actor will be busy 200ms).
        addr.do_send(SlowMsg).unwrap();
        // Let the actor dequeue it and begin handling.
        runtime::sleep(Duration::from_millis(30)).await;

        // Now fill the remaining mailbox capacity (4 slots).
        for _ in 0..4 {
            let _ = addr.do_send(SlowMsg);
        }

        // The mailbox should now be full — next send should fail.
        let result = addr.do_send(SlowMsg);
        assert!(
            result.is_err(),
            "expected mailbox full error, but send succeeded (unbounded!)"
        );

        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_succeeds_with_full_mailbox() {
        // Ensure shutdown doesn't deadlock when a mailbox is full.
        // The stop flag (AtomicBool) is set even if the noop wake-up
        // message is dropped due to a full mailbox.
        struct SlowActor;
        impl Actor for SlowActor {}

        struct Block;
        impl Message for Block {
            type Result = ();
        }
        impl Handler<Block> for SlowActor {
            async fn handle(&mut self, _msg: Block, _ctx: &mut Context<Self>) {
                runtime::sleep(Duration::from_millis(50)).await;
            }
        }

        let system = System::new();
        let addr = system.handle().spawn_with_capacity(SlowActor, 2);

        // Fill the mailbox.
        for _ in 0..4 {
            let _ = addr.do_send(Block);
        }

        // Shutdown should complete even though the noop wake-up may
        // be dropped due to the full mailbox.
        let shutdown = tokio::time::timeout(Duration::from_secs(5), system.shutdown());
        assert!(
            shutdown.await.is_ok(),
            "shutdown deadlocked with full mailbox"
        );
    }

    // ───── Supervision ─────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervised_actor_restarts() {
        #[derive(Clone)]
        struct RestartableActor {
            started_count: Arc<AtomicU32>,
        }

        impl Actor for RestartableActor {
            fn started(
                &mut self,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.started_count.fetch_add(1, Ordering::SeqCst);
                async {}
            }
        }

        struct StopMe;
        impl Message for StopMe {
            type Result = ();
        }
        impl Handler<StopMe> for RestartableActor {
            fn handle(
                &mut self,
                _msg: StopMe,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                ctx.stop();
                async {}
            }
        }

        let system = System::new();
        let started = Arc::new(AtomicU32::new(0));
        let actor = RestartableActor {
            started_count: started.clone(),
        };

        let addr = system
            .handle()
            .spawn_supervised(actor, RestartPolicy::OnFailure { max: 3 });

        // Stop the actor — should restart.
        addr.do_send(StopMe).unwrap();
        runtime::sleep(Duration::from_millis(100)).await;

        // Should have started at least twice (initial + 1 restart).
        let count = started.load(Ordering::SeqCst);
        assert!(count >= 2, "expected >= 2 starts, got {count}");

        system.shutdown().await;
    }

    // ───── Supervision: backoff policy ────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervised_actor_backoff_restarts() {
        #[derive(Clone)]
        struct BackoffActor {
            started_count: Arc<AtomicU32>,
        }
        impl Actor for BackoffActor {
            fn started(
                &mut self,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.started_count.fetch_add(1, Ordering::SeqCst);
                async {}
            }
        }
        struct StopMe;
        impl Message for StopMe {
            type Result = ();
        }
        impl Handler<StopMe> for BackoffActor {
            fn handle(
                &mut self,
                _msg: StopMe,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                ctx.stop();
                async {}
            }
        }

        let system = System::new();
        let started = Arc::new(AtomicU32::new(0));
        let actor = BackoffActor {
            started_count: started.clone(),
        };

        let addr = system.handle().spawn_supervised(
            actor,
            RestartPolicy::Backoff {
                initial: Duration::from_millis(10),
                max_delay: Duration::from_millis(50),
                max_retries: 3,
            },
        );

        // Stop the actor — should restart with backoff.
        addr.do_send(StopMe).unwrap();
        runtime::sleep(Duration::from_millis(200)).await;

        let count = started.load(Ordering::SeqCst);
        assert!(count >= 2, "expected >= 2 starts with backoff, got {count}");

        system.shutdown().await;
    }

    // ───── Supervision: Never policy does not restart ─────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervised_never_policy_no_restart() {
        #[derive(Clone)]
        struct NeverRestartActor {
            started_count: Arc<AtomicU32>,
        }
        impl Actor for NeverRestartActor {
            fn started(
                &mut self,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.started_count.fetch_add(1, Ordering::SeqCst);
                async {}
            }
        }
        struct StopMe;
        impl Message for StopMe {
            type Result = ();
        }
        impl Handler<StopMe> for NeverRestartActor {
            fn handle(
                &mut self,
                _msg: StopMe,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                ctx.stop();
                async {}
            }
        }

        let system = System::new();
        let started = Arc::new(AtomicU32::new(0));
        let actor = NeverRestartActor {
            started_count: started.clone(),
        };

        let addr = system
            .handle()
            .spawn_supervised(actor, RestartPolicy::Never);

        addr.do_send(StopMe).unwrap();
        runtime::sleep(Duration::from_millis(100)).await;

        // Started exactly once, then stopped permanently.
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert!(!addr.is_alive());

        system.shutdown().await;
    }

    // ───── Supervision: max restarts enforced ─────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervised_max_restarts_enforced() {
        #[derive(Clone)]
        struct MaxRestartActor {
            started_count: Arc<AtomicU32>,
        }
        impl Actor for MaxRestartActor {
            fn started(
                &mut self,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.started_count.fetch_add(1, Ordering::SeqCst);
                async {}
            }
        }
        struct StopMe;
        impl Message for StopMe {
            type Result = ();
        }
        impl Handler<StopMe> for MaxRestartActor {
            fn handle(
                &mut self,
                _msg: StopMe,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                ctx.stop();
                async {}
            }
        }

        let system = System::new();
        let started = Arc::new(AtomicU32::new(0));
        let actor = MaxRestartActor {
            started_count: started.clone(),
        };

        let addr = system
            .handle()
            .spawn_supervised(actor, RestartPolicy::OnFailure { max: 2 });

        // Stop 3 times to exhaust max restarts.
        for _ in 0..3 {
            let _ = addr.do_send(StopMe);
            runtime::sleep(Duration::from_millis(50)).await;
        }

        runtime::sleep(Duration::from_millis(100)).await;

        // 1 initial + 2 restarts = 3 total.
        let count = started.load(Ordering::SeqCst);
        assert_eq!(
            count, 3,
            "expected 3 starts (1 initial + 2 restarts), got {count}"
        );

        system.shutdown().await;
    }

    // ───── stopped() hook is called ───────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stopped_hook_called() {
        struct StoppableActor {
            stopped_flag: Arc<AtomicU32>,
        }
        impl Actor for StoppableActor {
            fn stopped(&mut self) -> impl std::future::Future<Output = ()> + Send {
                self.stopped_flag.fetch_add(1, Ordering::SeqCst);
                async {}
            }
        }
        struct StopMe;
        impl Message for StopMe {
            type Result = ();
        }
        impl Handler<StopMe> for StoppableActor {
            fn handle(
                &mut self,
                _msg: StopMe,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                ctx.stop();
                async {}
            }
        }

        let system = System::new();
        let stopped_flag = Arc::new(AtomicU32::new(0));
        let addr = system.spawn(StoppableActor {
            stopped_flag: stopped_flag.clone(),
        });

        addr.do_send(StopMe).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;

        assert_eq!(stopped_flag.load(Ordering::SeqCst), 1);
        assert!(!addr.is_alive());

        system.shutdown().await;
    }

    // ───── AskError::Closed when actor is dead ────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ask_dead_actor_returns_closed() {
        let system = System::new();
        let addr = system.spawn(CounterActor::new());

        // Stop the actor.
        system.shutdown().await;

        // shutdown() awaits done_rx, which fires when the mailbox loop calls
        // done.send(()) — but the tokio task hasn't yet dropped the Receiver,
        // so the channel's is_closed() can still return false for a brief
        // window. Poll until the channel is truly closed before asserting.
        for _ in 0..200 {
            if !addr.is_alive() {
                break;
            }
            runtime::sleep(Duration::from_millis(1)).await;
        }

        // Trying to ask a dead actor should fail with Closed.
        let result = addr.ask(GetCount).await;
        assert!(matches!(result, Err(AskError::Closed)));
    }

    // ───── SendError returns the message ──────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_dead_actor_returns_send_error() {
        let system = System::new();
        let addr = system.spawn(CounterActor::new());

        system.shutdown().await;

        // Poll until the channel is truly closed (see ask_dead_actor_returns_closed
        // for explanation of the done_rx vs is_closed() gap).
        for _ in 0..200 {
            if !addr.is_alive() {
                break;
            }
            runtime::sleep(Duration::from_millis(1)).await;
        }

        // send() on a dead actor should return the message.
        let result = addr.send(Increment);
        assert!(result.is_err());
        let err = result.unwrap_err();
        // SendError wraps the original message.
        let _recovered: Increment = err.0;
    }

    // ───── do_send on dead actor returns Err ──────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn do_send_dead_actor_returns_err() {
        let system = System::new();
        let addr = system.spawn(CounterActor::new());

        system.shutdown().await;
        for _ in 0..10 {
            if !addr.is_alive() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }

        let result = addr.do_send(Increment);
        assert!(result.is_err());
    }

    // ───── AnyAddr liveness ───────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn any_addr_tracks_liveness() {
        let system = System::new();
        let addr = system.spawn(CounterActor::new());
        let any: AnyAddr = addr.clone().into();

        assert!(any.is_alive());
        system.shutdown().await;
        for _ in 0..10 {
            if !any.is_alive() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert!(!any.is_alive());
    }

    // ───── Recipient on dead actor ────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recipient_dead_actor() {
        let system = System::new();
        let addr = system.spawn(CounterActor::new());
        let recipient: Recipient<Increment> = addr.clone().into();

        assert!(recipient.is_alive());

        system.shutdown().await;

        // Poll until the channel is truly closed (see ask_dead_actor_returns_closed
        // for explanation of the done_rx vs is_closed() gap).
        for _ in 0..200 {
            if !recipient.is_alive() {
                break;
            }
            runtime::sleep(Duration::from_millis(1)).await;
        }

        assert!(!recipient.is_alive());
        assert!(recipient.do_send(Increment).is_err());
    }

    // ───── Context::address returns self-sendable addr ────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn context_address_self_send() {
        struct SelfSender {
            count: u32,
        }
        impl Actor for SelfSender {}
        struct Ping;
        impl Message for Ping {
            type Result = ();
        }
        struct GetSelfCount;
        impl Message for GetSelfCount {
            type Result = u32;
        }
        impl Handler<Ping> for SelfSender {
            fn handle(
                &mut self,
                _msg: Ping,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = ()> + Send {
                self.count += 1;
                if self.count < 3 {
                    let addr = ctx.address();
                    let _ = addr.do_send(Ping);
                }
                async {}
            }
        }
        impl Handler<GetSelfCount> for SelfSender {
            fn handle(
                &mut self,
                _msg: GetSelfCount,
                _ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = u32> + Send {
                let c = self.count;
                async move { c }
            }
        }

        let system = System::new();
        let addr = system.spawn(SelfSender { count: 0 });

        addr.do_send(Ping).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;

        let count = addr.ask(GetSelfCount).await.unwrap();
        assert_eq!(count, 3, "actor should have sent to itself until count=3");

        system.shutdown().await;
    }

    // ───── Context::spawn creates child actor ─────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn context_spawn_child() {
        struct ParentActor;
        impl Actor for ParentActor {}

        struct SpawnChild;
        impl Message for SpawnChild {
            type Result = Addr<CounterActor>;
        }
        impl Handler<SpawnChild> for ParentActor {
            fn handle(
                &mut self,
                _msg: SpawnChild,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = Addr<CounterActor>> + Send {
                let child_addr = ctx.spawn(CounterActor::new());
                async move { child_addr }
            }
        }

        let system = System::new();
        let parent = system.spawn(ParentActor);

        let child_addr = parent.ask(SpawnChild).await.unwrap();
        assert!(child_addr.is_alive());

        child_addr.do_send(Increment).unwrap();
        let count = child_addr.ask(GetCount).await.unwrap();
        assert_eq!(count, 1);

        system.shutdown().await;
    }

    // ───── Shutdown ordering: system waits for children spawned via ctx ────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn system_shutdown_terminates_ctx_spawned_child() {
        // Children spawned via Context::spawn are tracked by the system and
        // must be stopped when the system shuts down. Without this guarantee,
        // child actors leak past their parent.
        struct ParentActor;
        impl Actor for ParentActor {}

        struct SpawnChild;
        impl Message for SpawnChild {
            type Result = Addr<CounterActor>;
        }
        impl Handler<SpawnChild> for ParentActor {
            fn handle(
                &mut self,
                _msg: SpawnChild,
                ctx: &mut Context<Self>,
            ) -> impl std::future::Future<Output = Addr<CounterActor>> + Send {
                let child = ctx.spawn(CounterActor::new());
                async move { child }
            }
        }

        let system = System::new();
        let parent = system.spawn(ParentActor);
        let child = parent.ask(SpawnChild).await.unwrap();

        assert!(parent.is_alive(), "parent must be alive before shutdown");
        assert!(child.is_alive(), "child must be alive before shutdown");

        system.shutdown().await;

        // Poll until both actors' channels are truly closed (see
        // ask_dead_actor_returns_closed for explanation of the done_rx vs
        // is_closed() gap that can cause immediate post-shutdown assertions
        // to race).
        for _ in 0..200 {
            if !parent.is_alive() && !child.is_alive() {
                break;
            }
            runtime::sleep(Duration::from_millis(1)).await;
        }

        // After shutdown(), both parent AND child must be stopped — system
        // is the registry root, ctx.spawn registers with the same system.
        assert!(
            !parent.is_alive(),
            "parent should be stopped after shutdown"
        );
        assert!(!child.is_alive(), "child should be stopped after shutdown");
    }

    // ───── Shutdown waits for in-flight handler to finish ─────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn system_shutdown_awaits_in_flight_handler() {
        // system.shutdown() must not return until each tracked actor has
        // run its mailbox loop to completion. We prove this by having a
        // handler signal "started" via a oneshot, then sleep, then increment
        // a flag. After shutdown() returns, the flag MUST be observed set —
        // proving shutdown awaited the in-flight handler.
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::sync::oneshot;

        struct LongHandlerActor {
            started_tx: Option<oneshot::Sender<()>>,
            finished: Arc<AtomicBool>,
        }
        impl Actor for LongHandlerActor {}

        struct DoWork;
        impl Message for DoWork {
            type Result = ();
        }
        impl Handler<DoWork> for LongHandlerActor {
            async fn handle(&mut self, _msg: DoWork, _ctx: &mut Context<Self>) {
                if let Some(tx) = self.started_tx.take() {
                    let _ = tx.send(());
                }
                runtime::sleep(Duration::from_millis(80)).await;
                self.finished.store(true, Ordering::SeqCst);
            }
        }

        let system = System::new();
        let (tx, rx) = oneshot::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let addr = system.spawn(LongHandlerActor {
            started_tx: Some(tx),
            finished: finished.clone(),
        });

        addr.do_send(DoWork).unwrap();

        // Wait until the handler has started — deterministic, no sleep.
        rx.await.expect("handler must signal started");

        // Now request shutdown. It must wait for the in-flight handler.
        system.shutdown().await;

        // After shutdown returns, the handler must have completed.
        assert!(
            finished.load(Ordering::SeqCst),
            "shutdown returned before in-flight handler finished — \
             parent must wait for child"
        );
    }

    // ───── Broker: delivery to multiple (>2) subscribers ──────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_delivers_to_many_subscribers() {
        // Broker fans out a single Publish to N subscribers. Existing
        // tests only cover 2 subscribers. This validates the core fanout
        // semantics with N=5 and uses an ask round-trip on each subscriber
        // as a deterministic barrier (no sleep-wait for propagation).
        use crate::broker::{Broker, BrokerSubscribe, Publish};

        #[derive(Clone)]
        struct Evt;
        impl Message for Evt {
            type Result = ();
        }

        struct EvtCounter {
            count: Arc<AtomicU32>,
        }
        impl Actor for EvtCounter {}
        impl Handler<Evt> for EvtCounter {
            async fn handle(&mut self, _msg: Evt, _ctx: &mut Context<Self>) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
        }

        // Ping = no-op message used as a FIFO barrier on each subscriber.
        // When `ask(Ping)` resolves, the actor has processed every envelope
        // queued before it — including any prior Publish-fanout deliveries.
        struct Ping;
        impl Message for Ping {
            type Result = ();
        }
        impl Handler<Ping> for EvtCounter {
            async fn handle(&mut self, _msg: Ping, _ctx: &mut Context<Self>) {}
        }

        let system = System::new();
        let broker = system.spawn(Broker::<Evt>::new());

        let n = 5usize;
        let mut subs: Vec<(Addr<EvtCounter>, Arc<AtomicU32>)> = Vec::with_capacity(n);
        for _ in 0..n {
            let count = Arc::new(AtomicU32::new(0));
            let addr = system.spawn(EvtCounter {
                count: count.clone(),
            });
            // ask() ensures the subscription is registered before we publish.
            broker
                .ask(BrokerSubscribe(addr.clone().into()))
                .await
                .unwrap();
            subs.push((addr, count));
        }

        broker.do_send(Publish(Evt)).unwrap();

        // Barrier strategy:
        // 1. ask() the broker to drain its mailbox through the Publish.
        //    Use a fresh dummy subscriber — re-subscribing an existing
        //    one wouldn't change semantics here.
        // 2. ask() each subscriber a Ping; FIFO on the mailbox guarantees
        //    the Evt do_send queued by the broker is processed before
        //    this Ping resolves.
        let dummy_count = Arc::new(AtomicU32::new(0));
        let dummy = system.spawn(EvtCounter {
            count: dummy_count.clone(),
        });
        broker
            .ask(BrokerSubscribe(dummy.clone().into()))
            .await
            .unwrap();
        // dummy subscribed AFTER the Publish, so it should not have
        // received it — sanity check that broker doesn't replay.
        dummy.ask(Ping).await.unwrap();
        assert_eq!(
            dummy_count.load(Ordering::SeqCst),
            0,
            "broker must not replay events to late subscribers"
        );

        for (i, (addr, count)) in subs.iter().enumerate() {
            addr.ask(Ping).await.unwrap();
            assert_eq!(
                count.load(Ordering::SeqCst),
                1,
                "subscriber {i} should have received exactly 1 Evt"
            );
        }

        system.shutdown().await;
    }

    // ───── Broker: slow subscriber doesn't block other deliveries ─────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_slow_subscriber_does_not_block_others() {
        // Broker uses do_send (fire-and-forget) per subscriber, so a slow
        // handler on one subscriber must not delay delivery to another.
        use crate::broker::{Broker, BrokerSubscribe, Publish};
        use tokio::sync::Notify;

        #[derive(Clone)]
        struct Evt;
        impl Message for Evt {
            type Result = ();
        }

        struct SlowSub {
            release: Arc<Notify>,
            received: Arc<AtomicU32>,
        }
        impl Actor for SlowSub {}
        impl Handler<Evt> for SlowSub {
            async fn handle(&mut self, _msg: Evt, _ctx: &mut Context<Self>) {
                // Block until we're explicitly released. Without per-subscriber
                // isolation, this would prevent the fast subscriber from
                // receiving its delivery.
                self.release.notified().await;
                self.received.fetch_add(1, Ordering::SeqCst);
            }
        }

        struct FastSub {
            received: Arc<Notify>,
        }
        impl Actor for FastSub {}
        impl Handler<Evt> for FastSub {
            async fn handle(&mut self, _msg: Evt, _ctx: &mut Context<Self>) {
                self.received.notify_one();
            }
        }

        let system = System::new();
        let broker = system.spawn(Broker::<Evt>::new());

        let slow_release = Arc::new(Notify::new());
        let slow_received = Arc::new(AtomicU32::new(0));
        let slow = system.spawn(SlowSub {
            release: slow_release.clone(),
            received: slow_received.clone(),
        });

        let fast_received = Arc::new(Notify::new());
        let fast = system.spawn(FastSub {
            received: fast_received.clone(),
        });

        broker.ask(BrokerSubscribe(slow.into())).await.unwrap();
        broker.ask(BrokerSubscribe(fast.into())).await.unwrap();

        broker.do_send(Publish(Evt)).unwrap();

        // Fast subscriber must receive without waiting on slow one.
        // Bounded wait is generous (2s) but the actual notify fires
        // within microseconds in practice — no flake.
        tokio::time::timeout(Duration::from_secs(2), fast_received.notified())
            .await
            .expect("fast subscriber should receive while slow is blocked");

        // Slow subscriber is still blocked — release it and confirm.
        assert_eq!(slow_received.load(Ordering::SeqCst), 0);
        slow_release.notify_one();

        // Now slow handler completes.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while slow_received.load(Ordering::SeqCst) == 0 {
            if std::time::Instant::now() >= deadline {
                panic!("slow subscriber never received after release");
            }
            runtime::sleep(Duration::from_millis(5)).await;
        }

        system.shutdown().await;
    }
}
