//! Greenfield Alumina browser/native application shell.

#![warn(clippy::pedantic)]

#[cfg(target_arch = "wasm32")]
pub mod browser_worker;
pub mod cache_delivery;
pub mod distributed_schedule;
pub mod m7_simulation;

use std::sync::{Arc, Mutex};

use alumina_interface_core::{
    CanonicalGlobalJob2, CanonicalMachinePartition2, CanonicalPathProgram2, ExactScene, ExactValue,
    Millimetres, compile_representative_global_job, compile_representative_program,
    package_canonical_program, project_for_display, representative_partition_policy,
};
use eframe::egui;
use eframe::glow::HasContext as _;
use hypergraphics::backend::{GpuColoredMesh, UnlitProgram};
use hypergraphics::{ExactCamera, PredicatePolicy, Projection64, Real, Viewport};

#[cfg(target_arch = "wasm32")]
use crate::browser_worker::{BrowserWorkerSupervisor, SupervisorLifecycle};
use crate::m7_simulation::{RepresentativeM7SimulationReport, run_representative_m7_simulation};
#[cfg(target_arch = "wasm32")]
use alumina_interface_client::worker::{
    ClockSamplingPolicy, DeviceConnectionRequest, DeviceSessionPhase, DeviceSessionSnapshot,
    WorkerCommand,
};

#[cfg(target_arch = "wasm32")]
struct LiveDeviceForm {
    label: String,
    origin: String,
    secret: String,
    error: Option<String>,
}

