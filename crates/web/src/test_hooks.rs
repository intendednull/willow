//! Test instrumentation for the Willow web UI.
//!
//! This module is gated behind the `test-hooks` cargo feature and is
//! **never compiled into production builds**. It exposes
//! `WillowTestHooks` to JavaScript via `wasm_bindgen` so Playwright
//! e2e tests can synchronise on real signals (applied events, DAG
//! heads, snapshot fields) instead of arbitrary `waitForTimeout`s.
//!
//! See `docs/specs/2026-04-27-event-based-waits-design.md`.

#![cfg(feature = "test-hooks")]

use wasm_bindgen::prelude::*;

/// Read-only test instrumentation handle exposed to JS as `window.__willow`.
#[wasm_bindgen]
pub struct WillowTestHooks {
    // ClientHandle field added in Task 2.2.
    _placeholder: (),
}
