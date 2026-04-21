//! # Code rendering — inline pills + fenced blocks
//!
//! Parses message bodies for backtick-delimited code spans and renders
//! them per `docs/specs/2026-04-19-ui-design/message-row.md` §Inline
//! artefacts (fenced code + inline code paragraphs) and §Code.
//!
//! Two flavours:
//!
//! * **Inline code.** Single backtick → mono pill on `--bg-2` / `--line`
//!   / `--ink-1` with 3 px radius, `0 4px` padding. Single-line only —
//!   backticks cannot span a newline.
//! * **Fenced code.** Triple backtick on its own line →
//!   `<pre class="code-fenced">` on `--bg-0` with `--line` border,
//!   8 px radius, `8px 12px` padding, mono 12 px, `max-width: 520px`.
//!   Opening fence may carry an optional `\w+` language token (parsed
//!   but unused in v1 — no highlighting). Includes a 24×24 copy button
//!   in the top-right corner, revealed on block hover (desktop) and
//!   always visible on mobile. The icon flips to a check for 900 ms
//!   after a successful copy.
//!
//! `parse_code_segments` runs in two passes: fences first, then
//! remaining `Text` segments re-run through inline-backtick parsing.
//! That ordering matters — it prevents a backtick inside a fenced body
//! from being treated as an inline span.

use leptos::prelude::*;

/// A single segment produced by [`parse_code_segments`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeSegment {
    /// Plain text (still feeds through the URL auto-link stage).
    Text(String),
    /// Content inside single backticks. Single-line only.
    Inline(String),
    /// Content inside a triple-backtick fence, plus the optional
    /// language token from the opening fence.
    Fenced { lang: Option<String>, body: String },
}

/// Parse a message body into interleaved text / inline-code / fenced
/// segments.
///
/// Two-pass algorithm:
///
/// 1. **Fence pass.** Walk the body looking for lines consisting of
///    exactly ` ``` ` (optionally followed by a `\w+` language token).
///    The closing fence is a line that is exactly ` ``` ` on its own.
///    Unmatched opening fences are treated as literal text — we don't
///    want a stray triple-backtick to eat the rest of the body.
/// 2. **Inline pass.** Every resulting `Text` segment is re-scanned for
///    single-backtick pairs, **single-line only**. Unmatched backticks
///    stay literal.
///
/// Edge cases covered by the module tests:
/// * No backticks at all → single `Text(body)`.
/// * Unmatched single backtick → literal text.
/// * Unmatched triple backtick → literal text.
/// * Three consecutive backticks inline (no surrounding newlines) do
///   not trigger a fence.
pub fn parse_code_segments(body: &str) -> Vec<CodeSegment> {
    let fence_pass = parse_fences(body);
    let mut out = Vec::with_capacity(fence_pass.len());
    for seg in fence_pass {
        match seg {
            CodeSegment::Text(t) => out.extend(parse_inline(&t)),
            other => out.push(other),
        }
    }
    out
}

/// First pass — pull fenced `\`\`\`` blocks out of the body.
///
/// The opening fence rule is deliberately strict so a stray triple
/// backtick mid-line cannot eat the rest of the body:
///
/// * Must be at byte offset 0 *or* preceded by a `\n`.
/// * After the three backticks we accept at most one `\w+` language
///   token, then a literal `\n`.
///
/// The closing fence must match the same constraints (start-of-body or
/// preceded by `\n`) and be exactly ` ``` ` terminated by end-of-body
/// or a `\n`.
fn parse_fences(body: &str) -> Vec<CodeSegment> {
    let bytes = body.as_bytes();
    let mut segments = Vec::new();
    // `text_cursor` marks where the next pending Text segment would start.
    // `scan_cursor` advances through failed opener attempts without
    // dropping characters from the pending Text run — that's how we
    // keep `\`\`\`rust extra\n…` intact as a literal when the lang
    // token is malformed.
    let mut text_cursor = 0usize;
    let mut scan_cursor = 0usize;

    while scan_cursor < bytes.len() {
        let Some(fence_start) = find_fence(body, scan_cursor) else {
            break;
        };

        // Parse the opening fence: `\`\`\`` then optional lang then `\n`.
        let after_backticks = fence_start + 3;
        let rest = &body[after_backticks..];
        let Some(nl) = rest.find('\n') else {
            // Opening fence has no terminating newline — leave literal.
            break;
        };
        let lang_token = rest[..nl].trim();
        let lang = if lang_token.is_empty() {
            None
        } else if lang_token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            Some(lang_token.to_string())
        } else {
            // Junk after the opening fence — skip past these three
            // backticks and keep looking. Do NOT advance `text_cursor`.
            scan_cursor = fence_start + 3;
            continue;
        };

        let body_start = after_backticks + nl + 1;

        let Some(close_rel) = find_closing_fence(body, body_start) else {
            // Unmatched — leave everything from `text_cursor` as text.
            break;
        };
        let fence_body = &body[body_start..close_rel];

        // Flush the text between the previous boundary and the fence.
        if fence_start > text_cursor {
            segments.push(CodeSegment::Text(
                body[text_cursor..fence_start].to_string(),
            ));
        }
        segments.push(CodeSegment::Fenced {
            lang,
            body: fence_body.to_string(),
        });

        let mut next = close_rel + 3;
        if body.as_bytes().get(next) == Some(&b'\n') {
            next += 1;
        }
        text_cursor = next;
        scan_cursor = next;
    }

    // Flush the remainder.
    if text_cursor < bytes.len() {
        segments.push(CodeSegment::Text(body[text_cursor..].to_string()));
    }

    if segments.is_empty() {
        segments.push(CodeSegment::Text(body.to_string()));
    }
    segments
}

