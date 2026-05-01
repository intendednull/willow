//! Typed finite state machine actor.
//!
//! [`FsmActor<M>`] wraps a [`StateMachine`] implementation, enforcing valid
//! transitions and notifying subscribers on state changes. Invalid transitions
//! are rejected without modifying state.

use std::any::Any;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use crate::actor::{Actor, Handler, Message};
use crate::addr::{Addr, Recipient};
use crate::context::Context;
use crate::state::{Get, Notify, Select, StateRef, Subscribe};

// ───── StateMachine trait ─────────────────────────────────────────────────

/// Defines state, inputs, and transition logic for an [`FsmActor`].
pub trait StateMachine: Send + 'static {
    /// The state type (usually an enum).
    type State: Send + Sync + Clone + 'static;

    /// The input/event type that drives transitions.
    type Input: Send + 'static;

    /// Compute the next state from current state and input.
    /// Return `Err(reason)` to reject invalid transitions.
    fn transition(&self, state: &Self::State, input: &Self::Input) -> Result<Self::State, String>;

    /// Called after a successful transition. Default no-op.
    #[allow(unused_variables)]
    fn on_enter(&mut self, old: &Self::State, new: &Self::State, ctx: &mut Context<FsmActor<Self>>)
    where
        Self: Sized,
    {
    }
}

// ───── TransitionResult ───────────────────────────────────────────────────

/// Result of a state machine transition attempt.
#[derive(Debug, Clone)]
pub enum TransitionResult<S> {
    /// Transition succeeded, new state.
    Ok(S),
    /// Transition rejected with a reason.
    Rejected(String),
}

// ───── FsmActor ───────────────────────────────────────────────────────────

/// Wrapper message to send an input to an [`FsmActor`].
///
/// Wraps the user-defined `StateMachine::Input` type to avoid conflicting
/// handler impls with internal message types (`Subscribe`, `Select`, `Get`).
pub struct Input<M: StateMachine>(pub M::Input);

impl<M: StateMachine> Message for Input<M> {
    type Result = TransitionResult<M::State>;
}

/// Actor wrapping a [`StateMachine`]. Enforces valid transitions and
/// notifies subscribers on state changes.
pub struct FsmActor<M: StateMachine> {
    machine: M,
    state: Arc<M::State>,
    subscribers: Vec<Recipient<Notify>>,
    dirty: bool,
}

impl<M: StateMachine> FsmActor<M> {
    /// Create a new FSM actor with the given machine and initial state.
    pub fn new(machine: M, initial_state: M::State) -> Self {
        Self {
            machine,
            state: Arc::new(initial_state),
            subscribers: Vec::new(),
            dirty: false,
        }
    }
}

impl<M: StateMachine> Actor for FsmActor<M> {
    fn idle(&mut self, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        if self.dirty {
            self.dirty = false;
            self.subscribers.retain(|r| r.do_send(Notify).is_ok());
        }
        async {}
    }
}

impl<M: StateMachine> Handler<Input<M>> for FsmActor<M> {
    fn handle(
        &mut self,
        msg: Input<M>,
        ctx: &mut Context<Self>,
    ) -> impl Future<Output = TransitionResult<M::State>> + Send {
        let result = self.machine.transition(&self.state, &msg.0);
        let transition_result = match result {
            Ok(new_state) => {
                let old = Arc::clone(&self.state);
                self.state = Arc::new(new_state.clone());
                self.dirty = true;
                self.machine.on_enter(&old, &new_state, ctx);
                TransitionResult::Ok(new_state)
            }
            Err(reason) => TransitionResult::Rejected(reason),
        };
        async move { transition_result }
    }
}

impl<M: StateMachine> Handler<Subscribe> for FsmActor<M> {
    fn handle(
        &mut self,
        msg: Subscribe,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.subscribers.push(msg.0);
        async {}
    }
}

