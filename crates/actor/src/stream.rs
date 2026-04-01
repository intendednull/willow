//! Actor-produced async streams via bounded channels.
//!
//! [`StreamOutput<T>`] allows actors to emit values to multiple consumers.
//! Each consumer gets an [`OutputStream<T>`] implementing `futures::Stream`.
//! Backpressure: values are dropped (with a warning) if a consumer's buffer is full.

use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

use futures_core::Stream;

use crate::actor::Message;
use crate::runtime::{self, BoundedReceiver, BoundedSender};

// ───── StreamOutput ───────────────────────────────────────────────────────

/// Actor-side stream producer. Holds bounded channels to multiple consumers.
///
/// Call [`emit()`](StreamOutput::emit) to push values to all subscribers.
/// Dead or full channels are handled gracefully.
pub struct StreamOutput<T: Clone + Send + 'static> {
    subscribers: Vec<BoundedSender<T>>,
    default_capacity: usize,
}

impl<T: Clone + Send + 'static> StreamOutput<T> {
    /// Create a new stream output with default buffer capacity of 64.
    pub fn new() -> Self {
        Self {
            subscribers: Vec::new(),
            default_capacity: 64,
        }
    }

    /// Emit a value to all subscribers.
    ///
    /// Prunes closed channels. Logs a warning when dropping a value due to
    /// a full buffer (backpressure).
    pub fn emit(&mut self, value: T) {
        self.subscribers.retain(|sender| {
            if sender.is_closed() {
                return false;
            }
            match sender.try_send(value.clone()) {
                Ok(()) => true,
                Err(_) => {
                    if sender.is_closed() {
                        false // channel closed, prune
                    } else {
                        tracing::warn!("StreamOutput: dropping value, consumer buffer full");
                        true // buffer full, keep but drop value
                    }
                }
            }
        });
    }

    /// Subscribe a new consumer with the default buffer capacity.
    pub fn subscribe(&mut self) -> OutputStream<T> {
        self.subscribe_with_capacity(self.default_capacity)
    }

    /// Subscribe a new consumer with a custom buffer capacity.
    pub fn subscribe_with_capacity(&mut self, capacity: usize) -> OutputStream<T> {
        let (tx, rx) = runtime::bounded_channel(capacity);
        self.subscribers.push(tx);
        OutputStream(rx)
    }
}

impl<T: Clone + Send + 'static> Default for StreamOutput<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ───── OutputStream ───────────────────────────────────────────────────────

/// Consumer-side stream. Implements `futures::Stream<Item = T>`.
pub struct OutputStream<T: Send + 'static>(BoundedReceiver<T>);

impl<T: Send + 'static> Stream for OutputStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx)
    }
}

// ───── SubscribeStream message ────────────────────────────────────────────

/// Message to subscribe to an actor's stream output.
pub struct SubscribeStream<T: Send + 'static>(PhantomData<T>);

impl<T: Send + 'static> Default for SubscribeStream<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T: Clone + Send + 'static> Message for SubscribeStream<T> {
    type Result = OutputStream<T>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Actor, Handler};
    use crate::context::Context;
    use crate::{runtime, System};
    use futures::StreamExt;
    use std::future::Future;
    use std::time::Duration;

    struct StreamProducer {
        output: StreamOutput<u32>,
    }

    impl Actor for StreamProducer {}

    struct Emit(u32);
    impl Message for Emit {
        type Result = ();
    }

    impl Handler<Emit> for StreamProducer {
        fn handle(
            &mut self,
            msg: Emit,
            _ctx: &mut Context<Self>,
        ) -> impl Future<Output = ()> + Send {
            self.output.emit(msg.0);
            async {}
        }
    }

    impl Handler<SubscribeStream<u32>> for StreamProducer {
        fn handle(
            &mut self,
            _msg: SubscribeStream<u32>,
            _ctx: &mut Context<Self>,
        ) -> impl Future<Output = OutputStream<u32>> + Send {
            let stream = self.output.subscribe();
            async move { stream }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_output_single_consumer() {
        let system = System::new();
        let addr = system.spawn(StreamProducer {
            output: StreamOutput::new(),
        });

        let mut stream = addr.ask(SubscribeStream::<u32>::default()).await.unwrap();
        addr.do_send(Emit(1)).unwrap();
        addr.do_send(Emit(2)).unwrap();
        addr.do_send(Emit(3)).unwrap();

        runtime::sleep(Duration::from_millis(50)).await;

        let v1 = stream.next().await.unwrap();
        let v2 = stream.next().await.unwrap();
        let v3 = stream.next().await.unwrap();
        assert_eq!((v1, v2, v3), (1, 2, 3));

        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_output_multi_consumer() {
        let system = System::new();
        let addr = system.spawn(StreamProducer {
            output: StreamOutput::new(),
        });

        let mut s1 = addr.ask(SubscribeStream::<u32>::default()).await.unwrap();
        let mut s2 = addr.ask(SubscribeStream::<u32>::default()).await.unwrap();

        addr.do_send(Emit(42)).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;

        assert_eq!(s1.next().await.unwrap(), 42);
        assert_eq!(s2.next().await.unwrap(), 42);

        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_output_consumer_drop() {
        let system = System::new();
        let addr = system.spawn(StreamProducer {
            output: StreamOutput::new(),
        });

        let stream = addr.ask(SubscribeStream::<u32>::default()).await.unwrap();
        drop(stream); // Drop consumer

        // Should not panic when emitting to dead consumer
        addr.do_send(Emit(1)).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;

        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_output_backpressure() {
        let system = System::new();
        let addr = system.spawn(StreamProducer {
            output: StreamOutput::new(),
        });

        // Subscribe with tiny buffer
        let mut output = StreamOutput::<u32>::new();
        let mut stream = output.subscribe_with_capacity(2);

        // Fill buffer
        output.emit(1);
        output.emit(2);
        // This should be dropped (buffer full)
        output.emit(3);

        let v1 = stream.next().await.unwrap();
        let v2 = stream.next().await.unwrap();
        assert_eq!((v1, v2), (1, 2));

        drop(addr);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_output_custom_capacity() {
        let mut output = StreamOutput::<u32>::new();
        let mut stream = output.subscribe_with_capacity(3);

        output.emit(1);
        output.emit(2);
        output.emit(3);
        // 4th should be dropped
        output.emit(4);

        assert_eq!(stream.next().await.unwrap(), 1);
        assert_eq!(stream.next().await.unwrap(), 2);
        assert_eq!(stream.next().await.unwrap(), 3);

        let system = System::new();
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_output_subscribe_stream_message() {
        let system = System::new();
        let addr = system.spawn(StreamProducer {
            output: StreamOutput::new(),
        });

        let mut stream = addr.ask(SubscribeStream::<u32>::default()).await.unwrap();
        addr.do_send(Emit(99)).unwrap();
        runtime::sleep(Duration::from_millis(50)).await;
        assert_eq!(stream.next().await.unwrap(), 99);

        system.shutdown().await;
    }
}
