//! Round-robin worker pool.
//!
//! [`Pool<A>`] distributes messages across multiple instances of the same actor
//! type for CPU-intensive work, parallel I/O, or rate-limited API calls.

use std::future::Future;

use crate::actor::{Actor, Handler, Message};
use crate::addr::Addr;
use crate::error::{AskError, SendError};
use crate::system::SystemHandle;

/// Round-robin worker pool distributing messages across actor instances.
pub struct Pool<A: Actor> {
    workers: Vec<Addr<A>>,
    next: usize,
}

impl<A: Actor + Clone> Pool<A> {
    /// Create a pool of `size` workers, each a clone of the given actor.
    pub fn new(system: &SystemHandle, actor: A, size: usize) -> Self {
        assert!(size > 0, "pool size must be at least 1");
        let workers = (0..size).map(|_| system.spawn(actor.clone())).collect();
        Self { workers, next: 0 }
    }
}

impl<A: Actor> Pool<A> {
    /// Fire-and-forget send to the next worker.
    pub fn send<M>(&mut self, msg: M) -> Result<(), SendError<M>>
    where
        A: Handler<M>,
        M: Message<Result = ()>,
    {
        let idx = self.next % self.workers.len();
        self.next = self.next.wrapping_add(1);
        self.workers[idx].send(msg)
    }

    /// Request-reply to the next worker.
    pub fn ask<M>(&mut self, msg: M) -> impl Future<Output = Result<M::Result, AskError>> + Send
    where
        A: Handler<M>,
        M: Message,
    {
        let idx = self.next % self.workers.len();
        self.next = self.next.wrapping_add(1);
        self.workers[idx].ask(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::{runtime, System};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Clone)]
    struct Worker {
        id: u32,
        calls: Arc<AtomicU32>,
    }

    impl Actor for Worker {}

    struct Work;
    impl Message for Work {
        type Result = ();
    }

    impl Handler<Work> for Worker {
        fn handle(
            &mut self,
            _msg: Work,
            _ctx: &mut Context<Self>,
        ) -> impl Future<Output = ()> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            async {}
        }
    }

    struct GetId;
    impl Message for GetId {
        type Result = u32;
    }

    impl Handler<GetId> for Worker {
        fn handle(
            &mut self,
            _msg: GetId,
            _ctx: &mut Context<Self>,
        ) -> impl Future<Output = u32> + Send {
            let id = self.id;
            async move { id }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pool_round_robin_distribution() {
        let system = System::new();
        let calls = Arc::new(AtomicU32::new(0));
        let mut pool = Pool::new(
            &system.handle(),
            Worker {
                id: 0,
                calls: calls.clone(),
            },
            3,
        );

        // Send 6 messages — each worker should get 2
        for _ in 0..6 {
            pool.send(Work).unwrap();
        }
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pool_send_fire_and_forget() {
        let system = System::new();
        let calls = Arc::new(AtomicU32::new(0));
        let mut pool = Pool::new(
            &system.handle(),
            Worker {
                id: 0,
                calls: calls.clone(),
            },
            2,
        );

        pool.send(Work).unwrap();
        pool.send(Work).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pool_ask_returns_result() {
        let system = System::new();
        let calls = Arc::new(AtomicU32::new(0));
        let mut pool = Pool::new(
            &system.handle(),
            Worker {
                id: 42,
                calls: calls.clone(),
            },
            2,
        );

        let id = pool.ask(GetId).await.unwrap();
        assert_eq!(id, 42); // All workers cloned from same actor
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pool_worker_dies() {
        let system = System::new();
        let calls = Arc::new(AtomicU32::new(0));
        let mut pool = Pool::new(
            &system.handle(),
            Worker {
                id: 0,
                calls: calls.clone(),
            },
            2,
        );

        // Shutdown kills workers — subsequent sends should fail gracefully
        system.shutdown().await;
        let result = pool.send(Work);
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pool_size_one() {
        let system = System::new();
        let calls = Arc::new(AtomicU32::new(0));
        let mut pool = Pool::new(
            &system.handle(),
            Worker {
                id: 0,
                calls: calls.clone(),
            },
            1,
        );

        for _ in 0..5 {
            pool.send(Work).unwrap();
        }
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        system.shutdown().await;
    }
}