/// Find the next opening fence on or after `from`. Returns the byte
/// offset of the first backtick.
///
/// An opening fence must be at the start of the body or preceded by a
/// `\n`, so three backticks mid-line are ignored.
fn find_fence(body: &str, from: usize) -> Option<usize> {
    let mut start = from;
    while let Some(rel) = body[start..].find("```") {
        let idx = start + rel;
        let is_line_start = idx == 0 || body.as_bytes().get(idx - 1) == Some(&b'\n');
        if is_line_start {
            return Some(idx);
        }
        start = idx + 3;
    }
    None
}

/// Find the closing fence starting at or after `from`. The closing
/// fence must be at the start of a line and be exactly ` ``` `
/// terminated by `\n` or end-of-body. Returns the byte offset of the
/// first backtick of the closer.
fn find_closing_fence(body: &str, from: usize) -> Option<usize> {
    let mut start = from;
    while let Some(rel) = body[start..].find("```") {
        let idx = start + rel;
        let is_line_start = idx == from || body.as_bytes().get(idx - 1) == Some(&b'\n');
        // After the three backticks we need `\n` or EOF — no trailing junk.
        let after = idx + 3;
        let terminates = after >= body.len() || body.as_bytes().get(after) == Some(&b'\n');
        if is_line_start && terminates {
            return Some(idx);
        }
        start = after;
    }
    None
}

/// Second pass — split a text segment on single-backtick pairs. Single
/// line only: a backtick run that would span a newline is left literal.
fn parse_inline(text: &str) -> Vec<CodeSegment> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        // Find the next single backtick that is NOT part of a ``` run.
        let Some(open) = find_single_backtick(text, cursor) else {
            break;
        };
        // Search for the matching closer on the same line.
        let after_open = open + 1;
        let Some(close) = find_inline_close(text, after_open) else {
            // Unmatched — stop here, leave the tail literal.
            break;
        };
        // Emit preceding text.
        if open > cursor {
            out.push(CodeSegment::Text(text[cursor..open].to_string()));
        }
        out.push(CodeSegment::Inline(text[after_open..close].to_string()));
        cursor = close + 1;
    }

    if cursor < bytes.len() {
        out.push(CodeSegment::Text(text[cursor..].to_string()));
    }
    if out.is_empty() {
        out.push(CodeSegment::Text(text.to_string()));
    }
    out
}

