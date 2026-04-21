//! Unit tests for the search module. One sub-module per file — see
//! each sub-module's doc for the behaviour it covers.

mod execute_tests {
    use super::super::execute::*;
    use super::super::index::*;
    use super::super::query::*;
    use willow_identity::Identity;

    #[allow(clippy::too_many_arguments)]
    fn mk(
        id: &str,
        body: &str,
        cid: &str,
        chname: &str,
        ts: u64,
        author: &str,
        handle: &str,
        grove: Option<&str>,
        letter: Option<&str>,
        img: bool,
        file: bool,
        link: bool,
    ) -> IndexableMessage {
        IndexableMessage {
            message_id: id.into(),
            channel_id: cid.into(),
            channel_name: chname.into(),
            grove_id: grove.map(String::from),
            letter_id: letter.map(String::from),
            author_peer_id: Identity::generate().endpoint_id(),
            author_handle: handle.into(),
            author_display_name: author.into(),
            timestamp_ms: ts,
            body: body.into(),
            has_image: img,
            has_file: file,
            has_link: link,
        }
    }

    fn seed_index() -> SearchIndex {
        let mut idx = SearchIndex::new();
        idx.insert(mk(
            "m1",
            "hello world",
            "c1",
            "general",
            100,
            "Mira",
            "mira",
            Some("g0"),
            None,
            false,
            false,
            false,
        ));
        idx.insert(mk(
            "m2",
            "hello everyone",
            "c2",
            "random",
            200,
            "Jun",
            "jun",
            Some("g0"),
            None,
            false,
            false,
            false,
        ));
        idx.insert(mk(
            "m3",
            "see https://ok",
            "c1",
            "general",
            300,
            "Mira",
            "mira",
            Some("g0"),
            None,
            false,
            false,
            true,
        ));
        idx.insert(mk(
            "m4",
            "letter text",
            "l1",
            "letter",
            400,
            "Jun",
            "jun",
            None,
            Some("l1"),
            false,
            false,
            false,
        ));
        idx.insert(mk(
            "m5",
            "two words here",
            "c1",
            "general",
            500,
            "Mira",
            "mira",
            Some("g0"),
            None,
            false,
            false,
            false,
        ));
        idx
    }

    #[test]
    fn scope_this_channel_only_matches_that_channel() {
        let idx = seed_index();
        let q = parse_query("hello");
        let hits = execute(&idx, &q, &SearchScope::ThisChannel("c1".into()));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m1");
    }

    #[test]
    fn scope_all_letters_excludes_grove_channels() {
        let idx = seed_index();
        let q = parse_query("text");
        let hits = execute(&idx, &q, &SearchScope::AllLetters);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m4");
    }

    #[test]
    fn scope_all_groves_and_letters_matches_both() {
        let idx = seed_index();
        let q = parse_query("hello");
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        let ids: Vec<_> = hits.iter().map(|h| h.message_id.clone()).collect();
        assert!(ids.contains(&"m1".into()));
        assert!(ids.contains(&"m2".into()));
    }

    #[test]
    fn quoted_phrase_matches_adjacent_only() {
        let idx = seed_index();
        let q = parse_query(r#""two words""#);
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m5");
    }

    #[test]
    fn quoted_phrase_requires_adjacency() {
        let idx = seed_index();
        // "hello words" — not adjacent anywhere in the corpus.
        let q = parse_query(r#""hello words""#);
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        assert!(hits.is_empty());
    }

    #[test]
    fn from_filter_narrows_by_author() {
        let idx = seed_index();
        let q = parse_query("hello from:@jun");
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m2");
    }

    #[test]
    fn in_filter_narrows_by_channel() {
        let idx = seed_index();
        let q = parse_query("hello in:#general");
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m1");
    }

    #[test]
    fn has_link_filter() {
        let idx = seed_index();
        // Pure-operator query: no tokens, no phrases. Executor must
        // fall through to `all_postings()` then apply the filter.
        let q = parse_query("has:link");
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m3");
    }

    #[test]
    fn results_ordered_desc_by_timestamp() {
        let idx = seed_index();
        let q = parse_query("hello");
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        assert!(hits.windows(2).all(|w| w[0].timestamp_ms >= w[1].timestamp_ms));
    }

    #[test]
    fn matched_ranges_populated_on_hit() {
        let idx = seed_index();
        let q = parse_query("hello");
        let hits = execute(&idx, &q, &SearchScope::ThisChannel("c1".into()));
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].matched_ranges.is_empty());
    }

