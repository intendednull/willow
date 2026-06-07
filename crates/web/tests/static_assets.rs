//! Static asset allow-list assertions.
//!
//! SVG icons are referenced from the service worker (`sw.js`), the HTML
//! shell (`index.html`), and the PWA manifest (`manifest.json`). Today
//! these paths are bundled, build-time-only assets. The risk this test
//! guards against is a future regression where, for example, a "custom
//! server icon" feature stores user-supplied SVGs at the same paths and
//! lets scripts inside such an SVG execute in the notification context.
//!
//! This test scans the three files for any SVG path reference and asserts
//! every match belongs to a hard-coded allow-list of bundled assets.
//! Sanitization of user-supplied icons is intentionally out of scope.
//!
//! See: GitHub issue #312 (SEC-W-07).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The complete set of SVG icon paths permitted to appear in the web
/// shell, service worker, and manifest. Adding a new bundled icon
/// requires updating this list — that's the point of the assertion.
const ALLOWED_SVG_ICONS: &[&str] = &["/icon-192.svg", "/icon-512.svg"];

fn web_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_asset(name: &str) -> String {
    let path = web_crate_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Match every `…icon…svg` style path-or-URL token in the given text.
///
/// Captures absolute (`/icon-192.svg`) and relative (`icon-192.svg`)
/// references in JS string literals, HTML attribute values, and JSON
/// fields. Substrings inside `<svg>` markup itself are excluded by
/// requiring a `.svg` suffix on the token.
fn collect_svg_references(haystack: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = haystack.as_bytes();
    let needle = b".svg";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            // Walk backwards to find the start of the path token.
            let mut start = i;
            while start > 0 {
                let c = bytes[start - 1];
                let is_path_char =
                    c.is_ascii_alphanumeric() || c == b'/' || c == b'-' || c == b'_' || c == b'.';
                if !is_path_char {
                    break;
                }
                start -= 1;
            }
            let end = i + needle.len();
            let token = &haystack[start..end];
            if !token.is_empty() {
                out.push(token.to_string());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn assert_all_in_allow_list(file: &str, refs: &[String]) {
    for r in refs {
        // Normalize bare `icon-192.svg` to `/icon-192.svg` for matching;
        // both forms are treated as references to the same bundled asset.
        let normalized: String = if r.starts_with('/') {
            r.clone()
        } else {
            format!("/{r}")
        };
        assert!(
            ALLOWED_SVG_ICONS.contains(&normalized.as_str()),
            "{file} references unbundled SVG icon path `{r}` (normalized `{normalized}`).\n\
             Allowed paths: {ALLOWED_SVG_ICONS:?}.\n\
             If you are adding a new bundled icon, update ALLOWED_SVG_ICONS in \
             crates/web/tests/static_assets.rs. NEVER reference user-supplied SVGs from \
             the service worker — see GitHub issue #312.",
        );
    }
}

#[test]
fn sw_js_only_references_bundled_svg_icons() {
    let contents = read_asset("sw.js");
    let refs = collect_svg_references(&contents);
    // sw.js MUST reference at least one icon path (notification rendering).
    assert!(
        !refs.is_empty(),
        "sw.js no longer references any SVG icon — did the notification icon move? \
         Update ALLOWED_SVG_ICONS or this test if intentional.",
    );
    assert_all_in_allow_list("sw.js", &refs);
}

#[test]
fn index_html_only_references_bundled_svg_icons() {
    let contents = read_asset("index.html");
    let refs = collect_svg_references(&contents);
    assert!(
        !refs.is_empty(),
        "index.html no longer references any SVG icon — favicon removed? \
         Update this test if intentional.",
    );
    assert_all_in_allow_list("index.html", &refs);
}

/// Expected directives for the CSP meta tag baked into `index.html`.
///
/// Each entry is `(directive-name, &[expected source tokens])`. The test
/// parses the meta tag's `content` attribute and asserts that the parsed
/// directive set equals this expected set *exactly* — no missing
/// directives, no extra directives, and no extra source tokens within a
/// directive. Token order is irrelevant (compared as sets).
///
/// The exact-match shape is deliberate: a substring-only check let
/// additions like widening `script-src` with `'unsafe-inline'` slip
/// through unnoticed (see GitHub issue #619). If you intentionally loosen
/// or tighten one of these directives, update both this list and the
/// rationale comment beside the meta tag in `index.html` so the
/// reasoning stays in sync (issue #175).
const EXPECTED_CSP_DIRECTIVES: &[(&str, &[&str])] = &[
    ("default-src", &["'self'"]),
    // WASM module + the still-extant js_sys::eval() sites (tracked by
    // issues #171 / #425). Drop 'unsafe-eval' once those are gone.
    (
        "script-src",
        &["'self'", "'wasm-unsafe-eval'", "'unsafe-eval'"],
    ),
    // Inline style="…" attrs from Leptos views + Google Fonts CSS @import.
    (
        "style-src",
        &["'self'", "'unsafe-inline'", "https://fonts.googleapis.com"],
    ),
    ("font-src", &["'self'", "https://fonts.gstatic.com"]),
    // ws/wss for relay transport, https for the relay HTTP bootstrap probe.
    ("connect-src", &["'self'", "ws:", "wss:", "https:"]),
    // data: for avatar URIs, blob: for runtime createObjectURL attachments.
    ("img-src", &["'self'", "https:", "data:", "blob:"]),
    ("media-src", &["'self'", "blob:"]),
    ("worker-src", &["'self'"]),
    ("object-src", &["'none'"]),
    ("base-uri", &["'self'"]),
    ("form-action", &["'self'"]),
    ("frame-ancestors", &["'none'"]),
];

/// Extract the `content="…"` attribute value of the
/// `<meta http-equiv="Content-Security-Policy" …>` tag from `index.html`.
fn extract_csp_content(html: &str) -> Option<&str> {
    let needle = "http-equiv=\"Content-Security-Policy\"";
    let pos = html.find(needle)?;
    // Find `content="…"` after the http-equiv attribute (within the same tag).
    let after = &html[pos..];
    let tag_end = after.find('>')?;
    let tag = &after[..tag_end];
    let content_marker = "content=\"";
    let content_start = tag.find(content_marker)? + content_marker.len();
    let content_rel = tag[content_start..].find('"')?;
    Some(&tag[content_start..content_start + content_rel])
}

/// Parse a CSP `content` attribute value into a directive → token-set map.
///
/// Splits on `;`, trims, then within each directive splits on whitespace,
/// taking the first token as the directive name and the rest as its
/// source tokens.
fn parse_csp(content: &str) -> BTreeMap<&str, BTreeSet<&str>> {
    content
        .split(';')
        .filter_map(|d| {
            let d = d.trim();
            if d.is_empty() {
                return None;
            }
            let mut parts = d.split_whitespace();
            let name = parts.next()?;
            Some((name, parts.collect()))
        })
        .collect()
}

#[test]
fn index_html_declares_content_security_policy() {
    let contents = read_asset("index.html");
    let needle = "http-equiv=\"Content-Security-Policy\"";
    assert!(
        contents.contains(needle),
        "index.html is missing the Content-Security-Policy meta tag. \
         See GitHub issue #175 — the CSP guards against script injection \
         and clickjacking and must stay in the document head.",
    );

    let csp = extract_csp_content(&contents).expect(
        "could not extract content=\"…\" from the Content-Security-Policy meta tag in index.html",
    );
    let actual = parse_csp(csp);
    let expected: BTreeMap<&str, BTreeSet<&str>> = EXPECTED_CSP_DIRECTIVES
        .iter()
        .map(|(name, tokens)| (*name, tokens.iter().copied().collect()))
        .collect();

    // Exact-set comparison: catches missing directives, extra directives,
    // missing tokens, and — most importantly for issue #619 — additions
    // of source tokens (e.g. widening script-src with 'unsafe-inline')
    // that a substring check would silently allow through.
    assert_eq!(
        actual, expected,
        "index.html CSP does not match the expected directive set.\n\
         Update both the meta tag in index.html AND \
         EXPECTED_CSP_DIRECTIVES in this test (and the rationale comment \
         beside the meta tag) if you are intentionally changing the CSP.\n\
         Expected: {expected:#?}\nActual:   {actual:#?}",
    );
}

/// Defense-in-depth: even if a future maintainer updates both the meta
/// tag and `EXPECTED_CSP_DIRECTIVES` together, certain source tokens are
/// dangerous enough that they should never appear outside specific
/// directives. This test bypasses the expected-set comparison and asserts
/// those invariants directly.
#[test]
fn csp_rejects_unsafe_inline_outside_style_src() {
    let contents = read_asset("index.html");
    let csp = extract_csp_content(&contents).expect("CSP meta tag missing");
    let parsed = parse_csp(csp);
    for (name, tokens) in &parsed {
        if *name == "style-src" {
            // 'unsafe-inline' is intentionally allowed here for Leptos
            // inline style="…" attributes. See the comment beside the
            // meta tag in index.html.
            continue;
        }
        assert!(
            !tokens.contains("'unsafe-inline'"),
            "CSP directive `{name}` contains 'unsafe-inline'. \
             Only `style-src` is permitted to use 'unsafe-inline'; any \
             other directive (especially script-src) opens an XSS vector. \
             See GitHub issue #619.",
        );
        assert!(
            !tokens.contains("'unsafe-hashes'"),
            "CSP directive `{name}` contains 'unsafe-hashes'. \
             'unsafe-hashes' relaxes hash matching to apply to inline \
             event handlers; it should not be added to any directive \
             without an explicit security review. See issue #619.",
        );
    }
}

#[test]
fn csp_rejects_data_in_script_src() {
    let contents = read_asset("index.html");
    let csp = extract_csp_content(&contents).expect("CSP meta tag missing");
    let parsed = parse_csp(csp);
    if let Some(tokens) = parsed.get("script-src") {
        assert!(
            !tokens.contains("data:"),
            "CSP `script-src` contains `data:`. This permits inline \
             scripts to be sourced from data: URIs and is a known XSS \
             vector — never add it. See GitHub issue #619.",
        );
    }
}

#[test]
fn manifest_json_only_references_bundled_svg_icons() {
    let contents = read_asset("manifest.json");
    let refs = collect_svg_references(&contents);
    assert!(
        !refs.is_empty(),
        "manifest.json no longer references any SVG icon — PWA icons removed? \
         Update this test if intentional.",
    );
    assert_all_in_allow_list("manifest.json", &refs);
}

#[test]
fn allow_listed_icons_actually_exist_on_disk() {
    // Belt-and-braces: catch the inverse mistake of listing a path that
    // isn't actually bundled.
    for name in ALLOWED_SVG_ICONS {
        let stripped = name.strip_prefix('/').unwrap_or(name);
        let path = web_crate_dir().join(stripped);
        assert!(
            path.exists(),
            "ALLOWED_SVG_ICONS lists `{name}` but the file is missing at {}",
            path.display(),
        );
    }
}

// ── Self-tests for the scanner ──────────────────────────────────────────────

#[test]
fn scanner_extracts_paths_from_mixed_content() {
    let sample = r#"
        icon: '/icon-192.svg',
        badge: "/icon-512.svg"
        <link rel="icon" href="/icon-192.svg">
        { "src": "/icon-512.svg" }
    "#;
    let refs = collect_svg_references(sample);
    assert!(refs.iter().any(|r| r == "/icon-192.svg"));
    assert!(refs.iter().any(|r| r == "/icon-512.svg"));
}

#[test]
fn scanner_flags_unbundled_path() {
    let sample = r#"icon: '/uploads/user-icon.svg',"#;
    let refs = collect_svg_references(sample);
    assert_eq!(refs, vec!["/uploads/user-icon.svg".to_string()]);
    let normalized = "/uploads/user-icon.svg";
    assert!(!ALLOWED_SVG_ICONS.contains(&normalized));
}