/// Find the next single backtick from `from`, skipping runs of 2+
/// consecutive backticks entirely. Returns `None` if none found.
fn find_single_backtick(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            // Count the run length.
            let mut j = i;
            while j < bytes.len() && bytes[j] == b'`' {
                j += 1;
            }
            let run = j - i;
            if run == 1 {
                return Some(i);
            }
            // Skip past the whole run — neither open nor close for inline.
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

/// Find the next single-backtick closer at or after `from`, bailing at
/// the first `\n` encountered. Runs of 2+ backticks don't close an
/// inline span.
fn find_inline_close(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => return None,
            b'`' => {
                let mut j = i;
                while j < bytes.len() && bytes[j] == b'`' {
                    j += 1;
                }
                let run = j - i;
                if run == 1 {
                    return Some(i);
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    None
}

/// A single `` `inline` `` code span.
///
/// Styled by `.code-inline` — mono, `--bg-2` background, `--line`
/// border, 3 px radius, `0 4px` padding, `--ink-1` foreground.
#[component]
pub fn InlineCodePill(text: String) -> impl IntoView {
    view! { <code class="code-inline">{text}</code> }
}

/// A fenced triple-backtick code block with a corner copy button.
///
/// `lang` is parsed from the opening fence but intentionally unused in
/// v1 (no syntax highlighting yet — see spec §Code). Pass `""` for
/// "no language". Styling lives on `.code-fenced` / `.code-copy-btn`.
///
/// The prop is typed as a bare `String` (defaulting to `""` via
/// `#[prop(optional)]`) rather than `Option<String>` because Leptos'
/// macro auto-wraps `Option<T>` props in `Some(..)`, which made
/// `lang=lang_option` impossible from the call site. When the Phase
/// 2b highlighting pass lands we can pivot on `lang.is_empty()`.
#[component]
pub fn FencedCodeBlock(body: String, #[prop(optional)] lang: String) -> impl IntoView {
    let _ = lang; // parsed but unused in v1 — future highlighting hook
    let (copied, set_copied) = signal(false);
    let body_for_copy = body.clone();
    let on_copy = move |_| {
        // Reuse the shared clipboard helper so fenced-block copy behaves
        // identically to every other copy surface (server id, invite
        // link, etc.) — HTTPS API first, silent textarea fallback.
        crate::util::copy_to_clipboard(&body_for_copy);
        set_copied.set(true);
        // Reset to the copy icon after 900 ms per spec.
        set_timeout(
            move || set_copied.set(false),
            std::time::Duration::from_millis(900),
        );
    };

    view! {
        <pre class="code-fenced">
            <button
                class="code-copy-btn"
                on:click=on_copy
                aria-label="copy code"
                type="button"
            >
                {move || if copied.get() {
                    crate::icons::icon_check().into_any()
                } else {
                    crate::icons::icon_copy().into_any()
                }}
            </button>
            <code>{body}</code>
        </pre>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_backticks() {
        assert_eq!(
            parse_code_segments("hello world"),
            vec![CodeSegment::Text("hello world".to_string())],
        );
    }

    #[test]
    fn parse_inline_only() {
        assert_eq!(
            parse_code_segments("a `b` c"),
            vec![
                CodeSegment::Text("a ".to_string()),
                CodeSegment::Inline("b".to_string()),
                CodeSegment::Text(" c".to_string()),
            ],
        );
    }

    #[test]
    fn parse_fenced_only() {
        // Fenced block sandwiched by plain text. The leading `a\n`
        // survives verbatim; the fenced body preserves its trailing
        // newline (everything between the fences is the body); the
        // closer eats its own trailing newline, leaving the next
        // segment to start with `b`.
        assert_eq!(
            parse_code_segments("a\n```\nx\n```\nb"),
            vec![
                CodeSegment::Text("a\n".to_string()),
                CodeSegment::Fenced {
                    lang: None,
                    body: "x\n".to_string(),
                },
                CodeSegment::Text("b".to_string()),
            ],
        );
    }

    #[test]
    fn parse_fenced_with_lang() {
        assert_eq!(
            parse_code_segments("```rust\nfn f() {}\n```"),
            vec![CodeSegment::Fenced {
                lang: Some("rust".to_string()),
                body: "fn f() {}\n".to_string(),
            }],
        );
    }

    #[test]
    fn parse_unmatched_backtick() {
        // Unclosed inline — leave the whole string literal.
        assert_eq!(
            parse_code_segments("a `b c"),
            vec![CodeSegment::Text("a `b c".to_string())],
        );
    }

    #[test]
    fn parse_unmatched_fence() {
        // Unclosed fence — leave the whole string literal (don't eat tail).
        assert_eq!(
            parse_code_segments("```\nabc\n"),
            vec![CodeSegment::Text("```\nabc\n".to_string())],
        );
    }

    #[test]
    fn parse_triple_backtick_inline_is_not_fence() {
        // Three backticks surrounded by text on the same line must NOT
        // trigger a fence — fences require start-of-line.
        assert_eq!(
            parse_code_segments("a ``` b"),
            vec![CodeSegment::Text("a ``` b".to_string())],
        );
    }

    #[test]
    fn parse_inline_does_not_span_newline() {
        // A backtick followed by a newline before a closer must NOT
        // open an inline span.
        assert_eq!(
            parse_code_segments("a `b\nc`"),
            vec![CodeSegment::Text("a `b\nc`".to_string())],
        );
    }

    #[test]
    fn parse_mixed_inline_and_fenced() {
        // Mentions→code pipeline relies on this: inline + fenced in
        // the same body, with tail text. Confirms the two-pass flow.
        let body = "foo `bar` baz\n```\nquux\n```";
        let segs = parse_code_segments(body);
        assert_eq!(
            segs,
            vec![
                CodeSegment::Text("foo ".to_string()),
                CodeSegment::Inline("bar".to_string()),
                CodeSegment::Text(" baz\n".to_string()),
                CodeSegment::Fenced {
                    lang: None,
                    body: "quux\n".to_string(),
                },
            ],
        );
    }

    #[test]
    fn parse_fence_with_junk_after_lang_falls_back_to_text() {
        // Opening fence has `rust extra` — only `\w+` is accepted, so
        // treat the opening as literal and re-scan from after the
        // backticks.
        let segs = parse_code_segments("```rust extra\nx\n```");
        // Since we bail after the opener, no fence is recognised; the
        // nested closer doesn't qualify as an opener (preceded by `\n`
        // but the leading three backticks weren't). Result: single
        // Text.  This keeps broken fences from eating the tail.
        assert_eq!(
            segs,
            vec![CodeSegment::Text("```rust extra\nx\n```".to_string())],
        );
    }
}
