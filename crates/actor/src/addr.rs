//! Actor addresses: [`Addr<A>`], [`AnyAddr`], and [`Recipient<M>`].

use std::future::Future;
use std::marker::PhantomData;

use crate::actor::{Actor, Handler, Message};
use crate::envelope::{self, BoxEnvelope};
use crate::error::{AskError, SendError};
use crate::runtime::{self, Sender};

/// Type-safe handle for sending messages to an actor.
///
/// Cheaply cloneable (wraps a channel sender).
pub struct Addr<A: Actor> {
    pub(crate) tx: Sender<BoxEnvelope<A>>,
    _phantom: PhantomData<A>,
}

impl<A: Actor> Addr<A> {
    pub(crate) fn new(tx: Sender<BoxEnvelope<A>>) -> Self {
        Self {
            tx,
            _phantom: PhantomData,
        }
    }

    /// Fire-and-forget: send a message, don't wait for a response.
    ///
    /// Returns `Err` if the actor's mailbox is closed.
    pub fn send<M>(&self, msg: M) -> Result<(), SendError<M>>
    where
        A: Handler<M>,
        M: Message<Result = ()>,
    {
        // Check if closed before wrapping the message, so we can return it.
        if self.tx.is_closed() {
            return Err(SendError(msg));
        }
        let envelope = envelope::envelope_send(msg);
        // Channel could close between our check and send — that's fine,
        // the message is lost (same as any async channel race).
        let _ = self.tx.send(envelope);
        Ok(())
    }

    /// Fire-and-forget with a simpler error (drops the message on failure).
    #[allow(clippy::result_unit_err)]
    pub fn do_send<M>(&self, msg: M) -> Result<(), ()>
    where
        A: Handler<M>,
        M: Message<Result = ()>,
    {
        let envelope = envelope::envelope_send(msg);
        self.tx.send(envelope).map_err(|_| ())
    }

    /// Request-reply: send a message and await the response.
    pub fn ask<M>(&self, msg: M) -> impl Future<Output = Result<M::Result, AskError>> + Send
    where
        A: Handler<M>,
        M: Message,
    {
        let (reply_tx, reply_rx) = runtime::oneshot();
        let envelope = envelope::envelope_ask(msg, reply_tx);
        let send_result = self.tx.send(envelope);

        async move {
            send_result.map_err(|_| AskError::Closed)?;
            reply_rx.recv().await.map_err(|_| AskError::NoResponse)
        }
    }

    /// Check if the actor is still alive.
    pub fn is_alive(&self) -> bool {
        !self.tx.is_closed()
    }
}

impl<A: Actor> Clone for Addr<A> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            _phantom: PhantomData,
        }
    }
}

// Addr is safe to share — it only holds a channel sender which is Send+Sync.
// The PhantomData<A> would prevent Sync if A isn't Sync, but the actor type
// is never accessed through the Addr — only the channel sender is used.
unsafe impl<A: Actor> Sync for Addr<A> {}

// ───── AnyAddr ─────────────────────────────────────────────────────────────

/// Type-erased actor address. Can check liveness but cannot send typed messages.
pub struct AnyAddr {
    is_closed: Box<dyn Fn() -> bool + Send + Sync>,
}

impl AnyAddr {
    /// Check if the actor is still alive.
    pub fn is_alive(&self) -> bool {
        !(self.is_closed)()
    }
}

impl<A: Actor> From<Addr<A>> for AnyAddr {
    fn from(addr: Addr<A>) -> Self {
        let tx = addr.tx.clone();
        Self {
            is_closed: Box::new(move || tx.is_closed()),
        }
    }
}

// ───── Recipient ───────────────────────────────────────────────────────────

/// Type-erased handle that can send a specific message type.
///
/// Useful for pub-sub patterns where the sender doesn't know the actor type.
pub struct Recipient<M: Message> {
    inner: Box<dyn RecipientInner<M> + Send + Sync>,
}

trait RecipientInner<M: Message>: Send + Sync {
    fn do_send(&self, msg: M) -> Result<(), ()>;
    fn ask(
        &self,
        msg: M,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<M::Result, AskError>> + Send>>;
    fn is_alive(&self) -> bool;
    fn clone_box(&self) -> Box<dyn RecipientInner<M> + Send + Sync>;
}

struct AddrRecipient<A: Actor> {
    addr: Addr<A>,
}

impl<A, M> RecipientInner<M> for AddrRecipient<A>
where
    A: Handler<M>,
    M: Message,
{
    fn do_send(&self, msg: M) -> Result<(), ()> {
        // Use ask envelope with a dropped receiver. The handler result
        // is computed but the oneshot send silently fails. This is slightly
        // wasteful (one oneshot alloc) but correct for any M::Result type.
        let (reply_tx, _reply_rx) = runtime::oneshot::<M::Result>();
        let envelope = envelope::envelope_ask(msg, reply_tx);
        self.addr.tx.send(envelope).map_err(|_| ())
    }

    fn ask(
        &self,
        msg: M,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<M::Result, AskError>> + Send>> {
        let (reply_tx, reply_rx) = runtime::oneshot();
        let envelope = envelope::envelope_ask(msg, reply_tx);
        let send_result = self.addr.tx.send(envelope);

        Box::pin(async move {
            send_result.map_err(|_| AskError::Closed)?;
            reply_rx.recv().await.map_err(|_| AskError::NoResponse)
        })
    }

    fn is_alive(&self) -> bool {
        self.addr.is_alive()
    }

    fn clone_box(&self) -> Box<dyn RecipientInner<M> + Send + Sync> {
        Box::new(AddrRecipient {
            addr: self.addr.clone(),
        })
    }
}

impl<M: Message> Recipient<M> {
    /// Send a message, discarding the result. Returns `Err(())` if the actor is dead.
    #[allow(clippy::result_unit_err)]
    pub fn do_send(&self, msg: M) -> Result<(), ()> {
        self.inner.do_send(msg)
    }

    /// Send a message and await the response.
    pub fn ask(
        &self,
        msg: M,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<M::Result, AskError>> + Send>> {
        self.inner.ask(msg)
    }

    /// Check if the underlying actor is still alive.
    pub fn is_alive(&self) -> bool {
        self.inner.is_alive()
    }
}

impl<M: Message> Clone for Recipient<M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone_box(),
        }
    }
}

impl<A, M> From<Addr<A>> for Recipient<M>
where
    A: Handler<M>,
    M: Message,
{
    fn from(addr: Addr<A>) -> Self {
        Self {
            inner: Box::new(AddrRecipient { addr }),
        }
    }
}
