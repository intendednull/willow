//! Tokenizer for local search.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Query language:
//! plain-text tokens split on whitespace + punctuation, case-insensitive.
//! This tokenizer additionally preserves `@handle`, `#channel`, and URL
//! shapes as single tokens so the operator filters (`from:`, `in:`,
//! `has:link`) can match them directly against the indexed postings.
//!
//! Sigil-prefixed tokens (`@mira`, `#general`) are ALSO emitted as their
//! stemmed form (`mira`, `general`) so a plain-text search for `mira`
//! still lands the same messages as `from:@mira` would.
//!
//! ASCII-lowercased. Unicode code points flow through
//! [`char::is_alphanumeric`], so non-Latin scripts tokenize correctly as
//! long as they are treated as alphanumeric by that predicate.

/// Return searchable tokens for `body`, lowercased and de-sigiled.
pub fn tokenize(body: &str) -> Vec<String> {
    token_positions(body)
        .into_iter()
        .flat_map(|(_, t)| expand(t))
        .collect()
}

/// Return `(byte_offset, token)` pairs, one per sigil-preserving token.
///
/// Used by [`tokenize`] above and by [`super::highlight`] to map
/// matched tokens back to byte spans for the `<mark>` wrapper.
pub fn token_positions(body: &str) -> Vec<(usize, String)> {
    let bytes = body.as_bytes();
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        // Skip non-token bytes. `char_len` is byte length of the
        // current char — important for multibyte input.
        let c = match body[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        if !is_token_start(&body[i..]) {
            i += c.len_utf8();
            continue;
        }

        let start = i;
        if is_url_start(&body[i..]) {
            // URL: consume everything up to the first whitespace.
            while i < bytes.len() {
                let ch = match body[i..].chars().next() {
                    Some(c) => c,
                    None => break,
                };
                if ch.is_whitespace() {
                    break;
                }
                i += ch.len_utf8();
            }
        } else if bytes[i] == b'@' || bytes[i] == b'#' {
            // Sigil token: consume sigil + handle-char run.
            i += 1;
            while i < bytes.len() {
                let ch = match body[i..].chars().next() {
                    Some(c) => c,
                    None => break,
                };
                if is_handle_char(ch) {
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
        } else {
            // Plain token: alnum + `-` + `_` + `'` + multibyte alpha.
            while i < bytes.len() {
                let ch = match body[i..].chars().next() {
                    Some(c) => c,
                    None => break,
                };
                if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '\'' {
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
        }

        if start < i {
            let raw = &body[start..i];
            // Drop trailing `'` if it's dangling (handles `it'` tail).
            let trimmed = raw.trim_end_matches('\'');
            if !trimmed.is_empty() {
                out.push((start, trimmed.to_lowercase()));
            }
        }
    }

    out
}

/// Is the run starting at `s` a valid token-start?
fn is_token_start(s: &str) -> bool {
    let Some(c) = s.chars().next() else {
        return false;
    };
    c.is_alphanumeric() || c == '@' || c == '#' || is_url_start(s)
}

/// Does the run starting at `s` begin a URL shape we want to keep
/// together as one token?
fn is_url_start(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("mailto:")
}

fn is_handle_char(c: char) -> bool {
    c.is_alphanumeric() || c == '.' || c == '_' || c == '-'
}

/// Sigil-prefixed tokens expand to `[sigil+stem, stem]` so plain-text
/// search lands the same message as an operator search would.
fn expand(tok: String) -> Vec<String> {
    if let Some(stem) = tok.strip_prefix('@').or_else(|| tok.strip_prefix('#')) {
        if stem.is_empty() {
            vec![tok]
        } else {
            vec![tok.clone(), stem.to_string()]
        }
    } else {
        vec![tok]
    }
}
