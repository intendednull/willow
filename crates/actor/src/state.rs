//! Generic state container actor with cheap reads and copy-on-write mutations.
//!
//! [`StateActor<S>`] holds state as `Arc<S>` for zero-copy reads. Mutations use
//! `Arc::make_mut()` for copy-on-write (in-place when sole owner). Subscribers
//! receive batched [`Notify`] messages via the `idle()` hook.
//!
//! [`StateRef<S>`] provides a type-erased, cloneable handle for composing
//! observable actors (works with both `StateActor` and `DerivedActor`).

use std::any::Any;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use crate::actor::{Actor, Handler, Message};
use crate::addr::{Addr, Recipient};
use crate::context::Context;

// ───── Message types ──────────────────────────────────────────────────────

/// Notification sent to subscribers after a state mutation.
#[derive(Clone)]
pub struct Notify;

impl Message for Notify {
    type Result = ();
}

/// Subscribe to state change notifications.
pub struct Subscribe(pub Recipient<Notify>);

impl Message for Subscribe {
    type Result = ();
}

/// Get the current state as a cheap `Arc` clone.
pub struct Get<S>(pub PhantomData<S>);

impl<S: Send + Sync + 'static> Message for Get<S> {
    type Result = Arc<S>;
}

/// Replace the entire state.
pub struct Set<S>(pub S);

impl<S: Send + Sync + 'static> Message for Set<S> {
    type Result = ();
}

/// Type alias for the mutator closure passed to `Mutate`.
pub type MutatorFn = Box<dyn FnOnce(&mut dyn Any) -> Box<dyn Any + Send> + Send>;

/// Read a projection of the state via a closure. The closure receives `&dyn Any`
/// (downcast to `&S` internally) and returns a boxed result.
pub struct Select(pub SelectorFn);

impl Message for Select {
    type Result = Box<dyn Any + Send>;
}

/// Mutate the state via a closure. Uses copy-on-write (`Arc::make_mut`).
/// The closure receives `&mut dyn Any` (downcast to `&mut S` internally).
pub struct Mutate(pub MutatorFn);

impl Message for Mutate {
    type Result = Box<dyn Any + Send>;
}

// ───── StateActor ─────────────────────────────────────────────────────────

/// Generic state container actor.
///
/// Holds state as `Arc<S>` for cheap reads via pointer bumps. Mutations use
/// `Arc::make_mut()` for copy-on-write. Subscriber notifications are batched
/// in `idle()` after the mailbox drains.
pub struct StateActor<S: Send + Sync + 'static> {
    state: Arc<S>,
    dirty: bool,
    subscribers: Vec<Recipient<Notify>>,
}

impl<S: Send + Sync + 'static> StateActor<S> {
    /// Create a new state actor with the given initial value.
    pub fn new(initial: S) -> Self {
        Self {
            state: Arc::new(initial),
            dirty: false,
            subscribers: Vec::new(),
        }
    }
}

impl<S: Send + Sync + 'static> Actor for StateActor<S> {
    fn idle(&mut self, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        if self.dirty {
            self.dirty = false;
            self.subscribers.retain(|r| r.do_send(Notify).is_ok());
        }
        async {}
    }
}

impl<S: Send + Sync + 'static> Handler<Get<S>> for StateActor<S> {
    fn handle(
        &mut self,
        _msg: Get<S>,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = Arc<S>> + Send {
        let arc = Arc::clone(&self.state);
        async move { arc }
    }
}

impl<S: Send + Sync + 'static> Handler<Set<S>> for StateActor<S> {
    fn handle(&mut self, msg: Set<S>, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        self.state = Arc::new(msg.0);
        self.dirty = true;
        async {}
    }
}

impl<S: Send + Sync + 'static> Handler<Select> for StateActor<S> {
    fn handle(
        &mut self,
        msg: Select,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = Box<dyn Any + Send>> + Send {
        let state_ref: &dyn Any = &*self.state;
        let result = (msg.0)(state_ref);
        async move { result }
    }
}

impl<S: Clone + Send + Sync + 'static> Handler<Mutate> for StateActor<S> {
    fn handle(
        &mut self,
        msg: Mutate,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = Box<dyn Any + Send>> + Send {
        let state_mut: &mut S = Arc::make_mut(&mut self.state);
        let state_any: &mut dyn Any = state_mut;
        let result = (msg.0)(state_any);
        self.dirty = true;
        async move { result }
    }
}

impl<S: Send + Sync + 'static> Handler<Subscribe> for StateActor<S> {
    fn handle(
        &mut self,
        msg: Subscribe,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.subscribers.push(msg.0);
        async {}
    }
}

// ───── Typed helpers ──────────────────────────────────────────────────────

/// Get the current state as a cheap `Arc` clone.
pub async fn get<S: Send + Sync + 'static>(addr: &Addr<StateActor<S>>) -> Arc<S> {
    addr.ask(Get(PhantomData)).await.unwrap()
}

