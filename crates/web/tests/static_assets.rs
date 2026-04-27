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
