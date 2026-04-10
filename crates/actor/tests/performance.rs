//! Performance and throughput tests for the actor framework.
//!
//! Run with: `cargo test -p willow-actor --test performance -- --nocapture`
//!
//! Thresholds are set conservatively for debug builds on CI.
//! Release builds should achieve 5-10x higher throughput.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use willow_actor::actor::{Actor, Handler, Message};
use willow_actor::context::Context;
use willow_actor::derived::derived;
use willow_actor::pool::Pool;
use willow_actor::runtime;
use willow_actor::state::{Get, Notify, StateActor, StateRef, Subscribe};
use willow_actor::stream::{StreamOutput, SubscribeStream};
use willow_actor::{mutate, System};

// ───── Helpers ────────────────────────────────────────────────────────────

struct NotifyCounter {
    count: Arc<AtomicU32>,
}

impl Actor for NotifyCounter {}

impl Handler<Notify> for NotifyCounter {
    fn handle(
        &mut self,
        _msg: Notify,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        self.count.fetch_add(1, Ordering::SeqCst);
        async {}
    }
}

// ───── State Actor Throughput ─────────────────────────────────────────────

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_state_actor_throughput() {
    let system = System::new();
    let addr = system.spawn(StateActor::new(0u64));

    let n = 10_000;
    let start = Instant::now();
    for _ in 0..n {
        mutate(&addr, |v: &mut u64| *v += 1).await;
    }
    let elapsed = start.elapsed();
    let ops_per_sec = n as f64 / elapsed.as_secs_f64();

    println!(
        "perf_state_actor_throughput: {n} mutations in {elapsed:.2?} ({ops_per_sec:.0} ops/sec)"
    );
    assert!(
        ops_per_sec > 5_000.0,
        "expected >5k ops/sec, got {ops_per_sec:.0}"
    );
    system.shutdown().await;
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_state_actor_get_throughput() {
    let system = System::new();
    let addr = system.spawn(StateActor::new(42u64));

    let n = 10_000;
    let start = Instant::now();
    for _ in 0..n {
        let _ = addr.ask(Get::<u64>(PhantomData)).await.unwrap();
    }
    let elapsed = start.elapsed();
    let ops_per_sec = n as f64 / elapsed.as_secs_f64();

    println!(
        "perf_state_actor_get_throughput: {n} gets in {elapsed:.2?} ({ops_per_sec:.0} ops/sec)"
    );
    assert!(
        ops_per_sec > 5_000.0,
        "expected >5k ops/sec, got {ops_per_sec:.0}"
    );
    system.shutdown().await;
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_state_actor_select_throughput() {
    let system = System::new();
    let addr = system.spawn(StateActor::new(42u64));

    let n = 10_000;
    let start = Instant::now();
    for _ in 0..n {
        let _: u64 = willow_actor::select(&addr, |v: &u64| *v).await;
    }
    let elapsed = start.elapsed();
    let ops_per_sec = n as f64 / elapsed.as_secs_f64();

    println!(
        "perf_state_actor_select_throughput: {n} selects in {elapsed:.2?} ({ops_per_sec:.0} ops/sec)"
    );
    assert!(
        ops_per_sec > 5_000.0,
        "expected >5k ops/sec, got {ops_per_sec:.0}"
    );
    system.shutdown().await;
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_state_actor_cow_vs_clone() {
    let system = System::new();
    let addr = system.spawn(StateActor::new(0u64));

    // Without outstanding refs (in-place)
    let n = 5_000;
    let start = Instant::now();
    for _ in 0..n {
        mutate(&addr, |v: &mut u64| *v += 1).await;
    }
    let inplace_elapsed = start.elapsed();

    // With outstanding refs (CoW clone each time)
    let _held = addr.ask(Get::<u64>(PhantomData)).await.unwrap();
    let start = Instant::now();
    for _ in 0..n {
        mutate(&addr, |v: &mut u64| *v += 1).await;
    }
    let cow_elapsed = start.elapsed();

    println!(
        "perf_state_actor_cow_vs_clone: in-place {inplace_elapsed:.2?}, CoW {cow_elapsed:.2?} (ratio: {:.2}x)",
        cow_elapsed.as_secs_f64() / inplace_elapsed.as_secs_f64()
    );
    // Informational only — no threshold assertion
    system.shutdown().await;
}

// ───── Stream Output Throughput ───────────────────────────────────────────

struct StreamProducer {
    output: StreamOutput<u32>,
}

impl Actor for StreamProducer {}

struct Emit(u32);
impl Message for Emit {
    type Result = ();
}

impl Handler<Emit> for StreamProducer {
    async fn handle(&mut self, msg: Emit, _ctx: &mut Context<Self>) {
        self.output.emit(msg.0);
    }
}

impl Handler<SubscribeStream<u32>> for StreamProducer {
    async fn handle(
        &mut self,
        _msg: SubscribeStream<u32>,
        _ctx: &mut Context<Self>,
    ) -> willow_actor::stream::OutputStream<u32> {
        self.output.subscribe()
    }
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_stream_output_throughput() {
    let system = System::new();
    let addr = system.spawn(StreamProducer {
        output: StreamOutput::new(),
    });

    let mut stream = addr.ask(SubscribeStream::<u32>::default()).await.unwrap();

    let n = 10_000u32;

    // Produce and consume concurrently — the bounded channel (cap 64) would
    // fill up and drop items if we sent everything before consuming.
    let producer_addr = addr.clone();
    let producer = tokio::spawn(async move {
        for i in 0..n {
            producer_addr.do_send(Emit(i)).unwrap();
            if i % 32 == 0 {
                tokio::task::yield_now().await;
            }
        }
    });

    let start = Instant::now();
    let mut received = 0u32;
    while received < n {
        if stream.next().await.is_some() {
            received += 1;
        }
    }
    let elapsed = start.elapsed();
    producer.await.unwrap();

    let ops_per_sec = n as f64 / elapsed.as_secs_f64();

    println!(
        "perf_stream_output_throughput: {n} items in {elapsed:.2?} ({ops_per_sec:.0} ops/sec)"
    );
    assert!(
        ops_per_sec > 10_000.0,
        "expected >10k ops/sec, got {ops_per_sec:.0}"
    );
    system.shutdown().await;
}

// ───── Pool Throughput ────────────────────────────────────────────────────

#[derive(Clone)]
struct PoolWorker;

impl Actor for PoolWorker {}

struct WorkMsg;
impl Message for WorkMsg {
    type Result = u32;
}

impl Handler<WorkMsg> for PoolWorker {
    async fn handle(&mut self, _msg: WorkMsg, _ctx: &mut Context<Self>) -> u32 {
        42
    }
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_pool_round_robin_throughput() {
    let system = System::new();
    let mut pool = Pool::new(&system.handle(), PoolWorker, 4);

    let n = 10_000;
    let start = Instant::now();
    for _ in 0..n {
        let _ = pool.ask(WorkMsg).await.unwrap();
    }
    let elapsed = start.elapsed();
    let ops_per_sec = n as f64 / elapsed.as_secs_f64();

    println!(
        "perf_pool_round_robin_throughput: {n} asks in {elapsed:.2?} ({ops_per_sec:.0} ops/sec)"
    );
    assert!(
        ops_per_sec > 5_000.0,
        "expected >5k ops/sec, got {ops_per_sec:.0}"
    );
    system.shutdown().await;
}

// ───── Fanout Tests ───────────────────────────────────────────────────────

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_state_notify_fanout() {
    for &n_subs in &[1, 10, 100] {
        let system = System::new();
        let addr = system.spawn(StateActor::new(0u32));
        let counts: Vec<Arc<AtomicU32>> = (0..n_subs)
            .map(|_| {
                let count = Arc::new(AtomicU32::new(0));
                let counter = system.spawn(NotifyCounter {
                    count: count.clone(),
                });
                addr.do_send(Subscribe(counter.into())).unwrap();
                count
            })
            .collect();

        let start = Instant::now();
        addr.ask(willow_actor::Set(1u32)).await.unwrap();
        // Wait for all subscribers to receive
        loop {
            runtime::sleep(Duration::from_millis(1)).await;
            if counts.iter().all(|c| c.load(Ordering::SeqCst) >= 1) {
                break;
            }
        }
        let elapsed = start.elapsed();

        println!("perf_state_notify_fanout({n_subs} subs): {elapsed:.2?}");
        if n_subs == 100 {
            assert!(
                elapsed < Duration::from_millis(200),
                "expected <200ms for 100 subscribers, got {elapsed:.2?}"
            );
        }
        system.shutdown().await;
    }
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_broker_fanout() {
    use willow_actor::broker::{Broker, BrokerSubscribe, Publish};

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
        fn handle(
            &mut self,
            _msg: Evt,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            self.count.fetch_add(1, Ordering::SeqCst);
            async {}
        }
    }

    for &n_subs in &[1, 10, 100] {
        let system = System::new();
        let broker = system.spawn(Broker::<Evt>::new());
        let counts: Vec<Arc<AtomicU32>> = (0..n_subs)
            .map(|_| {
                let count = Arc::new(AtomicU32::new(0));
                let counter = system.spawn(EvtCounter {
                    count: count.clone(),
                });
                let recipient: willow_actor::Recipient<Evt> = counter.into();
                drop(broker.ask(BrokerSubscribe(recipient)));
                count
            })
            .collect();

        runtime::sleep(Duration::from_millis(50)).await;

        let start = Instant::now();
        broker.do_send(Publish(Evt)).unwrap();
        loop {
            runtime::sleep(Duration::from_millis(1)).await;
            if counts.iter().all(|c| c.load(Ordering::SeqCst) >= 1) {
                break;
            }
        }
        let elapsed = start.elapsed();
        println!("perf_broker_fanout({n_subs} subs): {elapsed:.2?}");
        system.shutdown().await;
    }
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_stream_output_multi_consumer() {
    let system = System::new();
    let addr = system.spawn(StreamProducer {
        output: StreamOutput::new(),
    });

    let n_consumers = 10;
    let mut streams: Vec<_> = Vec::new();
    for _ in 0..n_consumers {
        streams.push(addr.ask(SubscribeStream::<u32>::default()).await.unwrap());
    }

    let n = 1_000u32;
    let producer_addr = addr.clone();
    let producer = tokio::spawn(async move {
        for i in 0..n {
            producer_addr.do_send(Emit(i)).unwrap();
            if i % 16 == 0 {
                tokio::task::yield_now().await;
            }
        }
    });

    let start = Instant::now();
    for stream in &mut streams {
        let mut received = 0u32;
        while received < n {
            if stream.next().await.is_some() {
                received += 1;
            }
        }
    }
    let elapsed = start.elapsed();
    producer.await.unwrap();

    let total = n as u64 * n_consumers as u64;
    let ops_per_sec = total as f64 / elapsed.as_secs_f64();
    println!(
        "perf_stream_output_multi_consumer: {total} items ({n_consumers} consumers x {n}) in {elapsed:.2?} ({ops_per_sec:.0} ops/sec)"
    );
    system.shutdown().await;
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_derived_multi_source_snapshot() {
    for n_sources in [2, 4, 6] {
        let system = System::new();
        let mut addrs = Vec::new();
        for i in 0..6 {
            addrs.push(system.spawn(StateActor::new(i as u32)));
        }

        let start = Instant::now();
        match n_sources {
            2 => {
                let r0 = StateRef::from(&addrs[0]);
                let r1 = StateRef::from(&addrs[1]);
                let d = derived(
                    &system.handle(),
                    (r0, r1),
                    |(a, b): &(Arc<u32>, Arc<u32>)| **a + **b,
                );
                runtime::sleep(Duration::from_millis(50)).await;
                let _ = d.get().await;
            }
            4 => {
                let r0 = StateRef::from(&addrs[0]);
                let r1 = StateRef::from(&addrs[1]);
                let r2 = StateRef::from(&addrs[2]);
                let r3 = StateRef::from(&addrs[3]);
                let d = derived(
                    &system.handle(),
                    (r0, r1, r2, r3),
                    |(a, b, c, d): &(Arc<u32>, Arc<u32>, Arc<u32>, Arc<u32>)| **a + **b + **c + **d,
                );
                runtime::sleep(Duration::from_millis(50)).await;
                let _ = d.get().await;
            }
            6 => {
                let r0 = StateRef::from(&addrs[0]);
                let r1 = StateRef::from(&addrs[1]);
                let r2 = StateRef::from(&addrs[2]);
                let r3 = StateRef::from(&addrs[3]);
                let r4 = StateRef::from(&addrs[4]);
                let r5 = StateRef::from(&addrs[5]);
                #[allow(clippy::type_complexity)]
                let d = derived(
                    &system.handle(),
                    (r0, r1, r2, r3, r4, r5),
                    |(a, b, c, d, e, f): &(
                        Arc<u32>,
                        Arc<u32>,
                        Arc<u32>,
                        Arc<u32>,
                        Arc<u32>,
                        Arc<u32>,
                    )| { **a + **b + **c + **d + **e + **f },
                );
                runtime::sleep(Duration::from_millis(50)).await;
                let _ = d.get().await;
            }
            _ => unreachable!(),
        }
        let elapsed = start.elapsed();
        println!("perf_derived_multi_source_snapshot({n_sources} sources): {elapsed:.2?}");
        system.shutdown().await;
    }
}

// ───── Derived Propagation ────────────────────────────────────────────────

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_derived_propagation_latency() {
    let system = System::new();
    let state = system.spawn(StateActor::new(0u32));
    let state_ref = StateRef::from(&state);
    let d = derived(&system.handle(), state_ref, |s: &Arc<u32>| **s * 2);

    runtime::sleep(Duration::from_millis(50)).await;

    let start = Instant::now();
    state.ask(willow_actor::Set(5u32)).await.unwrap();
    // Poll until derived has updated
    loop {
        let val = d.get().await;
        if *val == 10 {
            break;
        }
        runtime::sleep(Duration::from_millis(1)).await;
    }
    let elapsed = start.elapsed();

    println!("perf_derived_propagation_latency: {elapsed:.2?}");
    assert!(
        elapsed < Duration::from_millis(100),
        "expected <100ms, got {elapsed:.2?}"
    );
    system.shutdown().await;
}

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_derived_chain_depth() {
    let system = System::new();
    let state = system.spawn(StateActor::new(1u32));
    let mut current: StateRef<u32> = StateRef::from(&state);

    // Chain of 10 derived actors
    for _ in 0..10 {
        current = derived(&system.handle(), current, |s: &Arc<u32>| **s + 1);
    }

    runtime::sleep(Duration::from_millis(500)).await;

    let start = Instant::now();
    state.ask(willow_actor::Set(1u32)).await.unwrap();
    loop {
        let val = current.get().await;
        if *val == 11 {
            break;
        }
        runtime::sleep(Duration::from_millis(1)).await;
    }
    let elapsed = start.elapsed();

    println!("perf_derived_chain_depth(10 levels): {elapsed:.2?}");
    assert!(
        elapsed < Duration::from_millis(2000),
        "expected <2s for 10-level chain, got {elapsed:.2?}"
    );
    system.shutdown().await;
}

// ───── Batching Efficiency ────────────────────────────────────────────────

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn perf_derived_idle_batching_efficiency() {
    let system = System::new();
    let addr = system.spawn(StateActor::new(0u32));
    let count = Arc::new(AtomicU32::new(0));
    let counter = system.spawn(NotifyCounter {
        count: count.clone(),
    });
    addr.do_send(Subscribe(counter.into())).unwrap();

    let n = 1000u32;
    for i in 0..n {
        addr.do_send(willow_actor::Set(i)).unwrap();
    }
    runtime::sleep(Duration::from_millis(200)).await;

    let notifications = count.load(Ordering::SeqCst);
    let ratio = notifications as f64 / n as f64;
    println!(
        "perf_derived_idle_batching_efficiency: {notifications} notifications for {n} mutations ({:.1}%)",
        ratio * 100.0
    );
    assert!(
        (notifications as f64) < n as f64 * 0.5,
        "expected <50% notification ratio, got {notifications}/{n}"
    );
    system.shutdown().await;
}

// ───── Debounce Overhead ──────────────────────────────────────────────────

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn perf_debounce_overhead() {
    use willow_actor::debounce::{Debounce, Enqueue};

    #[derive(Clone)]
    struct Ping;
    impl Message for Ping {
        type Result = ();
    }

    struct PingReceiver {
        received: Arc<AtomicU32>,
    }
    impl Actor for PingReceiver {}
    impl Handler<Ping> for PingReceiver {
        fn handle(
            &mut self,
            _msg: Ping,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            self.received.fetch_add(1, Ordering::SeqCst);
            async {}
        }
    }

    let system = System::new();
    let received = Arc::new(AtomicU32::new(0));
    let receiver = system.spawn(PingReceiver {
        received: received.clone(),
    });
    let debounce = system.spawn(Debounce::new(receiver.into(), Duration::from_millis(50)));

    let start = Instant::now();
    debounce.do_send(Enqueue(Ping)).unwrap();

    loop {
        if received.load(Ordering::SeqCst) > 0 {
            break;
        }
        runtime::sleep(Duration::from_millis(1)).await;
    }
    let elapsed = start.elapsed();

    println!("perf_debounce_overhead: {elapsed:.2?} (50ms delay + overhead)");
    assert!(
        elapsed < Duration::from_millis(100),
        "expected <100ms (50ms delay + overhead), got {elapsed:.2?}"
    );
    system.shutdown().await;
}

// ───── Memory (informational) ─────────────────────────────────────────────

#[ignore] // Run explicitly via `just test-actor-perf`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn perf_state_actor_memory() {
    let system = System::new();
    let addr = system.spawn(StateActor::new(0u32));

    for _ in 0..100 {
        let count = Arc::new(AtomicU32::new(0));
        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        addr.do_send(Subscribe(counter.into())).unwrap();
    }

    println!(
        "perf_state_actor_memory: StateActor with 100 subscribers created (informational — no threshold)"
    );
    system.shutdown().await;
}