    #[test]
    fn since_before_filter_ranges() {
        use chrono::NaiveDate;
        // Seed with a millisecond-epoch timestamp that's well past
        // any local-tz offset edge case — 2026-04-20 UTC is
        // comfortably inside every local-tz window.
        let ts_2026 = 1_776_326_400_000; // 2026-04-16 04:00 UTC
        let mut idx = SearchIndex::new();
        idx.insert(mk(
            "m1",
            "hello world",
            "c1",
            "general",
            ts_2026,
            "Mira",
            "mira",
            Some("g0"),
            None,
            false,
            false,
            false,
        ));
        idx.insert(mk(
            "m2",
            "hello there",
            "c1",
            "general",
            ts_2026 + 86_400_000,
            "Jun",
            "jun",
            Some("g0"),
            None,
            false,
            false,
            false,
        ));

        let mut q = parse_query("hello");
        q.filters.since = Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap());
        q.filters.before = Some(NaiveDate::from_ymd_opt(2030, 1, 1).unwrap());
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        assert!(hits.len() >= 2, "expected ≥ 2 hits, got {}", hits.len());

        // Tight window that excludes everything.
        let mut q = parse_query("hello");
        q.filters.since = Some(NaiveDate::from_ymd_opt(2030, 1, 1).unwrap());
        let hits = execute(&idx, &q, &SearchScope::AllGrovesAndLetters);
        assert!(hits.is_empty());
    }
}

mod index_tests {
    use super::super::index::*;
    use willow_identity::Identity;

    fn mk(id: &str, body: &str, ts: u64, cid: &str) -> IndexableMessage {
        IndexableMessage {
            message_id: id.into(),
            channel_id: cid.into(),
            channel_name: cid.into(),
            grove_id: Some("g0".into()),
            letter_id: None,
            author_peer_id: Identity::generate().endpoint_id(),
            author_handle: "mira".into(),
            author_display_name: "Mira".into(),
            timestamp_ms: ts,
            body: body.into(),
            has_image: false,
            has_file: false,
            has_link: false,
        }
    }

    #[test]
    fn insert_then_lookup() {
        let mut idx = SearchIndex::new();
        idx.insert(mk("m1", "hello world", 100, "general"));
        assert_eq!(idx.message_count(), 1);
        assert!(idx.postings_for("hello").is_some());
        assert!(idx.postings_for("world").is_some());
    }

    #[test]
    fn insert_is_idempotent() {
        // Re-inserting the same `message_id` must not double-count —
        // live-arrival + batch-rebuild paths overlap in practice.
        let mut idx = SearchIndex::new();
        idx.insert(mk("m1", "hello world", 100, "general"));
        idx.insert(mk("m1", "hello world", 100, "general"));
        assert_eq!(idx.message_count(), 1);
    }

    #[test]
    fn remove_message_unthreads_all_tokens() {
        let mut idx = SearchIndex::new();
        idx.insert(mk("m1", "hello world", 100, "general"));
        idx.remove_message("m1");
        assert_eq!(idx.message_count(), 0);
        assert!(idx
            .postings_for("hello")
            .map(|p| p.is_empty())
            .unwrap_or(true));
    }

    #[test]
    fn remove_channel_drops_all_messages_in_channel() {
        let mut idx = SearchIndex::new();
        idx.insert(mk("m1", "hello", 100, "general"));
        idx.insert(mk("m2", "world", 100, "other"));
        idx.remove_channel("general");
        assert_eq!(idx.message_count(), 1);
    }