/// Read a projection of the state.
pub async fn select<S, T, F>(addr: &Addr<StateActor<S>>, f: F) -> T
where
    S: Send + Sync + 'static,
    T: Send + 'static,
    F: FnOnce(&S) -> T + Send + 'static,
{
    let result = addr
        .ask(Select(Box::new(move |any| {
            let s = any.downcast_ref::<S>().expect("StateActor type mismatch");
            Box::new(f(s)) as Box<dyn Any + Send>
        })))
        .await
        .unwrap();
    *result.downcast::<T>().expect("Select result type mismatch")
}

/// Mutate the state using copy-on-write. Requires `S: Clone`.
pub async fn mutate<S, T, F>(addr: &Addr<StateActor<S>>, f: F) -> T
where
    S: Clone + Send + Sync + 'static,
    T: Send + 'static,
    F: FnOnce(&mut S) -> T + Send + 'static,
{
    let result = addr
        .ask(Mutate(Box::new(move |any| {
            let s = any.downcast_mut::<S>().expect("StateActor type mismatch");
            Box::new(f(s)) as Box<dyn Any + Send>
        })))
        .await
        .unwrap();
    *result.downcast::<T>().expect("Mutate result type mismatch")
}

/// Subscribe an actor to state change notifications.
pub fn subscribe<S, A>(state: &Addr<StateActor<S>>, subscriber: Addr<A>)
where
    S: Send + Sync + 'static,
    A: Handler<Notify>,
{
    let recipient: Recipient<Notify> = subscriber.into();
    state.do_send(Subscribe(recipient)).ok();
}

// ───── StateRef ───────────────────────────────────────────────────────────

/// Closure type for subscribing to notifications.
type SubscribeFn = Arc<dyn Fn(Recipient<Notify>) + Send + Sync>;

/// Closure type for getting the current state.
type GetFn<S> = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Arc<S>> + Send>> + Send + Sync>;

/// Closure type for selecting a projection of state.
type SelectFn = Arc<
    dyn Fn(
            Box<dyn FnOnce(&dyn Any) -> Box<dyn Any + Send> + Send>,
        ) -> Pin<Box<dyn Future<Output = Box<dyn Any + Send>> + Send>>
        + Send
        + Sync,
>;

/// Type alias for the boxed selector closure passed to `Select` and `StateRef::select`.
pub type SelectorFn = Box<dyn FnOnce(&dyn Any) -> Box<dyn Any + Send> + Send>;

/// Type-erased, cloneable handle for observable state actors.
///
/// Works with both [`StateActor`] and [`DerivedActor`](crate::derived::DerivedActor).
/// Enables composition and chaining of reactive state.
pub struct StateRef<S: Send + Sync + 'static> {
    subscribe_fn: SubscribeFn,
    get_fn: GetFn<S>,
    select_fn: SelectFn,
}

impl<S: Send + Sync + 'static> StateRef<S> {
    /// Construct a `StateRef` from raw closures. Used by `DerivedActor` and `FsmActor`.
    pub fn new(subscribe_fn: SubscribeFn, get_fn: GetFn<S>, select_fn: SelectFn) -> Self {
        Self {
            subscribe_fn,
            get_fn,
            select_fn,
        }
    }

    /// Subscribe to state change notifications.
    pub fn subscribe(&self, recipient: Recipient<Notify>) {
        (self.subscribe_fn)(recipient);
    }

    /// Get the current state as a cheap `Arc` clone.
    pub fn get(&self) -> Pin<Box<dyn Future<Output = Arc<S>> + Send>> {
        (self.get_fn)()
    }

    /// Read a projection of the state.
    pub fn select(
        &self,
        f: SelectorFn,
    ) -> Pin<Box<dyn Future<Output = Box<dyn Any + Send>> + Send>> {
        (self.select_fn)(f)
    }
}

impl<S: Send + Sync + 'static> Clone for StateRef<S> {
    fn clone(&self) -> Self {
        Self {
            subscribe_fn: Arc::clone(&self.subscribe_fn),
            get_fn: Arc::clone(&self.get_fn),
            select_fn: Arc::clone(&self.select_fn),
        }
    }
}

