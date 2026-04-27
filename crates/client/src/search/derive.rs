//! Body-string derivation helpers for [`super::IndexableMessage`].
//!
//! `DisplayMessage` doesn't carry an `attachments` field — files are
//! inlined into the message body as `[file:<filename>:<base64>]` by
//! [`crate::ClientHandle::share_file_inline`]. The search index needs
//! booleans (`has_image`, `has_file`) to power the `has:image` /
//! `has:file` operators (see `local-search.md` §Operator filters), so
//! the body string is the source of truth.
//!
//! This module concentrates that derivation in one pure function so
//! the indexer call site in `crates/web/src/app.rs` doesn't have to
//! re-encode the inline-file format, and so a future native consumer
//! (agent / CLI) gets the same answers.
//!
//! Image vs file distinction follows the rendering rule in
//! `crates/web/src/components/message.rs`: an inline file with an
//! image extension renders as an inline embed (so `has:image`); any
//! other inline file renders as a download card (so `has:file`). A
//! body that carries no inline file but contains an image URL still
//! produces an inline embed in the message renderer, so `has:image`
//! also fires for that case. URLs to non-image files don't produce a
//! visible attachment, so `has:file` is reserved for the inline-file
//! payload format only.

/// Filename extensions the message renderer treats as inline-image
/// embeds. Kept in sync with `IMAGE_EXTENSIONS` in
/// `crates/web/src/components/mod.rs` (which scopes URL-driven embeds
/// to the same set).
const IMAGE_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".ico",
];

/// Derive `(has_image, has_file)` for an `IndexableMessage` from the
/// already-decrypted body string.
///
/// Rules (mirrors message-renderer logic):
/// - body starts with `[file:` and ends with `]` with at least one
///   inner `:` separator → inline file payload. If the filename has an
///   image extension → `has_image`; otherwise → `has_file`.
/// - body contains an `http(s)://…<image-ext>` URL → `has_image`
///   (auto-embed path in the renderer).
/// - otherwise both false.
pub fn derive_has_image_file(body: &str) -> (bool, bool) {
    if let Some(filename) = inline_file_filename(body) {
        let is_image = is_image_filename(&filename);
        return (is_image, !is_image);
    }
    let has_image = body_contains_image_url(body);
    (has_image, false)
}

/// Extract the filename from a body that matches the
/// `[file:<filename>:<base64>]` inline-file format. Returns `None` for
/// any other body shape (including malformed prefixes / missing inner
/// colon).
fn inline_file_filename(body: &str) -> Option<String> {
    let inner = body.strip_prefix("[file:")?.strip_suffix(']')?;
    let colon = inner.find(':')?;
    Some(inner[..colon].to_string())
}

fn is_image_filename(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    IMAGE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Cheap scan: does `body` contain a `http://` or `https://` URL whose
/// path ends in a known image extension? Mirrors the URL stage in
/// `crates/web/src/components/message.rs::extract_urls` without
/// pulling in the full segment splitter — the indexer only needs the
/// boolean.
fn body_contains_image_url(body: &str) -> bool {
    let mut idx = 0;
    while idx < body.len() {
        let rest = &body[idx..];
        let Some(start) = rest
            .find("https://")
            .map(|i| (i, "https://".len()))
            .or_else(|| rest.find("http://").map(|i| (i, "http://".len())))
            .map(|(i, _)| i)
        else {
            return false;
        };
        let abs_start = idx + start;
        let url_rest = &body[abs_start..];
        let url_end = url_rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == ')' || c == ']')
            .map(|i| abs_start + i)
            .unwrap_or(body.len());
        let url = &body[abs_start..url_end];
        // Strip query/fragment before extension check — `?` / `#` are
        // not whitespace so the URL extractor keeps them in `url`.
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let lower = path.to_lowercase();
        if IMAGE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
            return true;
        }
        idx = url_end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_body_yields_no_attachments() {
        assert_eq!(derive_has_image_file("hello world"), (false, false));
        assert_eq!(derive_has_image_file(""), (false, false));
    }

    #[test]
    fn inline_image_file_sets_has_image_only() {
        let body = "[file:photo.png:Zm9vYmFy]";
        assert_eq!(derive_has_image_file(body), (true, false));
    }

    #[test]
    fn inline_image_file_uppercase_extension() {
        let body = "[file:Photo.JPG:Zm9v]";
        assert_eq!(derive_has_image_file(body), (true, false));
    }

    #[test]
    fn inline_non_image_file_sets_has_file_only() {
        let body = "[file:notes.txt:Zm9vYmFy]";
        assert_eq!(derive_has_image_file(body), (false, true));
    }

    #[test]
    fn inline_pdf_is_a_file_not_image() {
        let body = "[file:report.pdf:Zm9v]";
        assert_eq!(derive_has_image_file(body), (false, true));
    }

    #[test]
    fn malformed_inline_file_falls_through_to_url_check() {
        // No inner colon — not the inline-file shape.
        assert_eq!(derive_has_image_file("[file:nodatahere]"), (false, false));
        // Missing closing bracket.
        assert_eq!(derive_has_image_file("[file:foo.png:abc"), (false, false));
    }

    #[test]
    fn body_with_image_url_sets_has_image() {
        let body = "look at this https://example.com/cat.png cute";
        assert_eq!(derive_has_image_file(body), (true, false));
    }

    #[test]
    fn body_with_image_url_query_string() {
        let body = "https://cdn.example.com/cat.jpg?w=400";
        assert_eq!(derive_has_image_file(body), (true, false));
    }

    #[test]
    fn body_with_non_image_url_no_attachment_flags() {
        let body = "https://example.com/article";
        assert_eq!(derive_has_image_file(body), (false, false));
    }

    #[test]
    fn body_with_multiple_urls_finds_image() {
        let body = "https://a.com/x https://b.com/photo.gif";
        assert_eq!(derive_has_image_file(body), (true, false));
    }
}
