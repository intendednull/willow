//! # Willow State
//!
//! Pure, deterministic event-sourced state machine for the Willow P2P chat
//! network. All state is derived from a per-author Merkle-DAG of signed
//! events via the [`materialize`] function. This crate has zero I/O, zero
//! networking — just DAG operations and deterministic state projection.

pub mod dag;
pub mod event;
pub mod hash;
