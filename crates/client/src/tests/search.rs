//! Focused contract tests for [`SearchIndexHandle::set_config`] —
//! audit F42 (issue #542).
//!
//! Prior to this file the only direct coverage of `set_config` lived in
//! `crates/client/src/search/tests.rs::handle_tests`, where each test
//! exercised one *side* of the contract (insert gating, recents gating,
//! grove opt-out gating). The audit asked for a focused test that locks
//! in `set_config` itself, independently of Effect wiring or the
//! `Insert` handler's own gating logic.
//!
//! Two complementary assertions live here:
//!
//! 1. [`set_config_round_trip`] — the minimum contract: after
//!    `set_config(c)`, a subsequent `config().await` returns exactly
//!    `c`. This pins the read/write symmetry of the actor's `SetConfig`
//!    + `GetConfig` handler pair.
//!
//! 2. [`set_config_changes_rebuild_query_results`] — the only path by
//!    which `set_config` produces an *observable query-result delta*
//!    against a previously-indexed corpus is via a subsequent
//!    `rebuild`, because the executor itself does not consult config
//!    (gating happens at write time in `SearchActor::message_allowed`).
//!    This test indexes a two-grove corpus, asserts both groves are
//!    queryable, then opts grove `g1` out via `set_config`, rebuilds,
//!    and asserts `g1`'s message no longer appears while `g0`'s still
//!    does. That sequence proves `set_config` actually mutated the
//!    actor's config snapshot in a way the next `rebuild` observes.
//!
//! Why not assert a query-result delta directly after `set_config`
//! without a rebuild? Because the executor in
//! `crates/client/src/search/execute.rs` reads only the index and the
//! query — never the config. Asserting an immediate delta would
//! require either changing production behaviour (queries consulting
//! config) or testing a contract the code doesn't make. We pick the
//! existing contract instead: rebuild reflects the latest config.

use willow_actor::System;
use willow_identity::Identity;

use crate::search::{
    parse_query, IndexableMessage, SearchIndexConfig, SearchIndexHandle, SearchScope,
};

fn mk(id: &str, body: &str, grove: &str) -> IndexableMessage {
    IndexableMessage {
        message_id: id.into(),
        channel_id: format!("c-{grove}"),
        channel_name: "general".into(),
        grove_id: Some(grove.into()),
        letter_id: None,
        author_peer_id: Identity::generate().endpoint_id(),
        author_handle: "mira".into(),
        author_display_name: "Mira".into(),
        timestamp_ms: 100,
        body: body.into(),
        has_image: false,
        has_file: false,
        has_link: false,
    }
}

/// Round-trip: writing a config and reading it back returns the same
/// value. Pins the `SetConfig` + `GetConfig` handler pair as a single
/// atomic read-after-write.
#[tokio::test]
async fn set_config_round_trip() {
    let sys = System::new();
    let h = SearchIndexHandle::new_in_memory(&sys.handle());

    let mut per_grove_enabled = std::collections::HashMap::new();
    per_grove_enabled.insert("g-quiet".into(), false);
    per_grove_enabled.insert("g-loud".into(), true);
    let cfg = SearchIndexConfig {
        enabled: false,
        horizon_days: 30,
        remember_recents: false,
        per_grove_enabled,
    };

    h.set_config(cfg.clone());
    assert_eq!(h.config().await, cfg);
}

/// `set_config` followed by `rebuild` must reflect the new config in
/// query results: a grove opted out via `per_grove_enabled` after the
/// initial index build disappears from queries once the index is
/// rebuilt against the same corpus.
#[tokio::test]
async fn set_config_changes_rebuild_query_results() {
    let sys = System::new();
    let h = SearchIndexHandle::new_in_memory(&sys.handle());

    // Two messages, one per grove, both containing the term `signal`.
    let corpus = vec![
        mk("m-keep", "signal here", "g0"),
        mk("m-drop", "signal there", "g1"),
    ];
    h.rebuild(corpus.clone()).await;

    // Baseline: both groves visible.
    let q = parse_query("signal");
    let pre = h.query(&q, &SearchScope::AllGrovesAndLetters).await;
    let pre_ids: Vec<_> = pre.iter().map(|r| r.message_id.clone()).collect();
    assert_eq!(
        pre.len(),
        2,
        "baseline must surface both groves: {pre_ids:?}"
    );
    assert!(pre_ids.contains(&"m-keep".into()));
    assert!(pre_ids.contains(&"m-drop".into()));

    // Opt grove `g1` out, then rebuild against the same corpus.
    let mut cfg = h.config().await;
    cfg.per_grove_enabled.insert("g1".into(), false);
    h.set_config(cfg);
    h.rebuild(corpus).await;

    // Post-rebuild the opted-out grove's message is gone; the kept
    // grove's message still hits.
    let post = h.query(&q, &SearchScope::AllGrovesAndLetters).await;
    let post_ids: Vec<_> = post.iter().map(|r| r.message_id.clone()).collect();
    assert_eq!(
        post.len(),
        1,
        "after set_config + rebuild only g0 must remain: {post_ids:?}"
    );
    assert_eq!(post[0].message_id, "m-keep");
}
