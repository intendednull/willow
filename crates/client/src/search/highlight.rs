//! Highlight match-ranges + centred excerpts.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Row anatomy:
//! each result row renders a three-line excerpt centred on the first
//! matched span, with matched ranges wrapped in `<mark>` under the
//! renderer. The byte ranges live on [`super::execute::SearchResult`].
//!
//! Full implementation lands in Task 5; Task 4 ships the
//! signature so the executor can populate `matched_ranges` at emit
//! time.

use super::query::SearchQuery;

/// Excerpt + local-to-excerpt match ranges for render.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Excerpt {
    /// The excerpt text (may start / end with `…`).
    pub text: String,
    /// Byte ranges inside `text` — NOT the original body.
    pub ranges: Vec<(usize, usize)>,
}

/// Find all match ranges in `body` for `query`, sorted + deduped.
///
/// Task 4 ships a minimal substring-walker; Task 5 will extend to
/// token-boundary awareness + phrase scoring.
pub fn match_ranges(body: &str, query: &SearchQuery) -> Vec<(usize, usize)> {
    let body_lc = body.to_lowercase();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for tok in &query.tokens {
        if tok.is_empty() {
            continue;
        }
        let mut cursor = 0usize;
        while let Some(p) = body_lc[cursor..].find(tok) {
            let start = cursor + p;
            let end = start + tok.len();
            out.push((start, end));
            cursor = end;
        }
    }
    for ph in &query.phrases {
        if ph.is_empty() {
            continue;
        }
        let mut cursor = 0usize;
        while let Some(p) = body_lc[cursor..].find(ph) {
            let start = cursor + p;
            let end = start + ph.len();
            out.push((start, end));
            cursor = end;
        }
    }
    merge_overlaps(out)
}

/// Build a three-line-ish excerpt centred on the first matched span,
/// translating ranges into excerpt-local byte offsets. Task 5 will
/// refine the truncation rules.
pub fn build_excerpt(body: &str, ranges: &[(usize, usize)], context_chars: usize) -> Excerpt {
    let Some(&(first_start, first_end)) = ranges.first() else {
        return Excerpt {
            text: body.to_string(),
            ranges: vec![],
        };
    };

    // Walk back `context_chars` codepoints from `first_start`.
    let start_byte = body[..first_start]
        .char_indices()
        .rev()
        .nth(context_chars)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end_byte = body[first_end..]
        .char_indices()
        .nth(context_chars)
        .map(|(i, _)| first_end + i)
        .unwrap_or(body.len());

    let mut text = String::new();
    let left_pad = start_byte > 0;
    let right_pad = end_byte < body.len();
    if left_pad {
        text.push('…');
    }
    text.push_str(&body[start_byte..end_byte]);
    if right_pad {
        text.push('…');
    }

    let base_offset = if left_pad { '…'.len_utf8() } else { 0 };
    let out_ranges: Vec<(usize, usize)> = ranges
        .iter()
        .filter(|&&(a, b)| a >= start_byte && b <= end_byte)
        .map(|&(a, b)| (a - start_byte + base_offset, b - start_byte + base_offset))
        .collect();

    Excerpt {
        text,
        ranges: out_ranges,
    }
}

/// Merge overlapping byte ranges.
fn merge_overlaps(mut r: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    r.sort_by_key(|&(a, _)| a);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (a, b) in r {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}
