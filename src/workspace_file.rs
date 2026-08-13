//! Platform file exchange for canonical `ALGW` bytes.
//!
//! This module never parses or trusts a workspace. It moves bounded bytes
//! between the platform and `control_graph_ui`, whose canonical replay and
//! audited semantic/layout admission remain authoritative.

use eframe::egui;

pub(crate) enum WorkspaceFileEvent {
    Import(Result<Vec<u8>, String>),
    Export(Result<usize, String>),
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(crate) struct WorkspaceFileBridge {
    path: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl WorkspaceFileBridge {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        bytes: &[u8],
        maximum_import_bytes: usize,
        _download_name: &str,
    ) -> Vec<WorkspaceFileEvent> {
        let mut events = Vec::new();
        ui.horizontal_wrapped(|ui| {
            ui.label("ALGW file");
            ui.add(
                egui::TextEdit::singleline(&mut self.path)
                    .desired_width(300.0)
                    .hint_text("/path/to/workspace.algw"),
            );
            if ui.small_button("open").clicked() {
                events.push(WorkspaceFileEvent::Import(read_bounded_file(
                    &self.path,
                    maximum_import_bytes,
                )));
            }
            if ui.small_button("save exact bytes").clicked() {
                events.push(WorkspaceFileEvent::Export(write_file(&self.path, bytes)));
            }
        });
        events
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bounded_file(path: &str, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    use std::io::Read as _;

    if path.is_empty() {
        return Err("ALGW file path is empty".to_owned());
    }
    let file =
        std::fs::File::open(path).map_err(|error| format!("could not open ALGW file: {error}"))?;
    let maximum_plus_one = maximum_bytes
        .checked_add(1)
        .ok_or_else(|| "ALGW file limit cannot be represented".to_owned())?;
    let read_limit = u64::try_from(maximum_plus_one)
        .map_err(|_| "ALGW file limit exceeds native reader width".to_owned())?;
    let mut bytes = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read ALGW file: {error}"))?;
    if bytes.len() > maximum_bytes {
        Err(format!(
            "ALGW file exceeds the {maximum_bytes}-byte admission limit"
        ))
    } else {
        Ok(bytes)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &str, bytes: &[u8]) -> Result<usize, String> {
    use std::io::Write as _;

    if path.is_empty() {
        return Err("ALGW file path is empty".to_owned());
    }
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("could not create ALGW file: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("could not write ALGW file: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("could not sync ALGW file: {error}"))?;
    Ok(bytes.len())
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct WorkspaceFileBridge {
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
impl WorkspaceFileBridge {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        bytes: &[u8],
        maximum_import_bytes: usize,
        download_name: &str,
    ) -> Vec<WorkspaceFileEvent> {
        let mut events = Vec::new();
        if let Some(result) = self
            .pending
            .as_ref()
            .and_then(|pending| pending.result.borrow_mut().take())
        {
            if let Some(pending) = self.pending.take() {
                pending.input.remove();
            }
            events.push(WorkspaceFileEvent::Import(result));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("ALGW file");
            if ui.small_button("download exact bytes").clicked() {
                events.push(WorkspaceFileEvent::Export(
                    download_workspace(download_name, bytes).map(|()| bytes.len()),
                ));
            }
            let waiting = self.pending.is_some();
            if ui
                .add_enabled(!waiting, egui::Button::new("open .algw"))
                .clicked()
            {
                match PendingImport::start(maximum_import_bytes) {
                    Ok(pending) => self.pending = Some(pending),
                    Err(error) => events.push(WorkspaceFileEvent::Import(Err(error))),
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
    fn start(maximum_bytes: usize) -> Result<Self, String> {
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
        input.set_accept(".algw,application/octet-stream");
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
                        Err("browser file selection was cancelled".to_owned()),
                    );
                    return;
                }
                let Some(file) = callback_input.files().and_then(|files| files.get(0)) else {
                    set_import_result(
                        &callback_result,
                        Err("browser file selection did not contain a file".to_owned()),
                    );
                    return;
                };
                let Ok(maximum) = u32::try_from(maximum_bytes) else {
                    set_import_result(
                        &callback_result,
                        Err("ALGW browser admission limit exceeds u32".to_owned()),
                    );
                    return;
                };
                let maximum_as_f64 = f64::from(maximum);
                if !file.size().is_finite() || file.size() > maximum_as_f64 {
                    set_import_result(
                        &callback_result,
                        Err(format!(
                            "ALGW file exceeds the {maximum_bytes}-byte admission limit"
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
                                    "ALGW file exceeds the {maximum_bytes}-byte admission limit"
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
fn download_workspace(name: &str, bytes: &[u8]) -> Result<(), String> {
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
    fn native_file_exchange_is_exact_and_import_allocation_is_bounded() {
        let path = temporary_path("exact");
        let path_text = path.to_string_lossy();
        let bytes = b"ALGW canonical fixture bytes";
        assert_eq!(write_file(&path_text, bytes).unwrap(), bytes.len());
        assert_eq!(read_bounded_file(&path_text, bytes.len()).unwrap(), bytes);
        assert!(
            read_bounded_file(&path_text, bytes.len() - 1)
                .unwrap_err()
                .contains("admission limit")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn native_file_exchange_rejects_empty_and_missing_paths() {
        assert!(read_bounded_file("", 10).unwrap_err().contains("empty"));
        assert!(write_file("", b"x").unwrap_err().contains("empty"));
        let path = temporary_path("missing");
        assert!(
            read_bounded_file(&path.to_string_lossy(), 10)
                .unwrap_err()
                .contains("could not open")
        );
    }
}
