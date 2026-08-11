//! Greenfield Alumina browser/native application shell.

#![warn(clippy::pedantic)]

use std::sync::{Arc, Mutex};

use alumina_interface_core::{
    CanonicalMachinePartition2, CanonicalPathProgram2, ExactScene, ExactValue, Millimetres,
    compile_representative_program, package_canonical_program, project_for_display,
    representative_partition_policy,
};
use eframe::egui;
use eframe::glow::HasContext as _;
use hypergraphics::backend::{GpuColoredMesh, UnlitProgram};
use hypergraphics::{ExactCamera, PredicatePolicy, Projection64, Real, Viewport};

struct RenderResources {
    program: Option<UnlitProgram>,
    meshes: Vec<GpuColoredMesh>,
}

impl RenderResources {
    unsafe fn upload(
        gl: &eframe::glow::Context,
        scene: &ExactScene,
    ) -> hypergraphics::Result<Self> {
        let program = unsafe { UnlitProgram::new(gl)? };
        let mut meshes: Vec<GpuColoredMesh> = Vec::with_capacity(scene.meshes().len());
        for exact in scene.meshes() {
            let mut gpu = match unsafe { GpuColoredMesh::new(gl, exact.primitive()) } {
                Ok(gpu) => gpu,
                Err(error) => {
                    for prior in meshes.drain(..) {
                        unsafe { prior.destroy(gl) };
                    }
                    unsafe { program.destroy(gl) };
                    return Err(error);
                }
            };
            if let Err(error) = unsafe { gpu.upload_exact_mesh(gl, exact) } {
                unsafe { gpu.destroy(gl) };
                for prior in meshes.drain(..) {
                    unsafe { prior.destroy(gl) };
                }
                unsafe { program.destroy(gl) };
                return Err(error);
            }
            meshes.push(gpu);
        }
        Ok(Self {
            program: Some(program),
            meshes,
        })
    }

    unsafe fn paint(
        &self,
        gl: &eframe::glow::Context,
        projection: &Projection64,
    ) -> hypergraphics::Result<()> {
        let Some(program) = &self.program else {
            return Ok(());
        };
        unsafe {
            gl.enable(eframe::glow::DEPTH_TEST);
            gl.depth_func(eframe::glow::LEQUAL);
            gl.clear(eframe::glow::DEPTH_BUFFER_BIT);
        }
        let result = unsafe { program.bind(gl, projection) };
        if result.is_ok() {
            for mesh in &self.meshes {
                unsafe { mesh.draw(gl) };
            }
        }
        unsafe {
            gl.disable(eframe::glow::DEPTH_TEST);
        }
        result
    }

    unsafe fn destroy(&mut self, gl: &eframe::glow::Context) {
        for mesh in self.meshes.drain(..) {
            unsafe { mesh.destroy(gl) };
        }
        if let Some(program) = self.program.take() {
            unsafe { program.destroy(gl) };
        }
    }
}

/// Minimal exact-stack application used by native and browser runners.
pub struct AluminaApp {
    scene: ExactScene,
    camera: ExactCamera,
    representative_program: Option<CanonicalPathProgram2>,
    representative_partition: Option<CanonicalMachinePartition2>,
    resources: Option<Arc<Mutex<RenderResources>>>,
    setup_error: Option<String>,
}

