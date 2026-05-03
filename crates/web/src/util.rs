/// Copy text to the clipboard.
///
/// Tries `navigator.clipboard.writeText` first (modern API, requires HTTPS).
/// Falls back to creating a temporary textarea and using `execCommand('copy')`
/// only if the modern API rejects (non-HTTPS, no user gesture, browser
/// restrictions). The signature stays sync so Leptos `on:click` handlers can
/// call it directly; the Promise returned by `writeText` is awaited inside a
/// `spawn_local` task so the fallback never runs on the happy path.
pub fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };

    // Kick off the modern clipboard API and await its Promise. The textarea
    // fallback only runs if the Promise rejects.
    let clipboard = window.navigator().clipboard();
    let promise = clipboard.write_text(text);
    let owned = text.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        match wasm_bindgen_futures::JsFuture::from(promise).await {
            Ok(_) => {
                tracing::debug!("clipboard.writeText succeeded");
            }
            Err(err) => {
                tracing::debug!(
                    ?err,
                    "clipboard.writeText rejected; using textarea fallback"
                );
                exec_command_copy_fallback(&owned);
            }
        }
    });
}

/// Legacy clipboard path: append a hidden textarea, select its contents,
/// invoke `document.execCommand('copy')`, and remove the textarea. Only
/// invoked when the modern `navigator.clipboard.writeText` Promise rejects.
fn exec_command_copy_fallback(text: &str) {
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    let Ok(el) = document.create_element("textarea") else {
        return;
    };
    let ta: web_sys::HtmlTextAreaElement = el.unchecked_into();
    ta.set_value(text);
    let style = ta.style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("left", "-9999px");
    let _ = body.append_child(&ta);
    ta.select();
    if let Ok(html_doc) = document.dyn_into::<web_sys::HtmlDocument>() {
        let _ = html_doc.exec_command("copy");
    }
    let _ = body.remove_child(&ta);
}

/// Render an elapsed-milliseconds duration as a humanised
/// "{N} {unit} ago" phrase used by the dormant-row meta line.
///
/// Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`
/// §Sidebar treatment — no abbreviations in the visible string.
/// Sub-minute elapsed times collapse to `just now`.
pub fn humanise_elapsed_ms(elapsed_ms: u64) -> String {
    const MIN: u64 = 60_000;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    if elapsed_ms < MIN {
        return "just now".into();
    }
    if elapsed_ms < HOUR {
        let n = elapsed_ms / MIN;
        return format!("{n} minute{} ago", if n == 1 { "" } else { "s" });
    }
    if elapsed_ms < DAY {
        let n = elapsed_ms / HOUR;
        return format!("{n} hour{} ago", if n == 1 { "" } else { "s" });
    }
    if elapsed_ms < WEEK {
        let n = elapsed_ms / DAY;
        return format!("{n} day{} ago", if n == 1 { "" } else { "s" });
    }
    let n = elapsed_ms / WEEK;
    format!("{n} week{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanise_elapsed_ms_just_now() {
        assert_eq!(humanise_elapsed_ms(0), "just now");
        assert_eq!(humanise_elapsed_ms(59_000), "just now");
    }

    #[test]
    fn humanise_elapsed_ms_minutes() {
        assert_eq!(humanise_elapsed_ms(60_000), "1 minute ago");
        assert_eq!(humanise_elapsed_ms(2 * 60_000), "2 minutes ago");
    }

    #[test]
    fn humanise_elapsed_ms_hours() {
        assert_eq!(humanise_elapsed_ms(60 * 60_000), "1 hour ago");
        assert_eq!(humanise_elapsed_ms(3 * 60 * 60_000), "3 hours ago");
    }

    #[test]
    fn humanise_elapsed_ms_days() {
        assert_eq!(humanise_elapsed_ms(24 * 60 * 60_000), "1 day ago");
        assert_eq!(humanise_elapsed_ms(2 * 24 * 60 * 60_000), "2 days ago");
    }

    #[test]
    fn humanise_elapsed_ms_weeks() {
        assert_eq!(humanise_elapsed_ms(7 * 24 * 60 * 60_000), "1 week ago");
        assert_eq!(humanise_elapsed_ms(2 * 7 * 24 * 60 * 60_000), "2 weeks ago");
    }
}
