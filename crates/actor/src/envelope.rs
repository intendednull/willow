//! Type-erased message dispatch via closure-based envelopes.
//!
//! Each message is wrapped in a closure that captures the handler call.
//! The mailbox receives `BoxEnvelope<A>` — one channel per actor,
//! handling all message types.

use std::future::Future;
use std::pin::Pin;

use crate::actor::{Handler, Message, StreamHandler};
use crate::context::Context;
use crate::runtime::OneshotTx;

/// A type-erased, boxed async closure that processes one message.
pub type BoxEnvelope<A> = Box<
    dyn for<'a> FnOnce(
            &'a mut A,
            &'a mut Context<A>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
        + Send,
>;

/// Create an envelope for a fire-and-forget message (`send()`).
pub fn envelope_send<A, M>(msg: M) -> BoxEnvelope<A>
where
    A: Handler<M>,
    M: Message<Result = ()>,
{
    Box::new(move |actor: &mut A, ctx: &mut Context<A>| {
        Box::pin(async move {
            actor.handle(msg, ctx).await;
        })
    })
}

/// Create an envelope for a request-reply message (`ask()`).
pub fn envelope_ask<A, M>(msg: M, reply_tx: OneshotTx<M::Result>) -> BoxEnvelope<A>
where
    A: Handler<M>,
    M: Message,
{
    Box::new(move |actor: &mut A, ctx: &mut Context<A>| {
        Box::pin(async move {
            let result = actor.handle(msg, ctx).await;
            let _ = reply_tx.send(result);
        })
    })
}

/// Create an envelope for a stream item.
pub fn envelope_stream_item<A, S>(item: S) -> BoxEnvelope<A>
where
    A: StreamHandler<S>,
    S: Send + 'static,
{
    Box::new(move |actor: &mut A, ctx: &mut Context<A>| {
        Box::pin(async move {
            actor.handle_stream_item(item, ctx).await;
        })
    })
}

/// Create an envelope for stream-finished notification.
pub fn envelope_stream_finished<A, S>() -> BoxEnvelope<A>
where
    A: StreamHandler<S>,
    S: Send + 'static,
{
    Box::new(move |actor: &mut A, ctx: &mut Context<A>| {
        Box::pin(async move {
            actor.stream_finished(ctx).await;
        })
    })
}