    #[test]
    fn remove_grove_drops_all_messages_in_grove() {
        let mut idx = SearchIndex::new();
        let mut m = mk("m1", "hello", 100, "general");
        m.grove_id = Some("grove-a".into());
        idx.insert(m);
        let mut m2 = mk("m2", "world", 100, "general");
        m2.grove_id = Some("grove-b".into());
        idx.insert(m2);
        idx.remove_grove("grove-a");
        assert_eq!(idx.message_count(), 1);
    }

    #[test]
    fn evict_older_than_drops_old_messages() {
        let mut idx = SearchIndex::new();
        idx.insert(mk("old", "old", 100, "general"));
        idx.insert(mk("new", "new", 10_000, "general"));
        idx.evict_older_than(1_000);
        assert_eq!(idx.message_count(), 1);
        assert!(idx.postings_for("new").is_some());
        assert!(idx
            .postings_for("old")
            .map(|p| p.is_empty())
            .unwrap_or(true));
    }

    #[test]
    fn author_synthetic_tokens_indexed() {
        // `from:@mira` execute-time lookups rely on `@mira` + `mira`
        // being synthetic tokens on every message.
        let mut idx = SearchIndex::new();
        idx.insert(mk("m1", "hello there", 100, "general"));
        assert!(idx.postings_for("@mira").is_some());
        assert!(idx.postings_for("mira").is_some());
    }

    #[test]
    fn all_postings_dedups_by_id() {
        // `hello world` has two body tokens plus several author /
        // channel synthetic tokens. `all_postings` must return the
        // message exactly once.
        let mut idx = SearchIndex::new();
        idx.insert(mk("m1", "hello world", 100, "general"));
        assert_eq!(idx.all_postings().len(), 1);
    }
}

mod tokenize_tests {
    use super::super::tokenize::*;

    #[test]
    fn empty_body_yields_empty() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn splits_on_whitespace() {
        assert_eq!(tokenize("hello world"), vec!["hello", "world"]);
    }

    #[test]
    fn splits_on_punctuation() {
        assert_eq!(tokenize("hello, world!"), vec!["hello", "world"]);
    }

    #[test]
    fn lowercases_all_tokens() {
        assert_eq!(tokenize("Hello WORLD"), vec!["hello", "world"]);
    }

    #[test]
    fn preserves_mention_token() {
        // `@mira` stays as a single token so `from:@mira` filtering
        // can match it. Body search still sees the `mira` stem as a
        // token too so plain-text queries hit it.
        let toks = tokenize("hello @mira there");
        assert!(toks.contains(&"@mira".to_string()));
        assert!(toks.contains(&"mira".to_string()));
    }

    #[test]
    fn preserves_channel_token() {
        let toks = tokenize("moved to #general");
        assert!(toks.contains(&"#general".to_string()));
        assert!(toks.contains(&"general".to_string()));
    }

    #[test]
    fn preserves_url_as_single_token() {
        let toks = tokenize("see https://willow.im");
        assert!(toks.contains(&"https://willow.im".to_string()));
    }

    #[test]
    fn token_positions_returns_byte_offsets() {
        let pairs = token_positions("hello world");
        assert_eq!(pairs, vec![(0, "hello".into()), (6, "world".into())]);
    }

    #[test]
    fn token_positions_handles_multibyte() {
        // `héllo` is 5 chars but 6 bytes — token_positions must stay
        // byte-addressable without truncating the trailing `o`.
        let body = "héllo";
        let pairs = token_positions(body);
        assert_eq!(pairs, vec![(0, "héllo".into())]);
    }

    #[test]
    fn plain_then_mention_then_plain() {
        let toks = tokenize("hi @mira bye");
        assert!(toks.contains(&"hi".to_string()));
        assert!(toks.contains(&"@mira".to_string()));
        assert!(toks.contains(&"mira".to_string()));
        assert!(toks.contains(&"bye".to_string()));
    }

