//! Platform file exchange for bounded untrusted bytes.
//!
//! This module never parses or trusts a file. It only moves caller-bounded
//! bytes between the platform and an owning workspace. Canonical artifact
//! replay or non-canonical source import remains the owning workspace's job.

use eframe::egui;

pub(crate) enum BoundedFileEvent {
    Import(Result<Vec<u8>, String>),
    Export(Result<usize, String>),
}

#[derive(Clone, Copy)]
pub(crate) struct BoundedFileSpec {
    label: &'static str,
    extension: &'static str,
}

impl BoundedFileSpec {
    pub(crate) const fn new(label: &'static str, extension: &'static str) -> Self {
        Self { label, extension }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(crate) struct BoundedFileBridge {
    path: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl BoundedFileBridge {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        bytes: &[u8],
        maximum_import_bytes: usize,
        _download_name: &str,
        spec: BoundedFileSpec,
    ) -> Vec<BoundedFileEvent> {
        let mut events = Vec::new();
        ui.horizontal_wrapped(|ui| {
            ui.label(spec.label);
            ui.add(
                egui::TextEdit::singleline(&mut self.path)
                    .desired_width(300.0)
                    .hint_text(format!("/path/to/artifact.{}", spec.extension)),
            );
            if ui
                .small_button(format!("open .{}", spec.extension))
                .clicked()
            {
                events.push(BoundedFileEvent::Import(read_bounded_file(
                    &self.path,
                    maximum_import_bytes,
                    spec.label,
                )));
            }
            if ui.small_button("save bytes").clicked() {
                events.push(BoundedFileEvent::Export(write_file(
                    &self.path, bytes, spec.label,
                )));
            }
        });
        events
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bounded_file(path: &str, maximum_bytes: usize, label: &str) -> Result<Vec<u8>, String> {
    use std::io::Read as _;

    if path.is_empty() {
        return Err(format!("{label} path is empty"));
    }
    let file =
        std::fs::File::open(path).map_err(|error| format!("could not open {label}: {error}"))?;
    let maximum_plus_one = maximum_bytes
        .checked_add(1)
        .ok_or_else(|| format!("{label} limit cannot be represented"))?;
    let read_limit = u64::try_from(maximum_plus_one)
        .map_err(|_| format!("{label} limit exceeds native reader width"))?;
    let mut bytes = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read {label}: {error}"))?;
    if bytes.len() > maximum_bytes {
        Err(format!(
            "{label} exceeds the {maximum_bytes}-byte admission limit"
        ))
    } else {
        Ok(bytes)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &str, bytes: &[u8], label: &str) -> Result<usize, String> {
    use std::io::Write as _;

    if path.is_empty() {
        return Err(format!("{label} path is empty"));
    }
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("could not create {label}: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("could not write {label}: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("could not sync {label}: {error}"))?;
    Ok(bytes.len())
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct BoundedFileBridge {
    pending: Option<PendingImport>,
}

#[cfg(target_arch = "wasm32")]
struct PendingImport {
    input: web_sys::HtmlInputElement,
    result: PendingImportResult,
    _callback: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
}

#[cfg(target_arch = "wasm32")]
type PendingImportResult = std::rc::Rc<std::cell::RefCell<Option<Result<Vec<u8>, String>>>>;

#[cfg(target_arch = "wasm32")]
impl BoundedFileBridge {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        bytes: &[u8],
        maximum_import_bytes: usize,
        download_name: &str,
        spec: BoundedFileSpec,
    ) -> Vec<BoundedFileEvent> {
        let mut events = Vec::new();
        if let Some(result) = self
            .pending
            .as_ref()
            .and_then(|pending| pending.result.borrow_mut().take())
        {
            if let Some(pending) = self.pending.take() {
                pending.input.remove();
            }
            events.push(BoundedFileEvent::Import(result));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(spec.label);
            if ui.small_button("download bytes").clicked() {
                events.push(BoundedFileEvent::Export(
                    download_bytes(download_name, bytes).map(|()| bytes.len()),
                ));
            }
            let waiting = self.pending.is_some();
            if ui
                .add_enabled(
                    !waiting,
                    egui::Button::new(format!("open .{}", spec.extension)),
                )
                .clicked()
            {
                match PendingImport::start(maximum_import_bytes, spec) {
                    Ok(pending) => self.pending = Some(pending),
                    Err(error) => events.push(BoundedFileEvent::Import(Err(error))),
                }
            }
            if waiting {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
                ui.weak("waiting for browser file selection");
                if ui.small_button("cancel request").clicked()
                    && let Some(pending) = self.pending.take()
                {
                    pending.input.remove();
                }
            }
        });
        events
    }
}

#[cfg(target_arch = "wasm32")]
impl PendingImport {
    fn start(maximum_bytes: usize, spec: BoundedFileSpec) -> Result<Self, String> {
        use wasm_bindgen::JsCast as _;

        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "browser document is unavailable".to_owned())?;
        let input = document
            .create_element("input")
            .map_err(|value| js_error(&value))?
            .dyn_into::<web_sys::HtmlInputElement>()
            .map_err(|element| js_error(&element.into()))?;
        input.set_type("file");
        input.set_accept(&format!(".{},application/octet-stream", spec.extension));
        input
            .set_attribute("hidden", "")
            .map_err(|value| js_error(&value))?;
        document
            .body()
            .ok_or_else(|| "browser document body is unavailable".to_owned())?
            .append_child(&input)
            .map_err(|value| js_error(&value))?;

        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let callback_result = std::rc::Rc::clone(&result);
        let callback_input = input.clone();
        let callback =
            wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::Event| {
                if event.type_() == "cancel" {
                    set_import_result(
                        &callback_result,
                        Err(format!("{} selection was cancelled", spec.label)),
                    );
                    return;
                }
                let Some(file) = callback_input.files().and_then(|files| files.get(0)) else {
                    set_import_result(
                        &callback_result,
                        Err(format!("{} selection did not contain a file", spec.label)),
                    );
                    return;
                };
                let Ok(maximum) = u32::try_from(maximum_bytes) else {
                    set_import_result(
                        &callback_result,
                        Err(format!(
                            "{} browser admission limit exceeds u32",
                            spec.label
                        )),
                    );
                    return;
                };
                let maximum_as_f64 = f64::from(maximum);
                if !file.size().is_finite() || file.size() > maximum_as_f64 {
                    set_import_result(
                        &callback_result,
                        Err(format!(
                            "{} exceeds the {maximum_bytes}-byte admission limit",
                            spec.label
                        )),
                    );
                    return;
                }
                let async_result = std::rc::Rc::clone(&callback_result);
                wasm_bindgen_futures::spawn_local(async move {
                    let result = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
                        .await
                        .map_err(|value| js_error(&value))
                        .and_then(|buffer| {
                            let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                            if bytes.len() > maximum_bytes {
                                Err(format!(
                                    "{} exceeds the {maximum_bytes}-byte admission limit",
                                    spec.label
                                ))
                            } else {
                                Ok(bytes)
                            }
                        });
                    set_import_result(&async_result, result);
                });
            }) as Box<dyn FnMut(_)>);
        input
            .add_event_listener_with_callback("change", callback.as_ref().unchecked_ref())
            .map_err(|value| js_error(&value))?;
        input
            .add_event_listener_with_callback("cancel", callback.as_ref().unchecked_ref())
            .map_err(|value| js_error(&value))?;
        input.click();
        Ok(Self {
            input,
            result,
            _callback: callback,
        })
    }
}

#[cfg(target_arch = "wasm32")]
fn set_import_result(slot: &PendingImportResult, result: Result<Vec<u8>, String>) {
    let mut slot = slot.borrow_mut();
    if slot.is_none() {
        *slot = Some(result);
    }
}

#[cfg(target_arch = "wasm32")]
fn download_bytes(name: &str, bytes: &[u8]) -> Result<(), String> {
    use wasm_bindgen::JsCast as _;

    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence(parts.as_ref())
        .map_err(|value| js_error(&value))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|value| js_error(&value))?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "browser document is unavailable".to_owned())?;
        let anchor = document
            .create_element("a")
            .map_err(|value| js_error(&value))?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|element| js_error(&element.into()))?;
        anchor.set_href(&url);
        anchor.set_download(name);
        anchor
            .set_attribute("hidden", "")
            .map_err(|value| js_error(&value))?;
        document
            .body()
            .ok_or_else(|| "browser document body is unavailable".to_owned())?
            .append_child(&anchor)
            .map_err(|value| js_error(&value))?;
        anchor.click();
        anchor.remove();
        Ok(())
    })();
    let revoke = web_sys::Url::revoke_object_url(&url).map_err(|value| js_error(&value));
    match (result, revoke) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(target_arch = "wasm32")]