impl AluminaApp {
    /// Construct the exact baseline scene and upload it through Hypergraphics.
    #[must_use]
    pub fn new(creation: &eframe::CreationContext<'_>) -> Self {
        let (scene, scene_error) = match ExactScene::baseline() {
            Ok(scene) => (scene, None),
            Err(error) => (
                ExactScene::default(),
                Some(format!("exact scene construction failed: {error}")),
            ),
        };
        let (representative_program, representative_partition, compiler_error) =
            match compile_representative_program() {
                Ok(program) => match representative_partition_policy()
                    .and_then(|policy| package_canonical_program(&program, policy))
                {
                    Ok(partition) => (Some(program), Some(partition), None),
                    Err(error) => (
                        Some(program),
                        None,
                        Some(format!("canonical partition packaging failed: {error}")),
                    ),
                },
                Err(error) => (
                    None,
                    None,
                    Some(format!("exact representative compilation failed: {error}")),
                ),
            };
        let (resources, renderer_error) = match creation.gl.as_deref() {
            Some(gl) => match unsafe { RenderResources::upload(gl, &scene) } {
                Ok(resources) => (Some(Arc::new(Mutex::new(resources))), None),
                Err(error) => (None, Some(error.to_string())),
            },
            None => (
                None,
                Some("the Hypergraphics glow backend is unavailable".to_owned()),
            ),
        };
        Self {
            scene,
            camera: ExactCamera::default(),
            representative_program,
            representative_partition,
            resources,
            setup_error: scene_error.or(compiler_error).or(renderer_error),
        }
    }

    fn update_camera(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if response.dragged_by(egui::PointerButton::Primary) {
            let delta = ui.input(|input| input.pointer.delta());
            let yaw = Real::try_from(f64::from(delta.x) * 0.01)
                .expect("finite pointer delta has an exact dyadic import");
            let pitch = Real::try_from(f64::from(delta.y) * 0.01)
                .expect("finite pointer delta has an exact dyadic import");
            self.camera.orbit(yaw, pitch);
        }

        if response.hovered() {
            let zoom_delta = ui.input(egui::InputState::zoom_delta);
            let scroll = ui.input(|input| input.raw_scroll_delta.y);
            let factor = if (zoom_delta - 1.0).abs() > f32::EPSILON {
                f64::from(zoom_delta)
            } else if scroll.abs() > f32::EPSILON {
                f64::from((1.0 + scroll * 0.001).clamp(0.1, 10.0))
            } else {
                1.0
            };
            if (factor - 1.0).abs() > f64::EPSILON {
                let factor = Real::try_from(factor)
                    .expect("finite UI zoom factor has an exact dyadic import");
                if let Err(error) = self.camera.zoom_by(factor, PredicatePolicy::STRICT) {
                    log::warn!("camera zoom rejected: {error}");
                }
            }
        }
    }

    fn projection(&self, rect: egui::Rect) -> hypergraphics::Result<Projection64> {
        let viewport = Viewport::new(
            f64::from(rect.min.x),
            f64::from(rect.min.y),
            f64::from(rect.width()),
            f64::from(rect.height()),
        )?;
        self.camera.projection64(viewport, PredicatePolicy::STRICT)
    }

