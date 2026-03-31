//! Platform abstraction for spawn, channels, and timers.
//!
//! On native: tokio mpsc/oneshot/sleep.
//! On WASM: futures-channel + gloo-timers + wasm-bindgen-futures.

use std::future::Future;
use std::time::Duration;

/// Spawn a future as a background task.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn<F: Future<Output = ()> + Send + 'static>(fut: F) {
    tokio::task::spawn(fut);
}

#[cfg(target_arch = "wasm32")]
pub fn spawn<F: Future<Output = ()> + 'static>(fut: F) {
    wasm_bindgen_futures::spawn_local(fut);
}

/// Sleep for a duration.
#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(target_arch = "wasm32")]
pub async fn sleep(duration: Duration) {
    gloo_timers::future::sleep(duration).await;
}

// ───── Unbounded MPSC channel ──────────────────────────────────────────────

/// Sender half of an unbounded MPSC channel.
#[cfg(not(target_arch = "wasm32"))]
pub struct Sender<T>(tokio::sync::mpsc::UnboundedSender<T>);

#[cfg(target_arch = "wasm32")]
pub struct Sender<T>(futures_channel::mpsc::UnboundedSender<T>);

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Send + 'static> Sender<T> {
    /// Send a value. Returns `Err` if the receiver is closed.
    pub fn send(&self, val: T) -> Result<(), T> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.send(val).map_err(|e| e.0)
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.0.unbounded_send(val).map_err(|e| e.into_inner())
        }
    }

    /// Check if the channel is closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// Receiver half of an unbounded MPSC channel.
#[cfg(not(target_arch = "wasm32"))]
pub struct Receiver<T>(tokio::sync::mpsc::UnboundedReceiver<T>);

#[cfg(target_arch = "wasm32")]
pub struct Receiver<T>(futures_channel::mpsc::UnboundedReceiver<T>);

impl<T: Send + 'static> Receiver<T> {
    /// Wait for the next value. Returns `None` when all senders are dropped.
    pub async fn recv(&mut self) -> Option<T> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.recv().await
        }

        #[cfg(target_arch = "wasm32")]
        {
            use futures_core::Stream;
            use std::pin::Pin;
            std::future::poll_fn(|cx| Pin::new(&mut self.0).poll_next(cx)).await
        }
    }

    /// Try to receive a value without blocking.
    pub fn try_recv(&mut self) -> Option<T> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.try_recv().ok()
        }

        #[cfg(target_arch = "wasm32")]
        {
            #[allow(deprecated)]
            match self.0.try_next() {
                Ok(Some(val)) => Some(val),
                _ => None,
            }
        }
    }
}

/// Create an unbounded MPSC channel.
pub fn unbounded_channel<T: Send + 'static>() -> (Sender<T>, Receiver<T>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Sender(tx), Receiver(rx))
    }

    #[cfg(target_arch = "wasm32")]
    {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        (Sender(tx), Receiver(rx))
    }
}

// ───── Oneshot channel ─────────────────────────────────────────────────────

/// Sender half of a oneshot channel.
#[cfg(not(target_arch = "wasm32"))]
pub struct OneshotTx<T>(pub(crate) tokio::sync::oneshot::Sender<T>);

#[cfg(target_arch = "wasm32")]
pub struct OneshotTx<T>(pub(crate) futures_channel::oneshot::Sender<T>);

impl<T: Send + 'static> OneshotTx<T> {
    /// Send a value. Returns `Err` if the receiver is dropped.
    pub fn send(self, val: T) -> Result<(), T> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.send(val)
        }

        #[cfg(target_arch = "wasm32")]
        {
            // futures_channel::oneshot::Sender::send returns Err(T) on failure,
            // but we need Err(T) for our API. The cancellation error doesn't
            // carry the value, so we reconstruct it.
            self.0.send(val).map_err(|e| e)
        }
    }
}

/// Receiver half of a oneshot channel.
#[cfg(not(target_arch = "wasm32"))]
pub struct OneshotRx<T>(pub(crate) tokio::sync::oneshot::Receiver<T>);

#[cfg(target_arch = "wasm32")]
pub struct OneshotRx<T>(pub(crate) futures_channel::oneshot::Receiver<T>);

impl<T: Send + 'static> OneshotRx<T> {
    /// Await the value. Returns `Err` if the sender is dropped.
    pub async fn recv(self) -> Result<T, ()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.await.map_err(|_| ())
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.0.await.map_err(|_| ())
        }
    }
}

/// Create a oneshot channel.
pub fn oneshot<T: Send + 'static>() -> (OneshotTx<T>, OneshotRx<T>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (OneshotTx(tx), OneshotRx(rx))
    }

    #[cfg(target_arch = "wasm32")]
    {
        let (tx, rx) = futures_channel::oneshot::channel();
        (OneshotTx(tx), OneshotRx(rx))
    }
}
