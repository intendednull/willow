//! Reactive derived values from one or more source actors.
//!
//! [`DerivedActor<Src, T>`] subscribes to source actors via [`DeriveSource`],
//! recomputes a derived value when sources change, and notifies its own
//! subscribers only when the value actually changes (via `PartialEq`).
//!
//! Use [`derived()`] for convenient construction returning a [`StateRef<T>`].

use std::any::Any;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use crate::actor::{Actor, Handler, Message};
use crate::addr::{Addr, Recipient};
use crate::context::Context;
use crate::state::{Get, Notify, Select, StateRef, Subscribe};
use crate::system::SystemHandle;

// ───── DeriveSource trait ─────────────────────────────────────────────────

/// Trait for types that can serve as sources for a [`DerivedActor`].
///
/// Implemented for [`StateRef<S>`] (single source) and tuples of `StateRef`s
/// (multi-source, up to arity 6).
pub trait DeriveSource: Clone + Send + 'static {
    /// The snapshot type returned by `snapshot()`.
    type Snapshot: Send + 'static;

    /// Subscribe a recipient to change notifications from all sources.
    fn subscribe_all(&self, recipient: Recipient<Notify>);

    /// Fetch the current snapshot from all sources (returns `Arc`s — cheap).
    fn snapshot(&self) -> Pin<Box<dyn Future<Output = Self::Snapshot> + Send>>;
}

impl<S: Send + Sync + 'static> DeriveSource for StateRef<S> {
    type Snapshot = Arc<S>;

    fn subscribe_all(&self, recipient: Recipient<Notify>) {
        self.subscribe(recipient);
    }

    fn snapshot(&self) -> Pin<Box<dyn Future<Output = Arc<S>> + Send>> {
        self.get()
    }
}

// ───── Tuple macro ────────────────────────────────────────────────────────

macro_rules! impl_derive_source_tuple {
    ( $( $idx:tt : $T:ident / $v:ident ),+ ) => {
        impl< $( $T: Send + Sync + 'static ),+ > DeriveSource for ( $( StateRef<$T>, )+ ) {
            type Snapshot = ( $( Arc<$T>, )+ );

            fn subscribe_all(&self, recipient: Recipient<Notify>) {
                $( self.$idx.subscribe(recipient.clone()); )+
            }

            fn snapshot(&self) -> Pin<Box<dyn Future<Output = Self::Snapshot> + Send>> {
                $( let $v = self.$idx.clone(); )+
                Box::pin(async move {
                    let ( $( $v, )+ ) = futures::join!( $( $v.get(), )+ );
                    ( $( $v, )+ )
                })
            }
        }
    };
}

impl_derive_source_tuple!(0: S0/s0, 1: S1/s1);
impl_derive_source_tuple!(0: S0/s0, 1: S1/s1, 2: S2/s2);
impl_derive_source_tuple!(0: S0/s0, 1: S1/s1, 2: S2/s2, 3: S3/s3);
impl_derive_source_tuple!(0: S0/s0, 1: S1/s1, 2: S2/s2, 3: S3/s3, 4: S4/s4);
impl_derive_source_tuple!(0: S0/s0, 1: S1/s1, 2: S2/s2, 3: S3/s3, 4: S4/s4, 5: S5/s5);

// ───── DerivedActor ───────────────────────────────────────────────────────

/// Internal message to update the cached derived value.
struct UpdateCache<T>(T);

impl<T: Send + 'static> Message for UpdateCache<T> {
    type Result = ();
}

/// Reactive derived actor that computes a value from one or more source actors.
///
/// Subscribes to sources on start, recomputes when notified, and only
/// propagates to its own subscribers when the derived value changes.
#[allow(clippy::type_complexity)]
pub struct DerivedActor<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> {
    sources: Src,
    selector: Arc<dyn Fn(&Src::Snapshot) -> T + Send + Sync>,
    cached: Option<Arc<T>>,
    subscribers: Vec<Recipient<Notify>>,
    dirty: bool,
}

impl<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> DerivedActor<Src, T> {
    /// Create a new derived actor with the given sources and selector function.
    pub fn new(
        sources: Src,
        selector: impl Fn(&Src::Snapshot) -> T + Send + Sync + 'static,
    ) -> Self {
        Self {
            sources,
            selector: Arc::new(selector),
            cached: None,
            subscribers: Vec::new(),
            dirty: false,
        }
    }
}

impl<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> Actor for DerivedActor<Src, T> {
    fn started(&mut self, ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        // Subscribe to all sources
        let recipient: Recipient<Notify> = ctx.address().into();
        self.sources.subscribe_all(recipient);

        // Initial computation
        let sources = self.sources.clone();
        let selector = Arc::clone(&self.selector);
        let addr = ctx.address();
        crate::runtime::spawn(async move {
            let snapshot = sources.snapshot().await;
            let value = selector(&snapshot);
            addr.do_send(UpdateCache(value)).ok();
        });

        async {}
    }

