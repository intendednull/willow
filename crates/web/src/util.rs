/// Copy text to the clipboard.
///
/// Tries `navigator.clipboard.writeText` first (modern API, requires HTTPS).
/// Falls back to creating a temporary textarea and using `execCommand('copy')`.
pub fn copy_to_clipboard(text: &str) {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n");
    let _ = js_sys::eval(&format!(
        r#"(function(){{
            var t = '{escaped}';
            if (navigator.clipboard && navigator.clipboard.writeText) {{
                navigator.clipboard.writeText(t).catch(function(){{}});
            }} else {{
                var a = document.createElement('textarea');
                a.value = t;
                a.style.position = 'fixed';
                a.style.left = '-9999px';
                document.body.appendChild(a);
                a.select();
                document.execCommand('copy');
                document.body.removeChild(a);
            }}
        }})()"#
    ));
}
