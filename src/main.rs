#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    alumina_interface::run_native()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