    #[test]
    fn apostrophe_inside_word_is_part_of_token() {
        let toks = tokenize("it's fine");
        // `it's` is a single token, not `it` + `s`.
        assert!(toks.contains(&"it's".to_string()));
        assert!(toks.contains(&"fine".to_string()));
    }
}

mod query_tests {
    use super::super::query::*;
    use chrono::NaiveDate;

    #[test]
    fn empty_query_is_no_op() {
        let q = parse_query("");
        assert!(q.tokens.is_empty());
        assert!(q.phrases.is_empty());
        assert_eq!(q.filters, QueryFilters::default());
        assert!(q.warnings.is_empty());
    }

    #[test]
    fn plain_text_tokens_split_on_whitespace() {
        let q = parse_query("hello world");
        assert_eq!(q.tokens, vec!["hello", "world"]);
    }

    #[test]
    fn tokens_lowercased() {
        let q = parse_query("HELLO World");
        assert_eq!(q.tokens, vec!["hello", "world"]);
    }

    #[test]
    fn quoted_phrase_single() {
        let q = parse_query(r#""two words""#);
        assert_eq!(q.phrases, vec!["two words"]);
        assert!(q.tokens.is_empty());
    }

    #[test]
    fn quoted_phrase_mixed_with_tokens() {
        let q = parse_query(r#"hello "two words" world"#);
        assert_eq!(q.tokens, vec!["hello", "world"]);
        assert_eq!(q.phrases, vec!["two words"]);
    }

    #[test]
    fn from_operator_with_at() {
        let q = parse_query("from:@mira");
        assert_eq!(q.filters.from_author, Some("mira".into()));
    }

    #[test]
    fn from_operator_without_at() {
        let q = parse_query("from:mira");
        assert_eq!(q.filters.from_author, Some("mira".into()));
    }

    #[test]
    fn in_operator() {
        let q = parse_query("in:#general");
        assert_eq!(q.filters.in_channel, Some("general".into()));
    }

    #[test]
    fn since_operator_parses_date() {
        let q = parse_query("since:2026-04-01");
        assert_eq!(
            q.filters.since,
            Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap())
        );
    }

    #[test]
    fn before_operator_parses_date() {
        let q = parse_query("before:2026-04-21");
        assert_eq!(
            q.filters.before,
            Some(NaiveDate::from_ymd_opt(2026, 4, 21).unwrap())
        );
    }

    #[test]
    fn has_image_operator() {
        let q = parse_query("has:image");
        assert!(q.filters.has_image);
    }

    #[test]
    fn has_file_operator() {
        let q = parse_query("has:file");
        assert!(q.filters.has_file);
    }

    #[test]
    fn has_link_operator() {
        let q = parse_query("has:link");
        assert!(q.filters.has_link);
    }

    #[test]
    fn unknown_prefix_treated_as_text_with_warning() {
        let q = parse_query("since:yesterday");
        assert_eq!(q.tokens, vec!["since:yesterday"]);
        assert_eq!(q.warnings.len(), 1);
        assert!(matches!(
            &q.warnings[0],
            QueryWarning::UnknownOperator { span } if span == "since:yesterday"
        ));
    }

    #[test]
    fn operator_mixed_with_text() {
        let q = parse_query("from:@mira hello world in:#general");
        assert_eq!(q.tokens, vec!["hello", "world"]);
        assert_eq!(q.filters.from_author, Some("mira".into()));
        assert_eq!(q.filters.in_channel, Some("general".into()));
    }

    #[test]
    fn url_in_query_does_not_trip_unknown_warning() {
        let q = parse_query("https://willow.im/docs");
        assert!(q.warnings.is_empty());
        assert_eq!(q.tokens, vec!["https://willow.im/docs"]);
    }

    #[test]
    fn raw_echo_preserved() {
        let q = parse_query("Hello");
        assert_eq!(q.raw, "Hello");
    }
}
