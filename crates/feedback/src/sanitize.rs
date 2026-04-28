//! User-supplied content sanitization for feedback issues.
//!
//! - `wrap_body_fenced` wraps the user body in a backtick code block
//!   long enough that no closing fence inside the body can escape.
//! - `sanitize_title` strips control / bidi codepoints and escapes
//!   leading brackets so the assembled title can't impersonate the
//!   metadata block.

use regex::Regex;
use std::sync::OnceLock;

/// Match a CommonMark closing-fence line for backtick fences:
/// 0–3 leading spaces, three or more backticks, optional trailing
/// whitespace, end of line.
fn close_fence_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[ ]{0,3}(`{3,})[ \t]*$").unwrap())
}

/// Wrap `body` in a backtick fenced markdown block with the `text`
/// info-string. Fence length is the smallest N ≥ 3 such that no line
/// in the body is `^[ ]{0,3}` `` ` ``{N,}` `[ \t]*$` AND no run of
/// backticks anywhere in the body is ≥ N — guaranteeing no body
/// content can close our fence.
///
/// CRLF line endings are normalized to LF before scanning and in the
/// output.
pub fn wrap_body_fenced(body: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let mut max_run: usize = 0;
    for line in body.split('\n') {
        if let Some(c) = close_fence_re().captures(line) {
            let n = c.get(1).unwrap().as_str().len();
            if n > max_run {
                max_run = n;
            }
        }
    }
    let mut current_run: usize = 0;
    for ch in body.chars() {
        if ch == '`' {
            current_run += 1;
            if current_run > 3 && current_run > max_run {
                max_run = current_run;
            }
        } else {
            current_run = 0;
        }
    }
    let fence_len = std::cmp::max(3, max_run + 1);
    let fence = "`".repeat(fence_len);
    format!("{fence}text\n{body}\n{fence}")
}

/// Sanitize a feedback title. Strips ASCII control codepoints
/// (0x00–0x1F, 0x7F) and Unicode bidi/RTL override codepoints
/// (U+202A..=U+202E, U+2066..=U+2069). Collapses internal
/// runs of whitespace to single spaces. Escapes `[` and `]`
/// with a backslash so the assembled title can't impersonate the
/// worker's metadata-block prefix.
pub fn sanitize_title(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_ws = false;
    for ch in raw.chars() {
        let c = ch as u32;
        let is_ascii_control = c <= 0x1F || c == 0x7F;
        let is_bidi_override = matches!(c, 0x202A..=0x202E | 0x2066..=0x2069);
        if is_ascii_control || is_bidi_override {
            continue;
        }
        if ch.is_whitespace() {
            if !last_was_ws && !out.is_empty() {
                out.push(' ');
            }
            last_was_ws = true;
        } else {
            last_was_ws = false;
            if ch == '[' {
                out.push_str(r"\[");
            } else if ch == ']' {
                out.push_str(r"\]");
            } else {
                out.push(ch);
            }
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: assert the wrapped body is well-formed and the inner
    /// content survives byte-for-byte (modulo CRLF normalization).
    fn assert_wrap_round_trips(input: &str) {
        let wrapped = wrap_body_fenced(input);
        let normalized = input.replace("\r\n", "\n");
        assert!(wrapped.starts_with('`'), "must open with backticks");
        assert!(
            wrapped.contains(&normalized),
            "wrapped body must contain the normalized input verbatim"
        );
    }

    #[test]
    fn wraps_plain_body_with_min_three_backticks() {
        let out = wrap_body_fenced("hello world");
        assert!(out.starts_with("```text\n"));
        assert!(out.ends_with("\n```"));
    }

    #[test]
    fn escapes_body_containing_three_backticks() {
        let body = "code: ```\nrust\n```\nend";
        let out = wrap_body_fenced(body);
        // Must use at least 4 backticks since body has runs of 3.
        assert!(out.starts_with("````text\n"));
        assert!(out.ends_with("\n````"));
        assert!(out.contains(body));
    }

    #[test]
    fn handles_indented_closing_fence() {
        // Up-to-3-space indent counts as a valid close per CommonMark.
        let body = "stuff\n   ```\nmore";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("````text\n"));
    }

    #[test]
    fn ignores_four_space_indent() {
        // 4+ spaces before backticks is a code block, not a fence.
        let body = "stuff\n    ```\nmore";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("```text\n"), "no escalation needed");
    }

    #[test]
    fn handles_crlf_line_endings() {
        let body = "line1\r\n```\r\nline3";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("````text\n"));
        // Wrapped output uses LF only.
        assert!(!out.contains("\r\n"));
    }

    #[test]
    fn ignores_tilde_fences() {
        let body = "~~~\nhi\n~~~";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("```text\n"), "tildes don't close backticks");
    }

    #[test]
    fn handles_info_string_after_fence() {
        // ```text on its own line is a CLOSE if it's just backticks
        // and whitespace; with `text` after, it's an open. Sanitizer
        // must still escalate because the regex `^[ ]{0,3}` `` ` ``{N,}` `[ \t]*$`
        // only matches *closing* fences.
        let body = "stuff\n```\nmore"; // bare close
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("````text\n"));

        let body2 = "stuff\n```rust\nmore"; // not a close
        let out2 = wrap_body_fenced(body2);
        assert!(out2.starts_with("```text\n"));
    }

    #[test]
    fn html_entity_backticks_dont_escape() {
        // HTML entities are rendered as text inside fenced blocks, so
        // they don't escape — sanitizer doesn't need to do anything.
        let body = "&#96;&#96;&#96;\n`code`\n&#96;&#96;&#96;";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("```text\n"));
        assert!(out.contains(body));
    }

    #[test]
    fn five_backticks_in_body_escalates_to_six() {
        let body = "weird: `````".to_string();
        let out = wrap_body_fenced(&body);
        assert!(out.starts_with("``````text\n"), "got: {}", out);
    }

    #[test]
    fn wrap_round_trips_assorted_inputs() {
        for s in [
            "",
            "hello",
            "@everyone please look",
            "![pixel](https://attacker/?ip=)",
            "<img onerror=alert(1)>",
            "[link](javascript:alert(1))",
            "#1 issue cross-ref",
        ] {
            assert_wrap_round_trips(s);
        }
    }

    #[test]
    fn sanitize_title_strips_controls() {
        let raw = "hello\u{0007}world\u{0001}";
        assert_eq!(sanitize_title(raw), "helloworld");
    }

    #[test]
    fn sanitize_title_strips_bidi_overrides() {
        let raw = "hello\u{202E}evil";
        assert_eq!(sanitize_title(raw), "helloevil");
    }

    #[test]
    fn sanitize_title_collapses_internal_whitespace() {
        let raw = "hello   \tworld   bar";
        assert_eq!(sanitize_title(raw), "hello world bar");
    }

    #[test]
    fn sanitize_title_escapes_leading_brackets() {
        assert_eq!(sanitize_title("[bug] crash"), r"\[bug\] crash");
        assert_eq!(sanitize_title("]nope"), r"\]nope");
    }
}
