//! Query grammar for local search.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Query language:
//! plain text + quoted phrases + prefix operators, case-insensitive. Each
//! parse returns a [`SearchQuery`] carrying token predicates, exact-phrase
//! predicates, operator filters, and a list of [`QueryWarning`]s for
//! malformed operators (which are treated as plain text per spec — see
//! `local-search.md` §Query language → "malformed operator").

use chrono::NaiveDate;

/// Prefix-operator filters that narrow a query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryFilters {
    /// `from:@peer` — author display name or handle equals `peer` (`@`
    /// is optional).
    pub from_author: Option<String>,
    /// `in:#channel` — channel name equals `channel` (`#` is optional).
    pub in_channel: Option<String>,
    /// `since:YYYY-MM-DD` — timestamp `>=` local-midnight on that date.
    pub since: Option<NaiveDate>,
    /// `before:YYYY-MM-DD` — timestamp `<` local-midnight on that date.
    pub before: Option<NaiveDate>,
    /// `has:image` — message has an image attachment.
    pub has_image: bool,
    /// `has:file` — message has a non-image file attachment.
    pub has_file: bool,
    /// `has:link` — message body contains a URL.
    pub has_link: bool,
}

/// One parsed warning — feeds the UI's
/// `unknown filter — treated as plain text` tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryWarning {
    /// An operator was unknown or malformed (treated as plain text).
    UnknownOperator {
        /// The offending span as it appeared in the raw input.
        span: String,
    },
}

/// A fully-parsed query.
///
/// `tokens` and `phrases` are both lowercased so the execute stage can
/// compare against a lowercased body without re-walking the text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchQuery {
    /// Whitespace-separated tokens; every token must appear in the body
    /// (AND-joined).
    pub tokens: Vec<String>,
    /// Quoted-phrase predicates; each must appear as a contiguous
    /// substring in the body.
    pub phrases: Vec<String>,
    /// Operator filters.
    pub filters: QueryFilters,
    /// Warnings for malformed operators.
    pub warnings: Vec<QueryWarning>,
    /// Raw echo before parsing. Used by the UI so the visible query
    /// preserves the user's exact casing.
    pub raw: String,
}

/// Parse `raw` into a [`SearchQuery`].
///
/// The grammar is tolerant: malformed operators fall through as plain
/// tokens plus a [`QueryWarning::UnknownOperator`] entry. Token matching
/// is case-insensitive at the tokenizer level; phrases are stored
/// lower-cased here and compared lower-cased at execute time.
pub fn parse_query(raw: &str) -> SearchQuery {
    let mut out = SearchQuery {
        raw: raw.to_string(),
        ..SearchQuery::default()
    };

    // Step 1: lift out quoted phrases so they don't split on whitespace.
    //
    // Walk char-by-char and track whether we're inside a `"..."` run.
    // Unclosed quotes fall through as plain tokens (tolerant).
    let mut rest = String::with_capacity(raw.len());
    let mut in_quote = false;
    let mut phrase = String::new();
    for c in raw.chars() {
        match (in_quote, c) {
            (false, '"') => in_quote = true,
            (true, '"') => {
                let trimmed = phrase.trim().to_lowercase();
                if !trimmed.is_empty() {
                    out.phrases.push(trimmed);
                }
                phrase.clear();
                in_quote = false;
            }
            (true, c) => phrase.push(c),
            (false, c) => rest.push(c),
        }
    }
    // Rescue unclosed phrase: treat its content as plain tokens so the
    // user still sees something.
    if in_quote && !phrase.is_empty() {
        rest.push(' ');
        rest.push_str(&phrase);
    }

    // Step 2: walk whitespace-separated tokens and dispatch each to an
    // operator or plain-token bucket.
    for span in rest.split_whitespace() {
        if let Some(rest) = span.strip_prefix("from:") {
            let h = rest.strip_prefix('@').unwrap_or(rest).to_lowercase();
            if h.is_empty() {
                out.tokens.push(span.to_lowercase());
                out.warnings.push(QueryWarning::UnknownOperator {
                    span: span.to_string(),
                });
            } else {
                out.filters.from_author = Some(h);
            }
        } else if let Some(rest) = span.strip_prefix("in:") {
            let ch = rest.strip_prefix('#').unwrap_or(rest).to_lowercase();
            if ch.is_empty() {
                out.tokens.push(span.to_lowercase());
                out.warnings.push(QueryWarning::UnknownOperator {
                    span: span.to_string(),
                });
            } else {
                out.filters.in_channel = Some(ch);
            }
        } else if let Some(rest) = span.strip_prefix("since:") {
            match NaiveDate::parse_from_str(rest, "%Y-%m-%d") {
                Ok(d) => out.filters.since = Some(d),
                Err(_) => {
                    out.tokens.push(span.to_lowercase());
                    out.warnings.push(QueryWarning::UnknownOperator {
                        span: span.to_string(),
                    });
                }
            }
        } else if let Some(rest) = span.strip_prefix("before:") {
            match NaiveDate::parse_from_str(rest, "%Y-%m-%d") {
                Ok(d) => out.filters.before = Some(d),
                Err(_) => {
                    out.tokens.push(span.to_lowercase());
                    out.warnings.push(QueryWarning::UnknownOperator {
                        span: span.to_string(),
                    });
                }
            }
        } else if span == "has:image" {
            out.filters.has_image = true;
        } else if span == "has:file" {
            out.filters.has_file = true;
        } else if span == "has:link" {
            out.filters.has_link = true;
        } else if let Some(op_prefix) = detect_unknown_prefix(span) {
            // `foo:bar` that isn't a known prefix or a URL scheme → warning
            // + plain-text fallback per spec §Query language.
            out.tokens.push(span.to_lowercase());
            out.warnings.push(QueryWarning::UnknownOperator {
                span: op_prefix.to_string(),
            });
        } else {
            out.tokens.push(span.to_lowercase());
        }
    }

    out
}

/// Return the whole `span` when it looks like a `foo:bar` operator but
/// the prefix is neither a known search operator nor a URL scheme. URL
/// schemes are excluded so pasting a link into the query doesn't trip
/// the warning path.
fn detect_unknown_prefix(span: &str) -> Option<&str> {
    let idx = span.find(':')?;
    let prefix = &span[..idx + 1];
    if matches!(
        prefix,
        "from:"
            | "in:"
            | "since:"
            | "before:"
            | "has:"
            | "http:"
            | "https:"
            | "mailto:"
            | "ftp:"
            | "ws:"
            | "wss:"
            | "file:"
            | "data:"
            | "javascript:"
    ) {
        return None;
    }
    Some(span)
}