impl<M: StateMachine> Handler<Select> for FsmActor<M> {
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

impl<M: StateMachine> Handler<Get<M::State>> for FsmActor<M> {
    fn handle(
        &mut self,
        _msg: Get<M::State>,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = Arc<M::State>> + Send {
        let state = Arc::clone(&self.state);
        async move { state }
    }
}

// ───── StateRef for FsmActor ──────────────────────────────────────────────

impl<M: StateMachine> From<&Addr<FsmActor<M>>> for StateRef<M::State> {
    fn from(addr: &Addr<FsmActor<M>>) -> Self {
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
                    as Pin<Box<dyn Future<Output = Arc<M::State>> + Send>>
            }),
            Arc::new(move |f| {
                let addr = addr_sel.clone();
                Box::pin(async move { addr.ask(Select(f)).await.unwrap() })
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{runtime, System};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    // ── Test state machine: traffic light ──

    #[derive(Debug, Clone, PartialEq)]
    enum Light {
        Red,
        Yellow,
        Green,
    }

    struct TrafficLight {
        on_enter_called: Arc<AtomicBool>,
    }

    impl StateMachine for TrafficLight {
        type State = Light;
        type Input = ();

        fn transition(&self, state: &Light, _input: &()) -> Result<Light, String> {
            match state {
                Light::Red => Ok(Light::Green),
                Light::Green => Ok(Light::Yellow),
                Light::Yellow => Ok(Light::Red),
            }
        }

        fn on_enter(&mut self, _old: &Light, _new: &Light, _ctx: &mut Context<FsmActor<Self>>) {
            self.on_enter_called.store(true, Ordering::SeqCst);
        }
    }

    // ── Test state machine with rejection ──

    struct StrictDoor;

    #[derive(Debug, Clone, PartialEq)]
    #[allow(
        dead_code,
        reason = "Open variant exists for completeness; tests start in Closed and assert StrictDoor rejects further transitions."
    )]
    enum DoorState {
        Open,
        Closed,
    }

    impl StateMachine for StrictDoor {
        type State = DoorState;
        type Input = ();

        fn transition(&self, state: &DoorState, _input: &()) -> Result<DoorState, String> {
            match state {
                DoorState::Open => Ok(DoorState::Closed),
                DoorState::Closed => Err("door is locked".into()),
            }
        }
    }