#[cfg(target_arch = "wasm32")]
impl Default for LiveDeviceForm {
    fn default() -> Self {
        Self {
            label: "TinyBee bench".to_owned(),
            origin: "http://192.168.4.1".to_owned(),
            secret: String::new(),
            error: None,
        }
    }
}

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
    representative_global_job: Option<CanonicalGlobalJob2>,
    representative_m7_simulation: Option<RepresentativeM7SimulationReport>,
    resources: Option<Arc<Mutex<RenderResources>>>,
    setup_error: Option<String>,
    #[cfg(target_arch = "wasm32")]
    worker: Option<BrowserWorkerSupervisor>,
    #[cfg(target_arch = "wasm32")]
    worker_start_error: Option<String>,
    #[cfg(target_arch = "wasm32")]
    live_device_form: LiveDeviceForm,
    #[cfg(target_arch = "wasm32")]
    next_connection_id: u64,
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
        let (
            representative_program,
            representative_partition,
            representative_global_job,
            compiler_error,
        ) = match compile_representative_program() {
            Ok(program) => {
                let (partition, partition_error) = match representative_partition_policy()
                    .and_then(|policy| package_canonical_program(&program, policy))
                {
                    Ok(partition) => (Some(partition), None),
                    Err(error) => (
                        None,
                        Some(format!("canonical partition packaging failed: {error}")),
                    ),
                };
                let (global_job, global_error) = match compile_representative_global_job(&program) {
                    Ok(global_job) => (Some(global_job), None),
                    Err(error) => (
                        None,
                        Some(format!("global job compilation failed: {error}")),
                    ),
                };
                (
                    Some(program),
                    partition,
                    global_job,
                    partition_error.or(global_error),
                )
            }
            Err(error) => (
                None,
                None,
                None,
                Some(format!("exact representative compilation failed: {error}")),
            ),
        };
        let (representative_m7_simulation, simulation_error) =
            match representative_global_job.as_ref() {
                Some(job) => match run_representative_m7_simulation(job) {
                    Ok(report) => (Some(report), None),
                    Err(error) => (None, Some(error.to_string())),
                },
                None => (None, None),
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
        #[cfg(target_arch = "wasm32")]
        let (worker, worker_start_error) = match BrowserWorkerSupervisor::new(&creation.egui_ctx) {
            Ok(worker) => (Some(worker), None),
            Err(error) => (
                None,
                Some(format!("control worker failed to start: {error:?}")),
            ),
        };
        Self {
            scene,
            camera: ExactCamera::default(),
            representative_program,
            representative_partition,
            representative_global_job,
            representative_m7_simulation,
            resources,
            setup_error: scene_error
                .or(compiler_error)
                .or(simulation_error)
                .or(renderer_error),
            #[cfg(target_arch = "wasm32")]
            worker,
            #[cfg(target_arch = "wasm32")]
            worker_start_error,
            #[cfg(target_arch = "wasm32")]
            live_device_form: LiveDeviceForm::default(),
            #[cfg(target_arch = "wasm32")]
            next_connection_id: 1,
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

    fn show_scene_status(&self, ui: &mut egui::Ui) {
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
    }

    fn show_motion_status(&self, ui: &mut egui::Ui) {
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
    }

    fn show_cached_job_status(&self, ui: &mut egui::Ui) {
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
        if let Some(global_job) = &self.representative_global_job {
            ui.separator();
            ui.label(format!(
                "Global job participants: {}",
                global_job.participants().len()
            ));
            ui.label(format!(
                "Canonical global manifest: {} bytes",
                global_job.manifest_bytes().len()
            ));
            ui.label(format!(
                "Global manifest chunks: {}",
                global_job.manifest_chunks().len()
            ));
        }
        self.show_m7_simulation_status(ui);
    }

    fn show_m7_simulation_status(&self, ui: &mut egui::Ui) {
        let Some(report) = &self.representative_m7_simulation else {
            return;
        };
        ui.separator();
        ui.label("M7 deterministic coordinator fixture");
        ui.label(format!(
            "Global terminal phase: {:?}",
            report.terminal_phase
        ));
        ui.label(format!("Shared UI epoch: {} ns", report.target_ui_ns));
        ui.horizontal_wrapped(|ui| {
            ui.label("Observed phases:");
            for phase in &report.observed_global_phases {
                ui.label(format!("{phase:?}"));
            }
        });
        ui.label(format!(
            "Pre-confirm window: {:?}",
            report.confirmation_window
        ));
        ui.label(format!(
            "Simulated start-edge spread: {} ns",
            report.simulated_edge_spread_ns
        ));
        ui.label(format!(
            "Maximum shared-epoch error: {} ns",
            report.maximum_target_error_ns
        ));
        ui.label(format!(
            "Lost install response reconciled: {}",
            report.lost_install_reconciled
        ));
        for participant in &report.participants {
            let bytes = participant.device_id.0;
            ui.collapsing(
                format!(
                    "MCU {:02x}{:02x}{:02x}{:02x}…",
                    bytes[0], bytes[1], bytes[2], bytes[3]
                ),
                |ui| {
                    ui.label(format!(
                        "clock: {:+} ppm, {} accepted / {} rejected",
                        participant.rate_adjustment_ppm,
                        participant.accepted_clock_samples,
                        participant.rejected_clock_samples
                    ));
                    ui.label(format!(
                        "start: cycle {}, uncertainty ±{} cycles",
                        participant.scheduled_cycle.0, participant.clock_uncertainty_cycles
                    ));
                    ui.label(format!(
                        "simulated edge: {} ns, phase {:?}",
                        participant.simulated_start_ui_ns, participant.terminal_phase
                    ));
                },
            );
        }
        ui.colored_label(
            egui::Color32::YELLOW,
            "Simulation only — no live device, output, or safety-chain evidence.",
        );
    }

    fn show_status(&self, ui: &mut egui::Ui) {
        ui.heading("Alumina");
        ui.label("Greenfield exact CAD/CAM baseline");
        ui.separator();
        self.show_scene_status(ui);
        self.show_motion_status(ui);
        self.show_cached_job_status(ui);
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

    #[cfg(target_arch = "wasm32")]
    fn show_live_control(&mut self, ui: &mut egui::Ui) {
        ui.heading("Live MCU clocks");
        ui.label("Dedicated worker; authenticated Wi-Fi heartbeat only");
        ui.colored_label(
            egui::Color32::YELLOW,
            "Diagnostic connection does not arm outputs or prove physical safety.",
        );
        ui.separator();

        let view = self.worker.as_ref().map(BrowserWorkerSupervisor::view);
        match view.as_ref().map(|view| &view.lifecycle) {
            Some(SupervisorLifecycle::Starting) => {
                ui.label("Control worker: starting");
            }
            Some(SupervisorLifecycle::Ready { scope_origin }) => {
                ui.label(format!("Control worker: ready ({scope_origin})"));
            }
            Some(SupervisorLifecycle::Failed(error)) => {
                ui.colored_label(
                    egui::Color32::RED,
                    format!("Control worker failed: {error}"),
                );
            }
            None => {
                ui.colored_label(
                    egui::Color32::RED,
                    self.worker_start_error
                        .as_deref()
                        .unwrap_or("Control worker is unavailable"),
                );
            }
        }

        self.show_live_connection_form(ui);

        let Some(view) = view else {
            return;
        };
        let mut actions = Vec::new();
        for snapshot in &view.devices {
            show_live_device_snapshot(ui, snapshot, &mut actions);
        }
        self.apply_live_actions(actions);
        if !view.diagnostics.is_empty() {
            ui.separator();
            ui.collapsing("Worker diagnostics", |ui| {
                for diagnostic in &view.diagnostics {
                    ui.colored_label(egui::Color32::LIGHT_RED, diagnostic);
                }
            });
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn show_live_connection_form(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Add diagnostic connection", |ui| {
            ui.label("Label");
            ui.text_edit_singleline(&mut self.live_device_form.label);
            ui.label("Device origin");
            ui.text_edit_singleline(&mut self.live_device_form.origin);
            ui.label("HMAC/AP passphrase");
            ui.add(
                egui::TextEdit::singleline(&mut self.live_device_form.secret)
                    .password(true)
                    .hint_text("Development default: alumina-development"),
            );
            if ui.button("Connect for clock diagnostics").clicked() {
                self.connect_live_device();
            }
            if let Some(error) = &self.live_device_form.error {
                ui.colored_label(egui::Color32::RED, error);
            }
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn apply_live_actions(&mut self, actions: Vec<(u64, bool)>) {
        for (connection_id, disconnect) in actions {
            let command = if disconnect {
                WorkerCommand::Disconnect { connection_id }
            } else {
                WorkerCommand::ProbeNow { connection_id }
            };
            if let Some(worker) = &self.worker
                && let Err(error) = worker.send(command)
            {
                self.live_device_form.error = Some(error);
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn connect_live_device(&mut self) {
        let Some(worker) = &self.worker else {
            self.live_device_form.error = Some("control worker is unavailable".to_owned());
            return;
        };
        let request = DeviceConnectionRequest {
            connection_id: self.next_connection_id,
            label: self.live_device_form.label.clone(),
            origin: self.live_device_form.origin.clone(),
            secret: self.live_device_form.secret.as_bytes().to_vec(),
            sampling: ClockSamplingPolicy::CONSERVATIVE_WIFI,
        };
        if let Err(error) = request.validate() {
            self.live_device_form.error = Some(error.to_string());
            return;
        }
        match worker.send(WorkerCommand::Configure { request }) {
            Ok(()) => {
                let mut secret = std::mem::take(&mut self.live_device_form.secret).into_bytes();
                secret.fill(0);
                self.live_device_form.error = None;
                self.next_connection_id = self.next_connection_id.saturating_add(1).max(1);
            }
            Err(error) => self.live_device_form.error = Some(error),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn show_live_device_snapshot(
    ui: &mut egui::Ui,
    snapshot: &DeviceSessionSnapshot,
    actions: &mut Vec<(u64, bool)>,
) {
    ui.separator();
    ui.collapsing(
        format!("{} — {:?}", snapshot.label, snapshot.phase),
        |ui| {
            ui.label(format!("origin: {}", snapshot.origin));
            ui.label(format!("session generation: {}", snapshot.generation));
            if let Some(boot_id) = snapshot.boot_id {
                ui.label(format!("boot: {}", encode_hex(&boot_id)));
            }
            ui.label(format!(
                "clock samples: {} accepted / {} rejected",
                snapshot.accepted_samples, snapshot.rejected_samples
            ));
            ui.label(format!(
                "consecutive failures: {}",
                snapshot.consecutive_failures
            ));
            if let Some(estimate) = snapshot.estimate {
                ui.label(format!(
                    "cycle interval @ {} ns: {}..={} (±{})",
                    estimate.ui_ns,
                    estimate.earliest_cycle,
                    estimate.latest_cycle,
                    estimate.uncertainty_cycles
                ));
            }
            if let Some(latest) = snapshot.history.last() {
                ui.label(format!(
                    "latest causal span: {} ns; device work: {} cycles",
                    latest.causal_span_ns, latest.processing_cycles
                ));
                ui.label(format!(
                    "clock: {} Hz; queue free/depth: {}/{}",
                    latest.frequency_hz, latest.command_queue_free, latest.work_queue_depth
                ));
                ui.label(format!(
                    "deadline misses: {}; raw flags: 0x{:04x}",
                    latest.missed_deadlines, latest.flags
                ));
            }
            if snapshot.phase == DeviceSessionPhase::DeviceUnhealthy {
                ui.colored_label(
                    egui::Color32::RED,
                    "Device reports unhealthy deadline or safety state.",
                );
            }
            if let Some(error) = &snapshot.last_error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.horizontal(|ui| {
                if ui.button("Probe now").clicked() {
                    actions.push((snapshot.connection_id, false));
                }
                if ui.button("Disconnect").clicked() {
                    actions.push((snapshot.connection_id, true));
                }
            });
            if snapshot.history.len() > 1 {
                ui.collapsing("Recent causal spans", |ui| {
                    for record in snapshot.history.iter().rev().take(8) {
                        ui.label(format!(
                            "#{}: {} ns, cycles {}..{}",
                            record.probe_id,
                            record.causal_span_ns,
                            record.receive_cycle,
                            record.transmit_cycle
                        ));
                    }
                });
            }
        },
    );
}

#[cfg(target_arch = "wasm32")]
fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing into a string is infallible");
    }
    encoded
}

impl eframe::App for AluminaApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        egui::SidePanel::left("exact_stack_status")
            .resizable(false)
            .default_width(260.0)
            .show(context, |ui| self.show_status(ui));

        #[cfg(target_arch = "wasm32")]
        egui::SidePanel::right("live_mcu_status")
            .resizable(true)
            .default_width(380.0)
            .show(context, |ui| self.show_live_control(ui));

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
    let Some(window) = web_sys::window() else {
        return Ok(());
    };
    let document = window
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

/// Explicitly install the dedicated browser control worker after WASM loading.
///
/// The module-worker bootstrap calls this synchronous entry rather than relying
/// on the asynchronous application start hook, making readiness and bootstrap
/// failures observable to the rendering realm.
///
/// # Errors
///
/// Returns a JavaScript error outside a dedicated worker or when its control
/// timer/origin cannot be installed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_control_worker() -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast as _;

    let worker = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>()?;
    browser_worker::install_control_worker(&worker)
}