impl<S: Send + Sync + 'static> From<&Addr<StateActor<S>>> for StateRef<S> {
    fn from(addr: &Addr<StateActor<S>>) -> Self {
        let addr_sub = addr.clone();
        let addr_get = addr.clone();
        let addr_sel = addr.clone();

        Self {
            subscribe_fn: Arc::new(move |recipient| {
                addr_sub.do_send(Subscribe(recipient)).ok();
            }),
            get_fn: Arc::new(move || {
                let addr = addr_get.clone();
                Box::pin(async move { addr.ask(Get(PhantomData)).await.unwrap() })
            }),
            select_fn: Arc::new(move |f| {
                let addr = addr_sel.clone();
                Box::pin(async move { addr.ask(Select(f)).await.unwrap() })
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{runtime, System};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    // Helper: actor that counts Notify messages
    struct NotifyCounter {
        count: Arc<AtomicU32>,
    }

    impl Actor for NotifyCounter {}

    impl Handler<Notify> for NotifyCounter {
        fn handle(
            &mut self,
            _msg: Notify,
            _ctx: &mut Context<Self>,
        ) -> impl Future<Output = ()> + Send {
            self.count.fetch_add(1, Ordering::SeqCst);
            async {}
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_get_returns_arc() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(42u32));
        let val = get(&addr).await;
        assert_eq!(*val, 42);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_get_arc_shares_identity() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(42u32));
        let a = get(&addr).await;
        let b = get(&addr).await;
        assert!(Arc::ptr_eq(&a, &b));
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_set_updates() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(0u32));
        addr.ask(Set(99u32)).await.unwrap();
        let val = get(&addr).await;
        assert_eq!(*val, 99);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_select_slice() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(vec![1, 2, 3]));
        let len: usize = select(&addr, |v: &Vec<i32>| v.len()).await;
        assert_eq!(len, 3);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_mutate_modifies() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(10u32));
        mutate(&addr, |v: &mut u32| *v += 5).await;
        let val = get(&addr).await;
        assert_eq!(*val, 15);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_mutate_returns_value() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(10u32));
        let old: u32 = mutate(&addr, |v: &mut u32| {
            let old = *v;
            *v = 20;
            old
        })
        .await;
        assert_eq!(old, 10);
        assert_eq!(*get(&addr).await, 20);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_mutate_cow_clones_when_held() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(1u32));
        let held = get(&addr).await; // refcount = 2
        mutate(&addr, |v: &mut u32| *v = 2).await;
        assert_eq!(*held, 1); // old Arc unchanged
        assert_eq!(*get(&addr).await, 2);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_mutate_inplace_when_sole() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(1u32));
        // Don't hold a reference — actor is sole owner
        mutate(&addr, |v: &mut u32| *v = 2).await;
        assert_eq!(*get(&addr).await, 2);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_subscribe_notifies() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(0u32));
        let count = Arc::new(AtomicU32::new(0));
        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        subscribe(&addr, counter);
        addr.ask(Set(1u32)).await.unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        assert!(count.load(Ordering::SeqCst) >= 1);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_no_notify_without_mutation() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(0u32));
        let count = Arc::new(AtomicU32::new(0));
        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        subscribe(&addr, counter);
        // Just read, no mutation
        let _ = get(&addr).await;
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_batch_notifications() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(0u32));
        let count = Arc::new(AtomicU32::new(0));
        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        subscribe(&addr, counter);
        // Rapid mutations — should batch into fewer notifications
        for i in 0..10u32 {
            addr.ask(Set(i)).await.unwrap();
        }
        runtime::sleep(Duration::from_millis(50)).await;
        let notifications = count.load(Ordering::SeqCst);
        assert!(
            notifications > 0 && notifications <= 10,
            "got {notifications} notifications"
        );
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_dead_subscribers_pruned() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(0u32));
        let count = Arc::new(AtomicU32::new(0));
        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        let counter_addr: Addr<NotifyCounter> = counter.clone();
        subscribe(&addr, counter);
        // Kill the subscriber
        drop(counter_addr);
        runtime::sleep(Duration::from_millis(20)).await;
        // Mutate — dead subscriber should be pruned without panic
        addr.ask(Set(1u32)).await.unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_multiple_subscribers() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(0u32));
        let count1 = Arc::new(AtomicU32::new(0));
        let count2 = Arc::new(AtomicU32::new(0));
        let c1 = system.spawn(NotifyCounter {
            count: count1.clone(),
        });
        let c2 = system.spawn(NotifyCounter {
            count: count2.clone(),
        });
        subscribe(&addr, c1);
        subscribe(&addr, c2);
        addr.ask(Set(1u32)).await.unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        assert!(count1.load(Ordering::SeqCst) >= 1);
        assert!(count2.load(Ordering::SeqCst) >= 1);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_ref_from_state_actor() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(42u32));
        let state_ref = StateRef::from(&addr);
        let val = state_ref.get().await;
        assert_eq!(*val, 42);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_ref_clone() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(42u32));
        let state_ref = StateRef::from(&addr);
        let cloned = state_ref.clone();
        let val = cloned.get().await;
        assert_eq!(*val, 42);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_ref_get() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(vec![1, 2, 3]));
        let state_ref = StateRef::from(&addr);
        let val = state_ref.get().await;
        assert_eq!(*val, vec![1, 2, 3]);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_ref_select() {
        let system = System::new();
        let addr = system.spawn(StateActor::new(vec![1, 2, 3]));
        let state_ref = StateRef::from(&addr);
        let result = state_ref
            .select(Box::new(|any| {
                let v = any.downcast_ref::<Vec<i32>>().unwrap();
                Box::new(v.len()) as Box<dyn Any + Send>
            }))
            .await;
        let len = *result.downcast::<usize>().unwrap();
        assert_eq!(len, 3);
        system.shutdown().await;
    }
}