    fn show_status(&self, ui: &mut egui::Ui) {
        ui.heading("Alumina");
        ui.label("Greenfield exact CAD/CAM baseline");
        ui.separator();
        ui.label(format!(
            "Exact scene vertices: {}",
            self.scene.vertex_count()
        ));
        ui.label(format!(
            "Exact scene triangles: {}",
            self.scene.triangle_count()
        ));
        if let Some(evidence) = self.scene.curve_display_evidence() {
            ui.label(format!(
                "Certified curve display chords: {}",
                evidence.chord_segment_count()
            ));
            ui.label(format!(
                "Curve display bound: {} mm",
                evidence.max_source_chord_error()
            ));
            ui.label(format!(
                "Retained source fragments: {}",
                evidence.source_fragment_count()
            ));
        }
        if let Some(evidence) = self.scene.region_display_evidence() {
            ui.label(format!(
                "Certified region loops: {} material / {} hole",
                evidence.material_loop_count(),
                evidence.hole_loop_count()
            ));
            ui.label(format!(
                "Certified region display chords: {}",
                evidence.chord_segment_count()
            ));
        }
        if let Some(program) = &self.representative_program {
            ui.separator();
            ui.label(format!(
                "Canonical motion segments: {}",
                program.segments().len()
            ));
            ui.label(format!(
                "Motion chord bound: {} mm",
                program.evidence().maximum_source_chord_error_mm()
            ));
            ui.label(format!(
                "Curve-to-command-chord bound: {} mm",
                program
                    .evidence()
                    .maximum_curve_to_canonical_chord_error_mm()
            ));
            ui.label(format!(
                "Canonical end tick: {}",
                program
                    .time_boundaries()
                    .last()
                    .expect("compiled path retains its terminal boundary")
                    .tick()
                    .get()
            ));
        }
        if let Some(partition) = &self.representative_partition {
            ui.label(format!(
                "Cached execution blocks: {}",
                partition.block_count()
            ));
            ui.label(format!(
                "Immutable partition bytes: {}",
                partition.bytes().len()
            ));
            ui.label(format!(
                "Content-addressed chunks: {}",
                partition.chunks().len()
            ));
            ui.label(format!(
                "Longest block horizon: {} ticks",
                partition.maximum_observed_block_ticks()
            ));
        }
        ui.label(format!(
            "Protocol schema: {}",
            alumina_protocol::PROTOCOL_VERSION
        ));
        let tenth =
            ExactValue::<Millimetres>::parse_decimal("0.1").expect("static exact decimal is valid");
        let display =
            project_for_display(&tenth).expect("small exact value has a finite display projection");
        ui.label(format!(
            "Explicit display projection: {:.3} {}",
            display.get(),
            ExactValue::<Millimetres>::unit_symbol()
        ));
        ui.separator();
        ui.label("Drag to orbit. Scroll or pinch to zoom.");
        ui.label("Geometry, CAM, and machine values never originate from this GPU view.");
        if let Some(error) = &self.setup_error {
            ui.colored_label(egui::Color32::RED, error);
        }
    }
}

impl eframe::App for AluminaApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        egui::SidePanel::left("exact_stack_status")
            .resizable(false)
            .default_width(260.0)
            .show(context, |ui| self.show_status(ui));

        egui::CentralPanel::default().show(context, |ui| {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            self.update_camera(ui, &response);

            let Some(resources) = &self.resources else {
                return;
            };
            match self.projection(rect) {
                Ok(projection) => {
                    let resources = Arc::clone(resources);
                    let callback = egui_glow::CallbackFn::new(move |_info, painter| {
                        let Ok(resources) = resources.lock() else {
                            log::error!("render-resource lock was poisoned");
                            return;
                        };
                        if let Err(error) = unsafe { resources.paint(painter.gl(), &projection) } {
                            log::error!("Hypergraphics paint failed: {error}");
                        }
                    });
                    ui.painter().add(egui::PaintCallback {
                        rect,
                        callback: Arc::new(callback),
                    });
                }
                Err(error) => log::warn!("camera projection rejected: {error}"),
            }
        });
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        let (Some(gl), Some(resources)) = (gl, self.resources.take()) else {
            return;
        };
        if let Ok(mut resources) = resources.lock() {
            unsafe { resources.destroy(gl) };
        }
    }
}

/// Start the native exact-stack application.
///
/// # Errors
///
/// Returns the native eframe startup or event-loop failure.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() -> eframe::Result {
    eframe::run_native(
        "Alumina",
        eframe::NativeOptions::default(),
        Box::new(|creation| Ok(Box::new(AluminaApp::new(creation)))),
    )
}

/// Start the browser exact-stack application.
///
/// # Errors
///
/// Returns a JavaScript error when browser discovery or eframe startup fails.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub async fn start() -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast as _;

    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    let document = web_sys::window()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("browser window is unavailable"))?
        .document()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("browser document is unavailable"))?;
    let canvas = document
        .get_element_by_id("alumina_canvas")
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("#alumina_canvas is missing"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|creation| Ok(Box::new(AluminaApp::new(creation)))),
        )
        .await
}
