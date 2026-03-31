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
pub use broker::{Broker, BrokerSubscribe, BrokerUnsubscribe, Publish, SubscriptionId};
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
}
