//! Unit tests for the search module. One sub-module per file — see
//! each sub-module's doc for the behaviour it covers.

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
