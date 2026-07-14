//! Browser-local TrueType font lookup.
//!
//! Web builds load base64-encoded font bytes previously stored in
//! `localStorage`. Browser storage quotas are small, so callers should retain
//! only fonts they need.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;
#[cfg(target_arch = "wasm32")]
use base64::Engine;

/// A compact view of what we persist.
#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone)]
pub struct PersistedFont {
    pub family: String,
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Result<web_sys::Storage, JsValue> {
    web_sys::window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .local_storage()?
        .ok_or_else(|| JsValue::from_str("no localStorage"))
}

#[cfg(target_arch = "wasm32")]
fn storage_key(family: &str, variant: &str) -> String {
    format!("alumina.ttf:{family}:{variant}")
}

/// Load bytes back from `localStorage` for `(family, variant)`.
#[cfg(target_arch = "wasm32")]
pub fn load_persisted_ttf(family: &str, variant: &str) -> Result<Option<Vec<u8>>, JsValue> {
    let key = storage_key(family, variant);
    let store = storage()?;
    let Some(b64) = store.get_item(&key)? else {
        return Ok(None);
    };
    // Convert the codec error into the JavaScript-facing error type.
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| JsValue::from_str(&format!("base64 decode error: {e}")))?;
    Ok(Some(bytes))
}

/// Enumerate all persisted fonts (`ttf:{family}:{variant}`) with decoded sizes.
#[cfg(target_arch = "wasm32")]
pub fn list_persisted_ttf() -> Result<Vec<PersistedFont>, JsValue> {
    let store = storage()?;
    let len = store.length()?;
    let mut out = Vec::new();
    for i in 0..len {
        if let Some(key) = store.key(i)? {
            if let Some(rest) = key.strip_prefix("alumina.ttf:") {
                let mut parts = rest.splitn(2, ':');
                let family = parts.next().unwrap_or_default().to_string();
                let _variant = parts.next().unwrap_or_default();
                out.push(PersistedFont { family });
            }
        }
    }
    Ok(out)
}