    fn idle(&mut self, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        if self.dirty {
            self.dirty = false;
            self.subscribers.retain(|r| r.do_send(Notify).is_ok());
        }
        async {}
    }
}

impl<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> Handler<Notify>
    for DerivedActor<Src, T>
{
    fn handle(&mut self, _msg: Notify, ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        let sources = self.sources.clone();
        let selector = Arc::clone(&self.selector);
        let addr = ctx.address();
        crate::runtime::spawn(async move {
            let snapshot = sources.snapshot().await;
            let value = selector(&snapshot);
            addr.do_send(UpdateCache(value)).ok();
        });
        async {}
    }
}

impl<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> Handler<UpdateCache<T>>
    for DerivedActor<Src, T>
{
    fn handle(
        &mut self,
        msg: UpdateCache<T>,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        let changed = match &self.cached {
            Some(cached) => **cached != msg.0,
            None => true,
        };
        if changed {
            self.cached = Some(Arc::new(msg.0));
            self.dirty = true;
        }
        async {}
    }
}

impl<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> Handler<Subscribe>
    for DerivedActor<Src, T>
{
    fn handle(
        &mut self,
        msg: Subscribe,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.subscribers.push(msg.0);
        async {}
    }
}

impl<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> Handler<Select>
    for DerivedActor<Src, T>
{
    fn handle(
        &mut self,
        msg: Select,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = Box<dyn Any + Send>> + Send {
        let cached = self.cached.clone();
        let result = match cached {
            Some(arc) => (msg.0)(&*arc as &dyn Any),
            None => Box::new(()) as Box<dyn Any + Send>,
        };
        async move { result }
    }
}

impl<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> Handler<Get<T>>
    for DerivedActor<Src, T>
{
    fn handle(
        &mut self,
        _msg: Get<T>,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = Arc<T>> + Send {
        if let Some(cached) = self.cached.clone() {
            return futures::future::Either::Left(async move { cached });
        }
        // Value not yet computed — fetch snapshot and compute synchronously
        let sources = self.sources.clone();
        let selector = Arc::clone(&self.selector);
        futures::future::Either::Right(async move {
            let snapshot = sources.snapshot().await;
            let value = selector(&snapshot);
            Arc::new(value)
        })
    }
}

// ───── StateRef for DerivedActor ──────────────────────────────────────────

impl<Src: DeriveSource, T: PartialEq + Send + Sync + 'static> From<&Addr<DerivedActor<Src, T>>>
    for StateRef<T>
{
    fn from(addr: &Addr<DerivedActor<Src, T>>) -> Self {
        let addr_sub = addr.clone();
        let addr_get = addr.clone();
        let addr_sel = addr.clone();

        StateRef::new(
            Arc::new(move |recipient| {
                addr_sub.do_send(Subscribe(recipient)).ok();
            }),
            Arc::new(move || {
                let addr = addr_get.clone();
                Box::pin(async move { addr.ask(Get(PhantomData)).await.unwrap() })
            }),
            Arc::new(move |f| {
                let addr = addr_sel.clone();
                Box::pin(async move { addr.ask(Select(f)).await.unwrap() })
            }),
        )
    }
}

// ───── Convenience constructor ────────────────────────────────────────────

/// Create a derived actor and return a [`StateRef<T>`] handle.
///
/// Spawns the actor on the given system, subscribes to sources, and returns
/// a type-erased handle suitable for further composition.
pub fn derived<Src, T>(
    system: &SystemHandle,
    sources: Src,
    selector: impl Fn(&Src::Snapshot) -> T + Send + Sync + 'static,
) -> StateRef<T>
where
    Src: DeriveSource,
    T: PartialEq + Send + Sync + 'static,
{
    let addr = system.spawn(DerivedActor::new(sources, selector));
    StateRef::from(&addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateActor;
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
    async fn derived_single_source() {
        let system = System::new();
        let state = system.spawn(StateActor::new(10u32));
        let state_ref = StateRef::from(&state);
        let d = derived(&system.handle(), state_ref, |snap: &Arc<u32>| **snap * 2);
        runtime::sleep(Duration::from_millis(50)).await;
        let val = d.get().await;
        assert_eq!(*val, 20);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_caches_unchanged() {
        let system = System::new();
        let state = system.spawn(StateActor::new(5u32));
        let state_ref = StateRef::from(&state);
        let count = Arc::new(AtomicU32::new(0));
        let d = derived(&system.handle(), state_ref, |snap: &Arc<u32>| **snap / 2); // 5/2=2
        runtime::sleep(Duration::from_millis(50)).await;

        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        d.subscribe(counter.into());

        // Set to 4 — derived is still 2 (4/2=2), should not notify
        state.ask(crate::state::Set(4u32)).await.unwrap();
        runtime::sleep(Duration::from_millis(100)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_multi_source_tuple2() {
        let system = System::new();
        let s1 = system.spawn(StateActor::new(3u32));
        let s2 = system.spawn(StateActor::new(7u32));
        let r1 = StateRef::from(&s1);
        let r2 = StateRef::from(&s2);
        let d = derived(
            &system.handle(),
            (r1, r2),
            |(a, b): &(Arc<u32>, Arc<u32>)| **a + **b,
        );
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(*d.get().await, 10);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_multi_source_tuple3() {
        let system = System::new();
        let s1 = system.spawn(StateActor::new(1u32));
        let s2 = system.spawn(StateActor::new(2u32));
        let s3 = system.spawn(StateActor::new(3u32));
        let r1 = StateRef::from(&s1);
        let r2 = StateRef::from(&s2);
        let r3 = StateRef::from(&s3);
        let d = derived(
            &system.handle(),
            (r1, r2, r3),
            |(a, b, c): &(Arc<u32>, Arc<u32>, Arc<u32>)| **a + **b + **c,
        );
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(*d.get().await, 6);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_multi_source_tuple4() {
        let system = System::new();
        let s1 = system.spawn(StateActor::new(1u32));
        let s2 = system.spawn(StateActor::new(2u32));
        let s3 = system.spawn(StateActor::new(3u32));
        let s4 = system.spawn(StateActor::new(4u32));
        let d = derived(
            &system.handle(),
            (
                StateRef::from(&s1),
                StateRef::from(&s2),
                StateRef::from(&s3),
                StateRef::from(&s4),
            ),
            |(a, b, c, d): &(Arc<u32>, Arc<u32>, Arc<u32>, Arc<u32>)| **a + **b + **c + **d,
        );
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(*d.get().await, 10);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_notifies_on_value_change() {
        let system = System::new();
        let state = system.spawn(StateActor::new(5u32));
        let state_ref = StateRef::from(&state);
        let count = Arc::new(AtomicU32::new(0));
        let d = derived(&system.handle(), state_ref, |snap: &Arc<u32>| **snap * 2);
        runtime::sleep(Duration::from_millis(50)).await;

        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        d.subscribe(counter.into());

        // Change value so derived changes: 5*2=10 -> 10*2=20
        state.ask(crate::state::Set(10u32)).await.unwrap();
        runtime::sleep(Duration::from_millis(100)).await;
        assert!(
            count.load(Ordering::SeqCst) >= 1,
            "subscriber should be notified when derived value changes"
        );
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_chain() {
        let system = System::new();
        let state = system.spawn(StateActor::new(5u32));
        let r = StateRef::from(&state);
        let d1 = derived(&system.handle(), r, |s: &Arc<u32>| **s * 2); // 10
        let d2 = derived(&system.handle(), d1, |s: &Arc<u32>| **s + 1); // 11
        runtime::sleep(Duration::from_millis(200)).await;
        assert_eq!(*d2.get().await, 11);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_chain_caches() {
        let system = System::new();
        let state = system.spawn(StateActor::new(5u32));
        let r = StateRef::from(&state);
        let d1 = derived(&system.handle(), r, |s: &Arc<u32>| **s * 2); // 10
        let d2 = derived(&system.handle(), d1, |s: &Arc<u32>| **s + 1); // 11
        runtime::sleep(Duration::from_millis(200)).await;
        let a = d2.get().await;
        let b = d2.get().await;
        assert!(Arc::ptr_eq(&a, &b));
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_initial_value() {
        let system = System::new();
        let state = system.spawn(StateActor::new(42u32));
        let r = StateRef::from(&state);
        let d = derived(&system.handle(), r, |s: &Arc<u32>| **s);
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(*d.get().await, 42);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_update_cache_self_message() {
        let system = System::new();
        let state = system.spawn(StateActor::new(1u32));
        let r = StateRef::from(&state);
        let d = derived(&system.handle(), r, |s: &Arc<u32>| **s * 10);
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(*d.get().await, 10);

        state.ask(crate::state::Set(2u32)).await.unwrap();
        runtime::sleep(Duration::from_millis(100)).await;
        assert_eq!(*d.get().await, 20);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_ref_from_derived_actor() {
        let system = System::new();
        let state = system.spawn(StateActor::new(7u32));
        let r = StateRef::from(&state);
        let d = derived(&system.handle(), r, |s: &Arc<u32>| **s + 3);
        runtime::sleep(Duration::from_millis(50)).await;
        // d is already a StateRef, verify it works
        let val = d.get().await;
        assert_eq!(*val, 10);
        system.shutdown().await;
    }
}
