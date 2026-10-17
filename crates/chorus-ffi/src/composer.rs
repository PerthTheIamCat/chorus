//! Composer calls: pure, no engine handle. Kept apart from `lib.rs` so other
//! sessions' FFI additions merge without touching this section.

use super::{borrow, err, out};
use chorus_core::composer;
use std::ffi::c_char;
use std::panic::catch_unwind;

/// Pasted HTML → Markdown. `plain` is the pasteboard's plain-text flavour,
/// used when a cap is hit (input/output bytes, nesting depth, time).
/// JSON: {"ok":true,"markdown":"...","fallback":null|"reason"}
///
/// # Safety
/// `html`, `plain` valid NUL-terminated UTF-8 (`plain` may be NULL).
#[no_mangle]
pub unsafe extern "C" fn chorus_html_to_markdown(
    html: *const c_char,
    plain: *const c_char,
) -> *mut c_char {
    let Some(html) = borrow(html) else {
        return err("html must be valid UTF-8");
    };
    let plain = borrow(plain).unwrap_or("");
    match catch_unwind(|| composer::html_to_markdown(html, plain)) {
        Ok(c) => out(
            serde_json::json!({"ok": true, "markdown": c.markdown, "fallback": c.fallback})
                .to_string(),
        ),
        Err(_) => err("panic in html_to_markdown"),
    }
}

/// Styling spans for the Markdown draft. Offsets are UTF-16 code units, the
/// unit `NSTextStorage` ranges use. JSON: {"ok":true,"spans":[{"start","end","kind"}]}
///
/// # Safety
/// `text` valid NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn chorus_markdown_spans(text: *const c_char) -> *mut c_char {
    let Some(text) = borrow(text) else {
        return err("text must be valid UTF-8");
    };
    match catch_unwind(|| composer::markdown_spans(text)) {
        Ok(spans) => out(serde_json::json!({"ok": true, "spans": spans}).to_string()),
        Err(_) => err("panic in markdown_spans"),
    }
}

/// Info line for the composer when attachments narrow the router.
/// `media_types` is a JSON array of strings. {"ok":true,"note":null|"..."}
///
/// # Safety
/// `media_types` valid NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn chorus_attachment_route_note(media_types: *const c_char) -> *mut c_char {
    let Some(json) = borrow(media_types) else {
        return err("media_types must be valid UTF-8");
    };
    if json.len() > 4096 {
        return err("media_types JSON exceeds 4096 bytes");
    }
    let Ok(types) = serde_json::from_str::<Vec<String>>(json) else {
        return err("media_types must be a JSON array of strings");
    };
    let refs: Vec<&str> = types.iter().map(String::as_str).collect();
    match catch_unwind(|| composer::attachment_route_note(&refs)) {
        Ok(note) => out(serde_json::json!({"ok": true, "note": note}).to_string()),
        Err(_) => err("panic in attachment_route_note"),
    }
}