fn js_error(value: &wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser API rejected: {value:?}"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn temporary_path(label: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let identity = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "alumina-workspace-file-{label}-{}-{identity}.algw",
            std::process::id()
        ))
    }

    #[test]
    fn native_file_exchange_preserves_bounded_bytes() {
        let path = temporary_path("bounded");
        let path_text = path.to_string_lossy();
        let bytes = b"ALGW canonical fixture bytes";
        assert_eq!(
            write_file(&path_text, bytes, "test artifact").unwrap(),
            bytes.len()
        );
        assert_eq!(
            read_bounded_file(&path_text, bytes.len(), "test artifact").unwrap(),
            bytes
        );
        assert!(
            read_bounded_file(&path_text, bytes.len() - 1, "test artifact")
                .unwrap_err()
                .contains("admission limit")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn native_file_exchange_rejects_empty_and_missing_paths() {
        assert!(
            read_bounded_file("", 10, "test artifact")
                .unwrap_err()
                .contains("empty")
        );
        assert!(
            write_file("", b"x", "test artifact")
                .unwrap_err()
                .contains("empty")
        );
        let path = temporary_path("missing");
        assert!(
            read_bounded_file(&path.to_string_lossy(), 10, "test artifact")
                .unwrap_err()
                .contains("could not open")
        );
    }
}
