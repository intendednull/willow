//! Error types for actor message passing.

use std::fmt;

/// Error returned when sending a message to a closed mailbox.
pub struct SendError<M>(pub M);

impl<M> fmt::Debug for SendError<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SendError(..)")
    }
}

impl<M> fmt::Display for SendError<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("actor mailbox is closed")
    }
}

impl<M> std::error::Error for SendError<M> {}

/// Error returned when an `ask()` call fails.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// The actor's mailbox is closed (actor stopped).
    #[error("actor mailbox is closed")]
    Closed,
    /// The actor did not respond (dropped the reply channel).
    #[error("actor did not respond (dropped the reply channel)")]
    NoResponse,
}
