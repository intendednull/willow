//! Topic-based pub/sub broker actor.
//!
//! [`Broker<T>`] provides push-based message distribution to multiple subscribers.
//! Dead subscribers are automatically pruned on publish.

use std::future::Future;

use crate::actor::{Actor, Handler, Message};
use crate::addr::Recipient;
use crate::context::Context;

// ───── Types ──────────────────────────────────────────────────────────────

/// Unique identifier for a broker subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(pub u64);

/// Publish a message to all subscribers.
pub struct Publish<T: Message<Result = ()> + Clone>(pub T);

impl<T: Message<Result = ()> + Clone> Message for Publish<T> {
    type Result = ();
}

/// Subscribe to receive published messages. Returns a [`SubscriptionId`].
pub struct BrokerSubscribe<T: Message<Result = ()>>(pub Recipient<T>);

impl<T: Message<Result = ()>> Message for BrokerSubscribe<T> {
    type Result = SubscriptionId;
}

/// Fire-and-forget subscribe. Behaves like [`BrokerSubscribe`] but returns
/// `()` so callers in synchronous contexts can use [`crate::Addr::do_send`]
/// to enqueue the subscription without awaiting confirmation.
///
/// Because the broker's mailbox is FIFO, any [`Publish`] enqueued after
/// this call is processed after the subscription is registered — no
/// events are lost as long as no publish was enqueued before this call
/// returns.
pub struct BrokerAttach<T: Message<Result = ()>>(pub Recipient<T>);

impl<T: Message<Result = ()>> Message for BrokerAttach<T> {
    type Result = ();
}

/// Unsubscribe by ID.
pub struct BrokerUnsubscribe(pub SubscriptionId);

impl Message for BrokerUnsubscribe {
    type Result = ();
}

// ───── Broker ─────────────────────────────────────────────────────────────

/// Push-based pub/sub broker for event distribution.
///
/// Useful for events without persistent state (chat messages, connection
/// events, errors). Dead subscribers are automatically pruned.
pub struct Broker<T: Message<Result = ()> + Clone> {
    subscribers: Vec<(SubscriptionId, Recipient<T>)>,
    next_id: u64,
}

impl<T: Message<Result = ()> + Clone> Broker<T> {
    /// Create a new empty broker.
    pub fn new() -> Self {
        Self {
            subscribers: Vec::new(),
            next_id: 0,
        }
    }
}

impl<T: Message<Result = ()> + Clone> Default for Broker<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Message<Result = ()> + Clone> Actor for Broker<T> {}

impl<T: Message<Result = ()> + Clone> Handler<Publish<T>> for Broker<T> {
    fn handle(
        &mut self,
        msg: Publish<T>,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.subscribers
            .retain(|(_, recipient)| recipient.do_send(msg.0.clone()).is_ok());
        async {}
    }
}

impl<T: Message<Result = ()> + Clone> Handler<BrokerSubscribe<T>> for Broker<T> {
    fn handle(
        &mut self,
        msg: BrokerSubscribe<T>,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = SubscriptionId> + Send {
        let id = SubscriptionId(self.next_id);
        self.next_id += 1;
        self.subscribers.push((id, msg.0));
        async move { id }
    }
}

impl<T: Message<Result = ()> + Clone> Handler<BrokerAttach<T>> for Broker<T> {
    fn handle(
        &mut self,
        msg: BrokerAttach<T>,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        let id = SubscriptionId(self.next_id);
        self.next_id += 1;
        self.subscribers.push((id, msg.0));
        async {}
    }
}

impl<T: Message<Result = ()> + Clone> Handler<BrokerUnsubscribe> for Broker<T> {
    fn handle(
        &mut self,
        msg: BrokerUnsubscribe,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.subscribers.retain(|(id, _)| *id != msg.0);
        async {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{runtime, System};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Clone)]
    struct Event(u32);
    impl Message for Event {
        type Result = ();
    }

    struct EventCollector {
        count: Arc<AtomicU32>,
        last: Arc<std::sync::Mutex<Option<u32>>>,
    }

    impl Actor for EventCollector {}

    impl Handler<Event> for EventCollector {
        fn handle(
            &mut self,
            msg: Event,
            _ctx: &mut Context<Self>,
        ) -> impl Future<Output = ()> + Send {
            self.count.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().unwrap() = Some(msg.0);
            async {}
        }
    }

    fn collector(
        system: &System,
    ) -> (
        crate::Addr<EventCollector>,
        Arc<AtomicU32>,
        Arc<std::sync::Mutex<Option<u32>>>,
    ) {
        let count = Arc::new(AtomicU32::new(0));
        let last = Arc::new(std::sync::Mutex::new(None));
        let addr = system.spawn(EventCollector {
            count: count.clone(),
            last: last.clone(),
        });
        (addr, count, last)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_publish_to_subscribers() {
        let system = System::new();
        let broker = system.spawn(Broker::<Event>::new());
        let (c1, count1, _) = collector(&system);
        let (c2, count2, _) = collector(&system);

        broker.ask(BrokerSubscribe(c1.into())).await.unwrap();
        broker.ask(BrokerSubscribe(c2.into())).await.unwrap();

        broker.do_send(Publish(Event(42))).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;

        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_no_subscribers() {
        let system = System::new();
        let broker = system.spawn(Broker::<Event>::new());
        // Should not panic
        broker.do_send(Publish(Event(1))).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_subscribe_returns_id() {
        let system = System::new();
        let broker = system.spawn(Broker::<Event>::new());
        let (c1, _, _) = collector(&system);
        let (c2, _, _) = collector(&system);

        let id1 = broker.ask(BrokerSubscribe(c1.into())).await.unwrap();
        let id2 = broker.ask(BrokerSubscribe(c2.into())).await.unwrap();

        assert_ne!(id1, id2);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_unsubscribe_by_id() {
        let system = System::new();
        let broker = system.spawn(Broker::<Event>::new());
        let (c1, count1, _) = collector(&system);

        let id = broker.ask(BrokerSubscribe(c1.into())).await.unwrap();
        broker.ask(BrokerUnsubscribe(id)).await.unwrap();

        broker.do_send(Publish(Event(1))).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(count1.load(Ordering::SeqCst), 0);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_dead_subscriber_pruned() {
        let system = System::new();
        let broker = system.spawn(Broker::<Event>::new());
        let (c1, _, _) = collector(&system);
        let (c2, count2, _) = collector(&system);

        broker
            .ask(BrokerSubscribe(c1.clone().into()))
            .await
            .unwrap();
        broker.ask(BrokerSubscribe(c2.into())).await.unwrap();

        drop(c1); // Kill first subscriber
        runtime::sleep(Duration::from_millis(20)).await;

        broker.do_send(Publish(Event(1))).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(count2.load(Ordering::SeqCst), 1);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_multiple_publishers() {
        let system = System::new();
        let broker = system.spawn(Broker::<Event>::new());
        let (c1, count1, last1) = collector(&system);

        broker.ask(BrokerSubscribe(c1.into())).await.unwrap();

        broker.do_send(Publish(Event(1))).unwrap();
        broker.do_send(Publish(Event(2))).unwrap();
        broker.do_send(Publish(Event(3))).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;

        assert_eq!(count1.load(Ordering::SeqCst), 3);
        assert_eq!(*last1.lock().unwrap(), Some(3));
        system.shutdown().await;
    }
}
