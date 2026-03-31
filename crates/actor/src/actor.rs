//! Core actor traits: [`Actor`], [`Handler`], [`StreamHandler`], and [`Message`].

use std::future::Future;

use crate::context::Context;

/// Marker trait for actor messages.
///
/// Every message type specifies the response type via `Result`.
/// Use `()` for fire-and-forget messages.
pub trait Message: Send + 'static {
    /// The response type. Use `()` for fire-and-forget.
    type Result: Send + 'static;
}

/// An actor processes messages sequentially in its own task.
///
/// Actors are spawned via [`System::spawn`](crate::System::spawn) and
/// communicate through typed [`Addr`](crate::Addr) handles.
pub trait Actor: Send + 'static + Sized {
    /// Called once when the actor starts, before processing messages.
    fn started(&mut self, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Called when the actor is stopping (mailbox closed or explicit stop).
    fn stopped(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Called after the mailbox drains all immediately-available messages.
    ///
    /// The mailbox processes one message via `recv().await`, then drains
    /// remaining messages via `try_recv()`, then calls `idle()`. Use this
    /// for batched notifications.
    fn idle(&mut self, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// Implement `Handler<M>` for each message type an actor accepts.
///
/// An actor can implement `Handler` for multiple message types.
/// Each handler is type-checked at compile time.
pub trait Handler<M: Message>: Actor {
    /// Handle a message and return a response.
    fn handle(&mut self, msg: M, ctx: &mut Context<Self>)
        -> impl Future<Output = M::Result> + Send;
}

/// Handle items from an attached stream.
///
/// Attach a stream via [`Context::add_stream`](crate::Context::add_stream).
pub trait StreamHandler<S: Send + 'static>: Actor {
    /// Called for each item from the stream.
    fn handle_stream_item(
        &mut self,
        item: S,
        ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send;

    /// Called when the stream ends.
    fn stream_finished(&mut self, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        async {}
    }
}