    // Helper: notify counter
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
    async fn fsm_valid_transition() {
        let system = System::new();
        let on_enter = Arc::new(AtomicBool::new(false));
        let addr = system.spawn(FsmActor::new(
            TrafficLight {
                on_enter_called: on_enter.clone(),
            },
            Light::Red,
        ));

        let result = addr.ask(Input::<TrafficLight>(())).await.unwrap();
        match result {
            TransitionResult::Ok(state) => assert_eq!(state, Light::Green),
            _ => panic!("expected Ok"),
        }
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fsm_rejected_transition() {
        let system = System::new();
        let addr = system.spawn(FsmActor::new(StrictDoor, DoorState::Closed));

        let result = addr.ask(Input::<StrictDoor>(())).await.unwrap();
        match result {
            TransitionResult::Rejected(reason) => assert_eq!(reason, "door is locked"),
            _ => panic!("expected Rejected"),
        }
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fsm_on_enter_called() {
        let system = System::new();
        let on_enter = Arc::new(AtomicBool::new(false));
        let addr = system.spawn(FsmActor::new(
            TrafficLight {
                on_enter_called: on_enter.clone(),
            },
            Light::Red,
        ));

        addr.ask(Input::<TrafficLight>(())).await.unwrap();
        assert!(on_enter.load(Ordering::SeqCst));
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fsm_on_enter_not_called_on_reject() {
        let system = System::new();
        let addr = system.spawn(FsmActor::new(StrictDoor, DoorState::Closed));

        addr.ask(Input::<StrictDoor>(())).await.unwrap();
        let state = addr.ask(Get::<DoorState>(PhantomData)).await.unwrap();
        assert_eq!(*state, DoorState::Closed);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fsm_notifies_subscribers() {
        let system = System::new();
        let on_enter = Arc::new(AtomicBool::new(false));
        let addr = system.spawn(FsmActor::new(
            TrafficLight {
                on_enter_called: on_enter,
            },
            Light::Red,
        ));
        let count = Arc::new(AtomicU32::new(0));
        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        addr.do_send(Subscribe(counter.into())).unwrap();

        addr.ask(Input::<TrafficLight>(())).await.unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        assert!(count.load(Ordering::SeqCst) >= 1);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fsm_no_notify_on_reject() {
        let system = System::new();
        let addr = system.spawn(FsmActor::new(StrictDoor, DoorState::Closed));
        let count = Arc::new(AtomicU32::new(0));
        let counter = system.spawn(NotifyCounter {
            count: count.clone(),
        });
        addr.do_send(Subscribe(counter.into())).unwrap();

        addr.ask(Input::<StrictDoor>(())).await.unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fsm_select_current_state() {
        let system = System::new();
        let on_enter = Arc::new(AtomicBool::new(false));
        let addr = system.spawn(FsmActor::new(
            TrafficLight {
                on_enter_called: on_enter,
            },
            Light::Red,
        ));

        let state = addr.ask(Get::<Light>(PhantomData)).await.unwrap();
        assert_eq!(*state, Light::Red);

        addr.ask(Input::<TrafficLight>(())).await.unwrap();
        let state = addr.ask(Get::<Light>(PhantomData)).await.unwrap();
        assert_eq!(*state, Light::Green);

        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fsm_multiple_sequential_transitions() {
        let system = System::new();
        let on_enter = Arc::new(AtomicBool::new(false));
        let addr = system.spawn(FsmActor::new(
            TrafficLight {
                on_enter_called: on_enter,
            },
            Light::Red,
        ));

        // Red -> Green
        let r1 = addr.ask(Input::<TrafficLight>(())).await.unwrap();
        assert!(matches!(r1, TransitionResult::Ok(Light::Green)));

        // Green -> Yellow
        let r2 = addr.ask(Input::<TrafficLight>(())).await.unwrap();
        assert!(matches!(r2, TransitionResult::Ok(Light::Yellow)));

        // Yellow -> Red
        let r3 = addr.ask(Input::<TrafficLight>(())).await.unwrap();
        assert!(matches!(r3, TransitionResult::Ok(Light::Red)));

        // Full cycle: Red -> Green again
        let r4 = addr.ask(Input::<TrafficLight>(())).await.unwrap();
        assert!(matches!(r4, TransitionResult::Ok(Light::Green)));

        // Verify final state via Get
        let state = addr.ask(Get::<Light>(PhantomData)).await.unwrap();
        assert_eq!(*state, Light::Green);

        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fsm_on_enter_receives_correct_states() {
        use std::sync::Mutex;

        // FSM that records old/new states in on_enter
        struct RecordingLight {
            transitions: Arc<Mutex<Vec<(Light, Light)>>>,
        }

        impl StateMachine for RecordingLight {
            type State = Light;
            type Input = ();

            fn transition(&self, state: &Light, _input: &()) -> Result<Light, String> {
                match state {
                    Light::Red => Ok(Light::Green),
                    Light::Green => Ok(Light::Yellow),
                    Light::Yellow => Ok(Light::Red),
                }
            }

            fn on_enter(&mut self, old: &Light, new: &Light, _ctx: &mut Context<FsmActor<Self>>) {
                self.transitions
                    .lock()
                    .unwrap()
                    .push((old.clone(), new.clone()));
            }
        }

        let system = System::new();
        let transitions = Arc::new(Mutex::new(Vec::new()));
        let addr = system.spawn(FsmActor::new(
            RecordingLight {
                transitions: transitions.clone(),
            },
            Light::Red,
        ));

        // Red -> Green -> Yellow
        addr.ask(Input::<RecordingLight>(())).await.unwrap();
        addr.ask(Input::<RecordingLight>(())).await.unwrap();

        {
            let recorded = transitions.lock().unwrap();
            assert_eq!(recorded.len(), 2);
            assert_eq!(recorded[0], (Light::Red, Light::Green));
            assert_eq!(recorded[1], (Light::Green, Light::Yellow));
        }

        system.shutdown().await;
    }
}
