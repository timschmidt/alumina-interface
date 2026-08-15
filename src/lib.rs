//! Greenfield Alumina browser/native application shell.

#![warn(clippy::pedantic)]

#[cfg(target_arch = "wasm32")]
pub mod browser_worker;
pub mod cache_delivery;
mod control_graph_ui;
pub mod distributed_schedule;
pub mod live_job;
pub mod m7_simulation;
mod machine_cam_ui;
mod workspace_file;

use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use std::collections::BTreeMap;

#[cfg(target_arch = "wasm32")]
use alumina_capability::encode_resource_id;
#[cfg(target_arch = "wasm32")]
use alumina_diagnostics::{
    DigitalCaptureFlags, DigitalLevel, OverviewFlags, ResourceValue, SampleQuality,
};
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
use crate::browser_worker::{
    BrowserWorkerSupervisor, ConnectedCapabilityView, ConnectedTelemetryView,
    ConnectedWaveformView, SupervisorLifecycle,
};
use crate::control_graph_ui::ExactControlWorkspace;
#[cfg(target_arch = "wasm32")]
use crate::control_graph_ui::WORKSPACE_STORAGE_KEY;
use crate::m7_simulation::{RepresentativeM7SimulationReport, run_representative_m7_simulation};
use crate::machine_cam_ui::MachineCamDeploymentTarget;
use crate::machine_cam_ui::MachineCamWorkspace;
#[cfg(target_arch = "wasm32")]
use alumina_interface_client::worker::{
    CapabilityDownloadPhaseSnapshot, ClockSamplingPolicy, ConfigurationStatusAvailabilitySnapshot,
    DeviceConnectionRequest, DeviceSessionPhase, DeviceSessionSnapshot, ExecutorStackSnapshot,
    RuntimeHealthAvailabilitySnapshot, WaveformCapturePhaseSnapshot, WorkerCachedJobPhaseSnapshot,
    WorkerCachedJobSnapshot, WorkerCommand, WorkerWaveformRequest,
};

/// Exact connected-MCU authority consumed by the representative browser CAM compiler.
///
/// This narrow entry point is shared by the egui shell and transport integration harnesses;
/// it grants no device access and performs no network or hardware operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CachedJobDeploymentTarget {
    /// UI-local worker connection identity.
    pub connection_id: u64,
    /// Worker generation observed with the remaining authority facts.
    pub generation: u64,
    /// Stable physical or simulated MCU identity.
    pub device_id: alumina_protocol::DeviceId,
    /// Authenticated boot identity to bind into the preparation receipt.
    pub boot_id: [u8; 16],
    /// Exact immutable board capability used for compilation.
    pub capability_digest: alumina_protocol::Digest,
    /// Exact active machine configuration used for compilation.
    pub config_digest: alumina_protocol::Digest,
}

/// Compiles the built-in exact geometry fixture into one strict worker cache handoff.
///
/// The returned bytes follow the same Hyper-backed CAM path used by the visible machine workspace.
/// No request is sent and no output authority is implied.
///
/// # Errors
///
/// Rejects missing, heterogeneous, stale, or unsupported target authority and every CAM,
/// approximation, schedule, replay, packaging, or worker-contract failure.
pub fn compile_representative_cached_job_request(
    job_id: u64,
    targets: &[CachedJobDeploymentTarget],
) -> Result<alumina_interface_client::worker::WorkerCachedJobRequest, String> {
    let workspace = MachineCamWorkspace::try_new()?;
    let targets: Vec<_> = targets
        .iter()
        .map(|target| MachineCamDeploymentTarget {
            connection_id: target.connection_id,
            generation: target.generation,
            device_id: target.device_id,
            boot_id: target.boot_id,
            capability_digest: target.capability_digest,
            config_digest: target.config_digest,
        })
        .collect();
    workspace.build_cached_job_request(job_id, &targets)
}

#[cfg(target_arch = "wasm32")]
struct LiveDeviceForm {
    label: String,
    origin: String,
    secret: String,
    error: Option<String>,
}

#[cfg(target_arch = "wasm32")]
enum LiveDeviceAction {
    Probe(u64),
    Capture(WorkerWaveformRequest),
    Disconnect(u64),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceView {
    MachineCam,
    Geometry,
    ExactControl,
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

fn initialize_exact_control_workspace() -> (Option<ExactControlWorkspace>, Option<String>) {
    #[cfg(target_arch = "wasm32")]
    let persisted = load_persisted_exact_control_workspace();
    #[cfg(not(target_arch = "wasm32"))]
    let persisted: Result<Option<String>, String> = Ok(None);

    let (persisted, persistence_error) = match persisted {
        Ok(persisted) => (persisted, None),
        Err(error) => (None, Some(error)),
    };
    match ExactControlWorkspace::try_new_with_persisted(persisted.as_deref()) {
        Ok(mut workspace) => {
            if let Some(error) = persistence_error {
                workspace.note_persistence_error(&error);
            }
            (Some(workspace), None)
        }
        Err(error) => (
            None,
            Some(format!("exact control workspace failed: {error}")),
        ),
    }
}

/// Minimal exact-stack application used by native and browser runners.
pub struct AluminaApp {
    workspace_view: WorkspaceView,
    scene: ExactScene,
    camera: ExactCamera,
    representative_program: Option<CanonicalPathProgram2>,
    representative_partition: Option<CanonicalMachinePartition2>,
    representative_global_job: Option<CanonicalGlobalJob2>,
    representative_m7_simulation: Option<RepresentativeM7SimulationReport>,
    machine_cam: Option<MachineCamWorkspace>,
    exact_control: Option<ExactControlWorkspace>,
    resources: Option<Arc<Mutex<RenderResources>>>,
    setup_error: Option<String>,
    exact_control_error: Option<String>,
    machine_cam_error: Option<String>,
    #[cfg(target_arch = "wasm32")]
    worker: Option<BrowserWorkerSupervisor>,
    #[cfg(target_arch = "wasm32")]
    worker_start_error: Option<String>,
    #[cfg(target_arch = "wasm32")]
    live_device_form: LiveDeviceForm,
    #[cfg(target_arch = "wasm32")]
    next_connection_id: u64,
    #[cfg(target_arch = "wasm32")]
    live_capture_cursors: BTreeMap<u64, u64>,
    #[cfg(target_arch = "wasm32")]
    next_job_id: u64,
    #[cfg(target_arch = "wasm32")]
    live_job_error: Option<String>,
}

impl AluminaApp {
    /// Construct the exact baseline scene and upload it through Hypergraphics.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "application construction keeps independently fallible exact workspaces and GPU resources explicit"
    )]
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
        let (exact_control, exact_control_error) = initialize_exact_control_workspace();
        let (machine_cam, machine_cam_error) = match MachineCamWorkspace::try_new() {
            Ok(workspace) => (Some(workspace), None),
            Err(error) => (None, Some(format!("machine/CAM workspace failed: {error}"))),
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
            workspace_view: WorkspaceView::MachineCam,
            scene,
            camera: ExactCamera::default(),
            representative_program,
            representative_partition,
            representative_global_job,
            representative_m7_simulation,
            machine_cam,
            exact_control,
            resources,
            setup_error: scene_error
                .or(compiler_error)
                .or(simulation_error)
                .or(renderer_error),
            exact_control_error,
            machine_cam_error,
            #[cfg(target_arch = "wasm32")]
            worker,
            #[cfg(target_arch = "wasm32")]
            worker_start_error,
            #[cfg(target_arch = "wasm32")]
            live_device_form: LiveDeviceForm::default(),
            #[cfg(target_arch = "wasm32")]
            next_connection_id: 1,
            #[cfg(target_arch = "wasm32")]
            live_capture_cursors: BTreeMap::new(),
            #[cfg(target_arch = "wasm32")]
            next_job_id: 1,
            #[cfg(target_arch = "wasm32")]
            live_job_error: None,
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
            "Conservative replay spread: <= {} ns",
            report.maximum_reconciled_edge_spread_ns
        ));
        ui.label(format!(
            "Maximum shared-epoch error: {} ns",
            report.maximum_target_error_ns
        ));
        ui.label(format!(
            "Conservative replay target error: <= {} ns",
            report.maximum_reconciled_target_error_ns
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
                        "simulated edge: {} ns, source {:?}, phase {:?}",
                        participant.simulated_start_ui_ns,
                        participant.observation_source,
                        participant.terminal_phase
                    ));
                    ui.label(format!(
                        "replayed interval: {}..={} ns",
                        participant.reconciled_earliest_ui_ns, participant.reconciled_latest_ui_ns
                    ));
                },
            );
        }
        ui.colored_label(
            egui::Color32::YELLOW,
            "Simulation only — no live device, output, or safety-chain evidence.",
        );
    }

    fn show_status(&mut self, ui: &mut egui::Ui) {
        ui.heading("Alumina");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.workspace_view,
                WorkspaceView::MachineCam,
                "Machine/CAM",
            );
            ui.selectable_value(
                &mut self.workspace_view,
                WorkspaceView::Geometry,
                "Exact geometry",
            );
            ui.selectable_value(
                &mut self.workspace_view,
                WorkspaceView::ExactControl,
                "Control graph",
            );
        });
        ui.separator();
        match self.workspace_view {
            WorkspaceView::MachineCam => match &self.machine_cam {
                Some(workspace) => workspace.show_sidebar(ui),
                None => {
                    ui.colored_label(
                        egui::Color32::RED,
                        self.machine_cam_error
                            .as_deref()
                            .unwrap_or("machine/CAM workspace is unavailable"),
                    );
                }
            },
            WorkspaceView::Geometry => {
                ui.label("Greenfield exact CAD/CAM baseline");
                self.show_scene_status(ui);
                self.show_motion_status(ui);
                self.show_cached_job_status(ui);
            }
            WorkspaceView::ExactControl => match &self.exact_control {
                Some(workspace) => workspace.show_sidebar(ui),
                None => {
                    ui.colored_label(
                        egui::Color32::RED,
                        self.exact_control_error
                            .as_deref()
                            .unwrap_or("exact control workspace is unavailable"),
                    );
                }
            },
        }
        ui.separator();
        ui.label(format!(
            "Protocol schema: {}",
            alumina_protocol::PROTOCOL_VERSION
        ));
        match self.workspace_view {
            WorkspaceView::MachineCam => {
                ui.label("Inspect exact configuration, schedules, cached IR, and replay evidence.");
                ui.label("No machine value originates from this display projection.");
            }
            WorkspaceView::Geometry => {
                let tenth = ExactValue::<Millimetres>::parse_decimal("0.1")
                    .expect("static exact decimal is valid");
                let display = project_for_display(&tenth)
                    .expect("small exact value has a finite display projection");
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
            WorkspaceView::ExactControl => {
                ui.label("Scroll the graph canvas; hover the trace to inspect an exact tick.");
                ui.label("No graph node shown here can arm or command firmware.");
            }
        }
    }

    fn show_geometry_workspace(&mut self, ui: &mut egui::Ui) {
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
    }

    #[cfg(target_arch = "wasm32")]
    fn show_live_control(&mut self, ui: &mut egui::Ui) {
        ui.heading("Live MCUs");
        ui.label("Dedicated worker; authenticated Wi-Fi clock, health, and board authority");
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
        self.show_live_job_control(ui, &view);
        let mut actions = Vec::new();
        for snapshot in &view.devices {
            let capability = view.capabilities.iter().find(|capability| {
                capability.connection_id() == snapshot.connection_id
                    && capability.generation() == snapshot.generation
            });
            let waveform = view.waveforms.iter().find(|waveform| {
                waveform.connection_id() == snapshot.connection_id
                    && waveform.generation() == snapshot.generation
                    && snapshot.waveform_capture_id == Some(waveform.capture_id())
            });
            let telemetry = view.telemetry.iter().find(|telemetry| {
                telemetry.connection_id() == snapshot.connection_id
                    && telemetry.generation() == snapshot.generation
            });
            let cursor = self
                .live_capture_cursors
                .entry(snapshot.connection_id)
                .or_insert(0);
            show_live_device_snapshot(
                ui,
                snapshot,
                capability,
                telemetry,
                waveform,
                cursor,
                &mut actions,
            );
        }
        self.live_capture_cursors.retain(|connection_id, _| {
            view.devices
                .iter()
                .any(|snapshot| snapshot.connection_id == *connection_id)
        });
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
    fn show_live_job_control(
        &mut self,
        ui: &mut egui::Ui,
        view: &crate::browser_worker::SupervisorView,
    ) {
        ui.separator();
        ui.heading("Exact cached job");
        let targets = live_cam_targets(view);
        ui.label(format!(
            "{} connected MCU(s) have matching boot, capability, active configuration, and qualified clock authority",
            targets.len()
        ));
        ui.weak(
            "Stage recompiles the current exact CAM source for every selected live identity, then transfers only immutable manifest/partition artifacts to the worker.",
        );

        if let Some(job) = &view.job {
            show_live_job_snapshot(ui, job);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        job.phase == WorkerCachedJobPhaseSnapshot::Ready,
                        egui::Button::new("Start at common future epoch"),
                    )
                    .on_hover_text(
                        "The worker independently revalidates armability, production credentials, clocks, boots, and active configurations before installing any commit.",
                    )
                    .clicked()
                {
                    self.send_live_job_command(WorkerCommand::StartCachedJob {
                        job_id: job.job_id,
                    });
                }
                let stoppable = !matches!(
                    job.phase,
                    WorkerCachedJobPhaseSnapshot::Aborted
                        | WorkerCachedJobPhaseSnapshot::Cancelled
                        | WorkerCachedJobPhaseSnapshot::Complete
                        | WorkerCachedJobPhaseSnapshot::CompletedAfterStopRequest
                        | WorkerCachedJobPhaseSnapshot::SplitAfterStopRequest
                        | WorkerCachedJobPhaseSnapshot::RetainedComplete
                        | WorkerCachedJobPhaseSnapshot::Faulted
                );
                if ui
                    .add_enabled(stoppable, egui::Button::new("Stop safely"))
                    .clicked()
                {
                    self.send_live_job_command(WorkerCommand::StopCachedJob {
                        job_id: job.job_id,
                    });
                }
                let clearable = matches!(
                    job.phase,
                    WorkerCachedJobPhaseSnapshot::Aborted
                        | WorkerCachedJobPhaseSnapshot::Cancelled
                        | WorkerCachedJobPhaseSnapshot::Complete
                        | WorkerCachedJobPhaseSnapshot::CompletedAfterStopRequest
                        | WorkerCachedJobPhaseSnapshot::SplitAfterStopRequest
                        | WorkerCachedJobPhaseSnapshot::RetainedComplete
                        | WorkerCachedJobPhaseSnapshot::Faulted
                );
                if ui
                    .add_enabled(clearable, egui::Button::new("Clear retained job"))
                    .clicked()
                {
                    self.send_live_job_command(WorkerCommand::ClearCachedJob {
                        job_id: job.job_id,
                    });
                }
            });
        } else if ui
            .add_enabled(
                !targets.is_empty() && self.machine_cam.is_some(),
                egui::Button::new("Compile exact CAM and stage immutable cache"),
            )
            .clicked()
        {
            let job_id = self.next_job_id;
            let result = self
                .machine_cam
                .as_ref()
                .ok_or_else(|| "machine/CAM workspace is unavailable".to_owned())
                .and_then(|workspace| workspace.build_cached_job_request(job_id, &targets))
                .and_then(|request| {
                    self.worker
                        .as_ref()
                        .ok_or_else(|| "control worker is unavailable".to_owned())?
                        .send(WorkerCommand::StageCachedJob {
                            request: Box::new(request),
                        })
                });
            match result {
                Ok(()) => {
                    self.next_job_id = self.next_job_id.saturating_add(1).max(1);
                    self.live_job_error = None;
                }
                Err(error) => self.live_job_error = Some(error),
            }
        }
        if let Some(error) = &self.live_job_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn send_live_job_command(&mut self, command: WorkerCommand) {
        let result = self
            .worker
            .as_ref()
            .ok_or_else(|| "control worker is unavailable".to_owned())
            .and_then(|worker| worker.send(command));
        match result {
            Ok(()) => self.live_job_error = None,
            Err(error) => self.live_job_error = Some(error),
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
    fn persist_exact_control_workspace(&mut self) {
        let Some(workspace) = self.exact_control.as_mut() else {
            return;
        };
        if !workspace.persistence_pending() {
            return;
        }
        let persisted = match workspace.persisted_workspace() {
            Ok(persisted) => persisted,
            Err(error) => {
                workspace.note_persistence_error(&error);
                return;
            }
        };
        let result = browser_local_storage().and_then(|storage| {
            storage
                .set_item(WORKSPACE_STORAGE_KEY, &persisted)
                .map_err(|value| browser_value_text(&value))
        });
        match result {
            Ok(()) => workspace.mark_persisted(),
            Err(error) => workspace.note_persistence_error(&error),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn apply_live_actions(&mut self, actions: Vec<LiveDeviceAction>) {
        for action in actions {
            let command = match action {
                LiveDeviceAction::Probe(connection_id) => WorkerCommand::ProbeNow { connection_id },
                LiveDeviceAction::Capture(request) => WorkerCommand::CaptureWaveform { request },
                LiveDeviceAction::Disconnect(connection_id) => {
                    WorkerCommand::Disconnect { connection_id }
                }
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
fn live_cam_targets(
    view: &crate::browser_worker::SupervisorView,
) -> Vec<MachineCamDeploymentTarget> {
    view.devices
        .iter()
        .filter_map(|snapshot| {
            if snapshot.phase != DeviceSessionPhase::ClockQualified
                || snapshot.capability_phase != CapabilityDownloadPhaseSnapshot::Complete
            {
                return None;
            }
            let boot_id = snapshot.boot_id?;
            let identity = snapshot.device_identity.as_ref()?;
            let capability = identity.capability.identity().ok()?;
            let configuration = snapshot.configuration?;
            if !configuration.jobs_authorized
                || configuration.active_digest.iter().all(|byte| *byte == 0)
                || !view.capabilities.iter().any(|document| {
                    document.connection_id() == snapshot.connection_id
                        && document.generation() == snapshot.generation
                        && document.board().identity() == capability
                })
            {
                return None;
            }
            Some(MachineCamDeploymentTarget {
                connection_id: snapshot.connection_id,
                generation: snapshot.generation,
                device_id: alumina_protocol::DeviceId(identity.device_id),
                boot_id,
                capability_digest: capability.digest,
                config_digest: alumina_protocol::Digest(configuration.active_digest),
            })
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn display_cache_progress(accepted_bytes: u64, total_bytes: u64) -> f32 {
    const DISPLAY_STEPS: u128 = 10_000;
    if total_bytes == 0 {
        return 0.0;
    }
    let scaled =
        u128::from(accepted_bytes.min(total_bytes)) * DISPLAY_STEPS / u128::from(total_bytes);
    let scaled = u16::try_from(scaled).unwrap_or(10_000);
    f32::from(scaled) / 10_000.0
}

#[cfg(target_arch = "wasm32")]
fn show_live_job_snapshot(ui: &mut egui::Ui, job: &WorkerCachedJobSnapshot) {
    ui.separator();
    ui.strong(format!(
        "job {} · {:?} · {:?}",
        job.job_id, job.execution_mode, job.phase
    ));
    ui.monospace(format!(
        "manifest {}… / {} bytes · participants {}…",
        encode_hex(&job.global_job_digest[..8]),
        job.manifest_byte_len,
        encode_hex(&job.participant_set_digest[..8])
    ));
    if let Some(target_ui_ns) = job.target_ui_ns {
        ui.monospace(format!("shared future epoch: {target_ui_ns} ns"));
    }
    if job.execution_mode == alumina_interface_client::worker::WorkerJobExecutionMode::Hardware {
        ui.colored_label(
            egui::Color32::YELLOW,
            "Hardware cache may be staged, but Start remains fail-closed unless every board and device-stored credential is production-armable.",
        );
    }
    if job.phase == WorkerCachedJobPhaseSnapshot::CompletedAfterStopRequest {
        ui.colored_label(
            egui::Color32::RED,
            "The job completed after a stop request crossed the abort point of no return. Treat the stop as missed; Wi-Fi is not a safety chain.",
        );
    }
    if job.phase == WorkerCachedJobPhaseSnapshot::SplitAfterStopRequest {
        ui.colored_label(
            egui::Color32::RED,
            "The stop split the job: some participants stopped while others crossed the point of no return and completed. Treat machine state as indeterminate; Wi-Fi is not a safety chain.",
        );
    }
    for participant in &job.participants {
        ui.collapsing(
            format!(
                "MCU {}… · {:?}",
                encode_hex(&participant.device_id[..4]),
                participant.schedule_phase
            ),
            |ui| {
                ui.label(format!(
                    "connection {} generation {} · cache {:?}/{:?}",
                    participant.connection_id,
                    participant.generation,
                    participant.cache_artifact,
                    participant.cache_phase
                ));
                let fraction =
                    display_cache_progress(participant.accepted_bytes, participant.total_bytes);
                ui.add(
                    egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).text(format!(
                        "{} / {} bytes",
                        participant.accepted_bytes, participant.total_bytes
                    )),
                );
                ui.monospace(format!(
                    "capability {}… · config {}… · boot {}…",
                    encode_hex(&participant.capability_digest[..8]),
                    encode_hex(&participant.config_digest[..8]),
                    encode_hex(&participant.boot_id[..8])
                ));
                if let Some(cycle) = participant.local_start_cycle {
                    ui.monospace(format!("bound local start cycle: {cycle}"));
                }
            },
        );
    }
    if job.consecutive_failures != 0 {
        ui.label(format!(
            "consecutive job failures: {}",
            job.consecutive_failures
        ));
    }
    if let Some(error) = &job.last_error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
    }
}

#[cfg(target_arch = "wasm32")]
fn show_live_device_snapshot(
    ui: &mut egui::Ui,
    snapshot: &DeviceSessionSnapshot,
    capability: Option<&ConnectedCapabilityView>,
    telemetry: Option<&ConnectedTelemetryView>,
    waveform: Option<&ConnectedWaveformView>,
    capture_cursor: &mut u64,
    actions: &mut Vec<LiveDeviceAction>,
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
            if let Some(identity) = &snapshot.device_identity {
                ui.label(format!(
                    "device: {} · board claim: {} · credential: {:?} · production-armable credential: {}",
                    encode_hex(&identity.device_id),
                    identity.board_id,
                    identity.credential_source,
                    identity.credential_source.production_armable()
                ));
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
            show_live_capability_snapshot(ui, snapshot, capability);
            show_live_configuration_snapshot(ui, snapshot);
            show_runtime_health_snapshot(ui, snapshot);
            show_live_telemetry_status(ui, snapshot, capability, telemetry);
            show_live_waveform_status(ui, snapshot, capability, waveform, capture_cursor);
            if snapshot.phase == DeviceSessionPhase::DeviceUnhealthy {
                ui.colored_label(
                    egui::Color32::RED,
                    "Device reports unhealthy deadline or safety state.",
                );
            }
            if let Some(error) = &snapshot.last_error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            show_live_device_actions(ui, snapshot, capability, actions);
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
fn show_live_device_actions(
    ui: &mut egui::Ui,
    snapshot: &DeviceSessionSnapshot,
    capability: Option<&ConnectedCapabilityView>,
    actions: &mut Vec<LiveDeviceAction>,
) {
    ui.horizontal(|ui| {
        if ui.button("Probe now").clicked() {
            actions.push(LiveDeviceAction::Probe(snapshot.connection_id));
        }
        if let Some(request) = default_live_waveform_request(snapshot, capability) {
            let available = matches!(
                snapshot.waveform_phase,
                None | Some(
                    WaveformCapturePhaseSnapshot::Complete
                        | WaveformCapturePhaseSnapshot::Stopped
                )
            );
            if ui
                .add_enabled(available, egui::Button::new("Capture inputs (2 ms)"))
                .on_hover_text(
                    "Starts only the diagnostic input acquisition; it grants no machine arm, output, or resource-write authority.",
                )
                .clicked()
            {
                actions.push(LiveDeviceAction::Capture(request));
            }
        }
        if ui.button("Disconnect").clicked() {
            actions.push(LiveDeviceAction::Disconnect(snapshot.connection_id));
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn show_live_capability_snapshot(
    ui: &mut egui::Ui,
    snapshot: &DeviceSessionSnapshot,
    capability: Option<&ConnectedCapabilityView>,
) {
    ui.separator();
    ui.strong("Authenticated board capability");
    show_capability_download_phase(ui, snapshot);
    if let Some(capability) = capability {
        show_admitted_board_capability(ui, capability);
    } else if snapshot.capability_phase == CapabilityDownloadPhaseSnapshot::Complete {
        ui.label("Validated one-time board transfer is awaiting UI admission.");
    }
    if snapshot.capability_consecutive_failures != 0 {
        ui.label(format!(
            "consecutive capability failures: {}",
            snapshot.capability_consecutive_failures
        ));
    }
    if let Some(error) = &snapshot.capability_last_error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
    }
}

#[cfg(target_arch = "wasm32")]
fn show_capability_download_phase(ui: &mut egui::Ui, snapshot: &DeviceSessionSnapshot) {
    match snapshot.capability_phase {
        CapabilityDownloadPhaseSnapshot::Discovering => {
            ui.label("Waiting for the first signed canonical capability range.");
        }
        CapabilityDownloadPhaseSnapshot::Downloading => {
            if let Some(identity) = snapshot.capability_identity {
                ui.label(format!(
                    "contiguous bytes: {} / {}; identity {}…",
                    snapshot.capability_received_bytes,
                    identity.document_bytes,
                    encode_hex(&identity.digest[..8])
                ));
            } else {
                ui.colored_label(
                    egui::Color32::RED,
                    "Capability download lacks its required stable identity.",
                );
            }
        }
        CapabilityDownloadPhaseSnapshot::Complete => {
            if let Some(identity) = snapshot.capability_identity {
                ui.label(format!(
                    "complete canonical document: {} bytes · {}…",
                    identity.document_bytes,
                    encode_hex(&identity.digest[..8])
                ));
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn show_admitted_board_capability(ui: &mut egui::Ui, capability: &ConnectedCapabilityView) {
    let board = capability.board();
    let summary = board.resource_summary();
    let (flash, internal_sram, psram) = board.memory_bytes();
    let (service_core, realtime_core) = board.core_assignment();
    let hotspot_count = board
        .visuals()
        .iter()
        .map(|visual| visual.hotspots().len())
        .sum::<usize>();
    ui.label(format!(
        "{} · {} · {:?} / {:?} · {} cores",
        board.board_id(),
        board.revision(),
        board.chip(),
        board.qualification(),
        board.application_cores()
    ));
    ui.label(format!(
        "service core {service_core}; real-time core {realtime_core}; flash {flash}; internal SRAM {internal_sram}; PSRAM {psram} bytes"
    ));
    ui.label(format!(
        "{} resources ({} service, {} real-time, {} hazardous, {} graph-addressable, {} passively observable, {} digitally capturable); {} aliases",
        board.resources().len(),
        summary.service,
        summary.realtime,
        summary.hazardous,
        summary.graph_addressable,
        summary.diagnostic_observable,
        summary.digitally_capturable,
        board.alias_count()
    ));
    let diagnostics = board.diagnostic_overview();
    if diagnostics.is_implemented() {
        ui.label(format!(
            "diagnostic overview V{}: {} / {} resources; {} / {} B request/event; {} µs cadence; {} µs freshness ceiling",
            diagnostics.schema_version,
            diagnostics.resource_count,
            diagnostics.maximum_resources,
            diagnostics.telemetry_request_bytes,
            diagnostics.telemetry_event_bytes,
            diagnostics.nominal_period_micros,
            diagnostics.maximum_age_micros,
        ));
    } else {
        ui.label("No passive diagnostic-overview provider is composed by this image.");
    }
    let capture = board.digital_capture();
    if capture.is_implemented() {
        ui.label(format!(
            "digital capture V{}: {} / {} channels; {} transitions; {} / {} / {} B configure/record/chunk; {} / {} / {} µs pretrigger/duration/arm horizon",
            capture.schema_version,
            capture.resource_count,
            capture.maximum_channels,
            capture.maximum_transitions,
            capture.configure_bytes,
            capture.record_bytes,
            capture.maximum_chunk_bytes,
            capture.maximum_pretrigger_micros,
            capture.maximum_duration_micros,
            capture.arm_horizon_micros,
        ));
    } else {
        ui.label("No device-produced digital-capture provider is composed by this image.");
    }
    ui.label(format!(
        "{} licensed visuals / {} reviewed hotspots; {} HIL requirements; armable claim: {}",
        board.visuals().len(),
        hotspot_count,
        board.hil_requirement_count(),
        board.armable()
    ));
    ui.small(
        "These immutable facts label later diagnostic selection; they grant no resource lease, output command, arm transition, or physical-safety claim.",
    );
}

#[cfg(target_arch = "wasm32")]
fn default_live_waveform_request(
    snapshot: &DeviceSessionSnapshot,
    capability: Option<&ConnectedCapabilityView>,
) -> Option<WorkerWaveformRequest> {
    let capability = capability?;
    let identity = snapshot.device_identity.as_ref()?;
    let latest = snapshot.history.last()?;
    if snapshot.boot_id.is_none()
        || identity.board_id != capability.board().board_id()
        || identity.capability.identity().ok()? != capability.board().identity()
    {
        return None;
    }
    let capture = capability.board().digital_capture();
    if !capture.is_implemented() {
        return None;
    }
    let mut resources = capability
        .board()
        .resources()
        .iter()
        .filter(|resource| resource.is_digitally_capturable())
        .map(|resource| resource.descriptor().id)
        .collect::<Vec<_>>();
    resources.sort_unstable();
    resources.truncate(usize::from(capture.maximum_channels));
    if resources.is_empty() {
        return None;
    }
    let preferred_duration_cycles = latest.frequency_hz.saturating_add(499) / 500;
    let maximum_duration_cycles = u64::try_from(
        u128::from(latest.frequency_hz) * u128::from(capture.maximum_duration_micros) / 1_000_000,
    )
    .ok()?;
    if maximum_duration_cycles == 0 {
        return None;
    }
    let duration_cycles = preferred_duration_cycles.min(maximum_duration_cycles);
    Some(WorkerWaveformRequest {
        connection_id: snapshot.connection_id,
        channels: resources.into_iter().map(encode_resource_id).collect(),
        duration_cycles: duration_cycles.max(1),
    })
}

#[cfg(target_arch = "wasm32")]
fn show_live_telemetry_status(
    ui: &mut egui::Ui,
    snapshot: &DeviceSessionSnapshot,
    capability: Option<&ConnectedCapabilityView>,
    telemetry: Option<&ConnectedTelemetryView>,
) {
    ui.separator();
    ui.strong("Live capability-bound input status");
    match snapshot.telemetry_phase {
        None => {
            ui.label("No telemetry subscription is available for this session yet.");
        }
        Some(phase) => {
            ui.label(format!("telemetry subscription: {phase:?}"));
            if let (Some(subscription_id), Some(digest)) = (
                snapshot.telemetry_subscription_id,
                snapshot.telemetry_subscription_digest,
            ) {
                ui.monospace(format!(
                    "{subscription_id:016x} · {}…",
                    encode_hex(&digest[..8])
                ));
            }
            ui.label(format!(
                "newest exact event: {}; device-side latest-only replacements: {}",
                snapshot.telemetry_event_sequence, snapshot.telemetry_dropped_events
            ));
        }
    }
    if snapshot.telemetry_consecutive_failures != 0 {
        ui.label(format!(
            "consecutive telemetry failures: {}",
            snapshot.telemetry_consecutive_failures
        ));
    }
    if let Some(error) = &snapshot.telemetry_last_error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
    }
    match (
        capability,
        telemetry.and_then(ConnectedTelemetryView::latest),
    ) {
        (Some(capability), Some(event)) => {
            let overview = event.overview();
            let simulated = overview.flags().contains(OverviewFlags::SIMULATED);
            ui.colored_label(
                if simulated {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::LIGHT_GREEN
                },
                if simulated {
                    "SIMULATED sampled input evidence — no physical measurement claim"
                } else {
                    "Canonical device-sampled input evidence"
                },
            );
            ui.label(format!(
                "event {} · overview {} · snapshot cycle {} · {} retained UI samples",
                event.event_sequence(),
                overview.sequence(),
                overview.snapshot_cycle().0,
                telemetry
                    .expect("latest telemetry came from this view")
                    .event_count()
            ));
            for sample in overview.samples() {
                let age = overview
                    .snapshot_cycle()
                    .0
                    .saturating_sub(sample.captured_cycle.0);
                ui.colored_label(
                    live_telemetry_quality_color(sample.quality),
                    format!(
                        "{}: {} · {:?} / {:?} · captured {} · age {} cycles",
                        live_resource_label(capability.board(), sample.resource),
                        live_resource_value(sample.value),
                        sample.provenance,
                        sample.quality,
                        sample.captured_cycle.0,
                        age
                    ),
                );
            }
            show_live_telemetry_plot(
                ui,
                capability.board(),
                telemetry.expect("latest telemetry came from this view"),
            );
        }
        (_, None) if snapshot.telemetry_phase.is_some() => {
            ui.label("The subscription is awaiting its first complete exact overview event.");
        }
        _ => {}
    }
    ui.small(
        "These low-rate, latest-only samples are passive input observations. Labels resolve through the same typed capability resources used by any reviewed board-image hotspots; this path cannot write or lease an I/O.",
    );
}

#[cfg(target_arch = "wasm32")]
fn live_resource_value(value: ResourceValue) -> String {
    match value {
        ResourceValue::Unavailable => "unavailable".to_owned(),
        ResourceValue::Boolean(value) => if value { "HIGH" } else { "LOW" }.to_owned(),
        ResourceValue::Unsigned(value) => value.to_string(),
        ResourceValue::Signed(value) => value.to_string(),
        ResourceValue::ExactRatio {
            numerator,
            denominator,
        } => format!("{numerator}/{denominator}"),
    }
}

#[cfg(target_arch = "wasm32")]
fn live_telemetry_quality_color(quality: SampleQuality) -> egui::Color32 {
    match quality {
        SampleQuality::Valid => egui::Color32::LIGHT_GREEN,
        SampleQuality::Stale => egui::Color32::YELLOW,
        SampleQuality::Unavailable | SampleQuality::Faulted => egui::Color32::LIGHT_RED,
    }
}

#[cfg(target_arch = "wasm32")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "bounded sampled-history projection is lossy while retained cycles remain exact"
)]
fn show_live_telemetry_plot(
    ui: &mut egui::Ui,
    board: &alumina_interface_core::BoardExplorerSnapshot,
    telemetry: &ConnectedTelemetryView,
) {
    let Some(latest) = telemetry.latest() else {
        return;
    };
    let resources = latest
        .overview()
        .samples()
        .map(|sample| sample.resource)
        .collect::<Vec<_>>();
    if resources.is_empty() {
        return;
    }
    let cycles = telemetry
        .events()
        .map(|event| event.overview().snapshot_cycle().0)
        .collect::<Vec<_>>();
    let Some(start_cycle) = cycles.first().copied() else {
        return;
    };
    let end_cycle = cycles
        .last()
        .copied()
        .unwrap_or(start_cycle)
        .max(start_cycle.saturating_add(1));
    let duration = end_cycle.saturating_sub(start_cycle).max(1);
    let width = ui.available_width().max(360.0);
    let height = 34.0 + 36.0 * resources.len() as f32;
    let (response, painter) = ui.allocate_painter(egui::vec2(width, height), egui::Sense::hover());
    let plot = egui::Rect::from_min_max(
        response.rect.min + egui::vec2(170.0, 8.0),
        response.rect.max - egui::vec2(12.0, 22.0),
    );
    painter.rect_filled(plot, 3.0, egui::Color32::from_rgb(17, 21, 29));
    let lane_height = plot.height() / resources.len() as f32;
    for (index, resource) in resources.into_iter().enumerate() {
        let center = plot.top() + lane_height * (index as f32 + 0.5);
        let color = live_capture_color(index);
        painter.text(
            egui::pos2(plot.left() - 8.0, center),
            egui::Align2::RIGHT_CENTER,
            live_resource_label(board, resource),
            egui::FontId::monospace(10.0),
            color,
        );
        let mut points = telemetry.events().filter_map(|event| {
            let overview = event.overview();
            let sample = overview.sample(resource)?;
            let ResourceValue::Boolean(level) = sample.value else {
                return None;
            };
            Some((overview.snapshot_cycle().0, level))
        });
        let Some((mut prior_cycle, mut prior_level)) = points.next() else {
            continue;
        };
        for (cycle, level) in points {
            let from_x = live_telemetry_x(plot, prior_cycle, start_cycle, duration);
            let to_x = live_telemetry_x(plot, cycle, start_cycle, duration);
            let from_y = live_boolean_y(center, prior_level);
            let to_y = live_boolean_y(center, level);
            painter.line_segment(
                [egui::pos2(from_x, from_y), egui::pos2(to_x, from_y)],
                egui::Stroke::new(1.8_f32, color),
            );
            painter.line_segment(
                [egui::pos2(to_x, from_y), egui::pos2(to_x, to_y)],
                egui::Stroke::new(1.8_f32, color),
            );
            prior_cycle = cycle;
            prior_level = level;
        }
        painter.line_segment(
            [
                egui::pos2(
                    live_telemetry_x(plot, prior_cycle, start_cycle, duration),
                    live_boolean_y(center, prior_level),
                ),
                egui::pos2(plot.right(), live_boolean_y(center, prior_level)),
            ],
            egui::Stroke::new(1.8_f32, color),
        );
    }
    painter.text(
        egui::pos2(plot.left(), plot.bottom() + 3.0),
        egui::Align2::LEFT_TOP,
        start_cycle.to_string(),
        egui::FontId::monospace(9.0),
        egui::Color32::GRAY,
    );
    painter.text(
        egui::pos2(plot.right(), plot.bottom() + 3.0),
        egui::Align2::RIGHT_TOP,
        end_cycle.to_string(),
        egui::FontId::monospace(9.0),
        egui::Color32::GRAY,
    );
}

#[cfg(target_arch = "wasm32")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "bounded sampled-history projection is lossy"
)]
fn live_telemetry_x(rect: egui::Rect, cycle: u64, start: u64, duration: u64) -> f32 {
    rect.left()
        + rect.width() * (cycle.saturating_sub(start).min(duration) as f64 / duration as f64) as f32
}

#[cfg(target_arch = "wasm32")]
const fn live_boolean_y(center: f32, level: bool) -> f32 {
    if level { center - 7.0 } else { center + 7.0 }
}

#[cfg(target_arch = "wasm32")]
fn show_live_waveform_status(
    ui: &mut egui::Ui,
    snapshot: &DeviceSessionSnapshot,
    capability: Option<&ConnectedCapabilityView>,
    waveform: Option<&ConnectedWaveformView>,
    capture_cursor: &mut u64,
) {
    ui.separator();
    ui.strong("Capability-bound digital logic capture");
    match snapshot.waveform_phase {
        None => {
            ui.label("No input capture has been requested in this worker generation.");
        }
        Some(phase) => {
            ui.label(format!("diagnostic acquisition: {phase:?}"));
            if phase == WaveformCapturePhaseSnapshot::Downloading {
                ui.label(format!(
                    "canonical range assembly: {} / {} bytes",
                    snapshot.waveform_received_bytes, snapshot.waveform_total_bytes
                ));
            }
            if phase == WaveformCapturePhaseSnapshot::Complete && waveform.is_none() {
                ui.label("Complete worker evidence is awaiting rendering-realm admission.");
            }
        }
    }
    if snapshot.waveform_consecutive_failures != 0 {
        ui.label(format!(
            "consecutive waveform failures: {}",
            snapshot.waveform_consecutive_failures
        ));
    }
    if let Some(error) = &snapshot.waveform_last_error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
    }
    if let (Some(capability), Some(waveform)) = (capability, waveform) {
        show_live_waveform_plot(ui, capability.board(), waveform, capture_cursor);
    }
    ui.small(
        "This path configures and arms only an input diagnostic acquisition. It cannot lease resources, write pins, schedule motion, energize outputs, or arm the machine.",
    );
}

#[cfg(target_arch = "wasm32")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    reason = "screen projection is lossy while retained device cycles remain exact integers"
)]
fn show_live_waveform_plot(
    ui: &mut egui::Ui,
    board: &alumina_interface_core::BoardExplorerSnapshot,
    waveform: &ConnectedWaveformView,
    capture_cursor: &mut u64,
) {
    let capture = waveform.capture();
    let context = capture.context();
    let simulated = capture.flags().contains(DigitalCaptureFlags::SIMULATED);
    ui.colored_label(
        if simulated {
            egui::Color32::YELLOW
        } else {
            egui::Color32::LIGHT_GREEN
        },
        if simulated {
            "SIMULATED canonical input evidence — no physical measurement claim"
        } else {
            "Canonical device input evidence"
        },
    );
    let (start, end) = capture.cycle_window();
    let duration = end.0.saturating_sub(start.0).max(1);
    let (trigger_cycle, _, trigger, _) = capture.trigger();
    ui.horizontal_wrapped(|ui| {
        ui.monospace(format!(
            "ALMDIG01 {} B · {} Hz · [{}..{})",
            waveform.record_bytes(),
            context.clock_frequency_hz,
            start.0,
            end.0
        ));
        ui.label(format!(
            "{:?}; {} channels; {} transitions; trigger {:?} at {}",
            capture.state(),
            capture.channel_count(),
            capture.transition_count(),
            trigger,
            trigger_cycle.0
        ));
    });

    let channel_count = capture.channel_count();
    if channel_count == 0 {
        return;
    }
    let width = ui.available_width().max(360.0);
    let height = 45.0 + 40.0 * channel_count as f32;
    let (response, painter) = ui.allocate_painter(egui::vec2(width, height), egui::Sense::hover());
    let plot = egui::Rect::from_min_max(
        response.rect.min + egui::vec2(170.0, 10.0),
        response.rect.max - egui::vec2(12.0, 28.0),
    );
    painter.rect_filled(plot, 3.0, egui::Color32::from_rgb(17, 21, 29));
    if let Some(pointer) = response.hover_pos().filter(|point| plot.contains(*point)) {
        let ratio = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        *capture_cursor = (f64::from(ratio) * duration as f64).floor() as u64;
    }
    *capture_cursor = (*capture_cursor).min(duration.saturating_sub(1));

    for grid in 0..=4_u64 {
        let offset = duration.saturating_mul(grid) / 4;
        let x = live_capture_x(plot, offset, duration);
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(0.7_f32, egui::Color32::from_gray(49)),
        );
        painter.text(
            egui::pos2(x, plot.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            offset.to_string(),
            egui::FontId::monospace(9.0),
            egui::Color32::GRAY,
        );
    }
    let cursor_x = live_capture_x(plot, *capture_cursor, duration);
    painter.line_segment(
        [
            egui::pos2(cursor_x, plot.top()),
            egui::pos2(cursor_x, plot.bottom()),
        ],
        egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(150)),
    );

    let lane_height = plot.height() / channel_count as f32;
    for (index, channel) in capture.channels().enumerate() {
        let lane_top = plot.top() + lane_height * index as f32;
        let center = lane_top + lane_height * 0.5;
        let color = live_capture_color(index);
        painter.text(
            egui::pos2(plot.left() - 8.0, center),
            egui::Align2::RIGHT_CENTER,
            live_resource_label(board, channel.resource),
            egui::FontId::monospace(10.0),
            color,
        );
        let channel_index = u16::try_from(index).expect("canonical channel index fits u16");
        let mut level = channel.initial_level;
        let mut prior_offset = 0_u64;
        for transition in capture
            .transitions()
            .filter(|transition| transition.channel_index == channel_index)
        {
            let from_x = live_capture_x(plot, prior_offset, duration);
            let to_x = live_capture_x(plot, transition.offset_cycles, duration);
            let from_y = live_level_y(center, level);
            let to_y = live_level_y(center, transition.level);
            painter.line_segment(
                [egui::pos2(from_x, from_y), egui::pos2(to_x, from_y)],
                egui::Stroke::new(1.8_f32, color),
            );
            painter.line_segment(
                [egui::pos2(to_x, from_y), egui::pos2(to_x, to_y)],
                egui::Stroke::new(1.8_f32, color),
            );
            prior_offset = transition.offset_cycles;
            level = transition.level;
        }
        painter.line_segment(
            [
                egui::pos2(
                    live_capture_x(plot, prior_offset, duration),
                    live_level_y(center, level),
                ),
                egui::pos2(plot.right(), live_level_y(center, level)),
            ],
            egui::Stroke::new(1.8_f32, color),
        );
        let cursor_level = live_capture_level_at(capture, channel_index, *capture_cursor);
        painter.circle_filled(
            egui::pos2(cursor_x, live_level_y(center, cursor_level)),
            3.0,
            color,
        );
    }
    ui.horizontal_wrapped(|ui| {
        ui.strong(format!(
            "cursor +{} cycles · absolute {}",
            *capture_cursor,
            start.0.saturating_add(*capture_cursor)
        ));
        for (index, channel) in capture.channels().enumerate() {
            let level = live_capture_level_at(
                capture,
                u16::try_from(index).expect("canonical channel index fits u16"),
                *capture_cursor,
            );
            ui.colored_label(
                live_capture_color(index),
                format!(
                    "{} {}",
                    live_resource_label(board, channel.resource),
                    match level {
                        DigitalLevel::Low => "LOW",
                        DigitalLevel::High => "HIGH",
                        DigitalLevel::Unknown => "UNKNOWN",
                    }
                ),
            );
        }
    });
    ui.small(
        "Hover the trace for an exact device-cycle cursor. Channel labels are resolved from the same capability resources and annotated-image hotspots shown by the board package.",
    );
}

#[cfg(target_arch = "wasm32")]
fn live_capture_level_at(
    capture: alumina_diagnostics::DigitalCaptureView<'_>,
    channel_index: u16,
    offset: u64,
) -> DigitalLevel {
    let mut level = capture
        .channel(channel_index)
        .expect("canonical channel index is present")
        .initial_level;
    for transition in capture.transitions() {
        if transition.offset_cycles > offset {
            break;
        }
        if transition.channel_index == channel_index {
            level = transition.level;
        }
    }
    level
}

#[cfg(target_arch = "wasm32")]
fn live_resource_label(
    board: &alumina_interface_core::BoardExplorerSnapshot,
    resource: alumina_board::ResourceId,
) -> String {
    board.resource(resource).map_or_else(
        || format!("{resource:?}"),
        |entry| {
            entry
                .aliases()
                .first()
                .cloned()
                .unwrap_or_else(|| format!("{resource:?}"))
        },
    )
}

#[cfg(target_arch = "wasm32")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "bounded screen projection"
)]
fn live_capture_x(rect: egui::Rect, offset: u64, duration: u64) -> f32 {
    rect.left() + rect.width() * (offset.min(duration) as f64 / duration.max(1) as f64) as f32
}

#[cfg(target_arch = "wasm32")]
const fn live_level_y(center: f32, level: DigitalLevel) -> f32 {
    match level {
        DigitalLevel::High => center - 8.0,
        DigitalLevel::Low => center + 8.0,
        DigitalLevel::Unknown => center,
    }
}

#[cfg(target_arch = "wasm32")]
fn live_capture_color(index: usize) -> egui::Color32 {
    const COLORS: [egui::Color32; 6] = [
        egui::Color32::from_rgb(103, 193, 232),
        egui::Color32::from_rgb(248, 183, 82),
        egui::Color32::from_rgb(126, 211, 133),
        egui::Color32::from_rgb(211, 132, 226),
        egui::Color32::from_rgb(239, 111, 108),
        egui::Color32::from_rgb(123, 216, 204),
    ];
    COLORS[index % COLORS.len()]
}

#[cfg(target_arch = "wasm32")]
fn show_live_configuration_snapshot(ui: &mut egui::Ui, snapshot: &DeviceSessionSnapshot) {
    ui.separator();
    ui.strong("Authenticated active configuration");
    match (snapshot.configuration_availability, snapshot.configuration) {
        (ConfigurationStatusAvailabilitySnapshot::Unobserved, None) => {
            ui.label("No canonical configuration status has been accepted yet.");
        }
        (ConfigurationStatusAvailabilitySnapshot::Unsupported, None) => {
            ui.colored_label(
                egui::Color32::YELLOW,
                "This image does not expose the active-configuration coordinator.",
            );
        }
        (ConfigurationStatusAvailabilitySnapshot::Available, Some(configuration)) => {
            ui.label(format!(
                "phase: {:?} · jobs authorized: {} · transaction {}",
                configuration.phase,
                configuration.jobs_authorized,
                configuration.active_transaction_id
            ));
            if configuration.active_bytes != 0 {
                ui.monospace(format!(
                    "ALMCFG {}… / {} bytes",
                    encode_hex(&configuration.active_digest[..8]),
                    configuration.active_bytes
                ));
            } else {
                ui.label("No durable active machine configuration.");
            }
            if let Some(summary) = configuration.summary {
                ui.label(format!(
                    "{} records ({} RT) · {} bindings · {} stepper / {} FOC axes · safety binding: {}",
                    summary.record_count,
                    summary.realtime_record_count,
                    summary.binding_count,
                    summary.stepper_axes,
                    summary.foc_axes,
                    summary.safety_binding
                ));
                ui.monospace(format!("configuration flags: 0x{:08x}", summary.flags));
            }
        }
        _ => {
            ui.colored_label(
                egui::Color32::RED,
                "Worker configuration snapshot failed its state relationship.",
            );
        }
    }
    if let Some(error) = &snapshot.configuration_last_error {
        ui.colored_label(
            egui::Color32::LIGHT_RED,
            format!(
                "configuration status failures: {} · {error}",
                snapshot.configuration_consecutive_failures
            ),
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn show_runtime_health_snapshot(ui: &mut egui::Ui, snapshot: &DeviceSessionSnapshot) {
    ui.separator();
    ui.strong("Passive runtime diagnostics");
    match (
        snapshot.runtime_health_availability,
        snapshot.runtime_health,
    ) {
        (RuntimeHealthAvailabilitySnapshot::Unobserved, None) => {
            ui.label("No authenticated runtime-health result in this session yet.");
        }
        (RuntimeHealthAvailabilitySnapshot::Unsupported, None) => {
            ui.label("This firmware explicitly reports runtime-health as unsupported.");
        }
        (RuntimeHealthAvailabilitySnapshot::Available, Some(health)) => {
            ui.label(format!(
                "latest valid response cycle: {}",
                health.snapshot_cycle
            ));
            for (label, queue) in [
                ("command", health.command_queue),
                ("work block", health.work_queue),
                ("telemetry", health.telemetry_queue),
            ] {
                ui.label(format!(
                    "{label} queue: {} / {} occupied; {} free",
                    queue.depth,
                    queue.capacity,
                    queue.free()
                ));
            }
            show_executor_stack_snapshot(
                ui,
                "service-core stack",
                health.service_stack,
                health.snapshot_cycle,
            );
            match health.realtime_stack {
                Some(realtime) => {
                    show_executor_stack_snapshot(
                        ui,
                        "real-time-core stack",
                        realtime,
                        health.snapshot_cycle,
                    );
                    if health.realtime_stack_fresh {
                        ui.label("real-time stack witness: present and firmware-fresh");
                    } else {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "real-time stack witness: present but stale",
                        );
                    }
                }
                None => {
                    ui.label("real-time stack witness: not observed in this boot");
                }
            }
            ui.small(
                "Stack watermarks converge incrementally from a partial boot epoch; they are diagnostic evidence, not a transient-depth, sizing, or safety proof.",
            );
        }
        _ => {
            ui.colored_label(
                egui::Color32::RED,
                "Worker runtime-health state was internally inconsistent.",
            );
        }
    }
    if snapshot.runtime_health_consecutive_failures != 0 {
        ui.label(format!(
            "consecutive runtime-health failures: {}",
            snapshot.runtime_health_consecutive_failures
        ));
    }
    if let Some(error) = &snapshot.runtime_health_last_error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
        if snapshot.runtime_health.is_some() {
            ui.small("The last valid runtime-health snapshot remains visible above.");
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn show_executor_stack_snapshot(
    ui: &mut egui::Ui,
    label: &str,
    stack: ExecutorStackSnapshot,
    response_cycle: u64,
) {
    ui.label(format!(
        "{label}: maximum observed use {} / {} monitored bytes; minimum headroom {} bytes",
        stack.observed_maximum_used_bytes(),
        stack.monitored_bytes(),
        stack.minimum_headroom_bytes
    ));
    ui.label(format!(
        "allocation {} bytes; low exclusion {}; painted {}; unpainted reserve {}",
        stack.allocated_bytes,
        stack.excluded_low_bytes,
        stack.painted_bytes,
        stack.unpainted_bytes()
    ));
    ui.label(format!(
        "samples {}; completed sweeps {}; sample age {} cycles; epoch/sample cycles {}/{}",
        stack.samples,
        stack.completed_sweeps,
        stack.sample_age_cycles(response_cycle),
        stack.epoch_cycle,
        stack.sampled_at
    ));
    ui.label(format!(
        "partial boot epoch: {}; completed sweep observed: {}; raw flags: 0x{:04x}",
        stack.is_partial_boot_epoch(),
        stack.has_completed_sweep(),
        stack.flags
    ));
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

#[cfg(target_arch = "wasm32")]
fn browser_local_storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or_else(|| "browser window is unavailable".to_owned())?
        .local_storage()
        .map_err(|value| browser_value_text(&value))?
        .ok_or_else(|| "browser local storage is unavailable".to_owned())
}

#[cfg(target_arch = "wasm32")]
fn load_persisted_exact_control_workspace() -> Result<Option<String>, String> {
    browser_local_storage()?
        .get_item(WORKSPACE_STORAGE_KEY)
        .map_err(|value| browser_value_text(&value))
}

#[cfg(target_arch = "wasm32")]
fn browser_value_text(value: &wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser API rejected: {value:?}"))
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

        egui::CentralPanel::default().show(context, |ui| match self.workspace_view {
            WorkspaceView::MachineCam => match self.machine_cam.as_mut() {
                Some(workspace) => {
                    egui::ScrollArea::vertical()
                        .id_salt("machine_cam_workspace_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| workspace.show(ui));
                }
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.colored_label(
                            egui::Color32::RED,
                            self.machine_cam_error
                                .as_deref()
                                .unwrap_or("machine/CAM workspace is unavailable"),
                        );
                    });
                }
            },
            WorkspaceView::Geometry => self.show_geometry_workspace(ui),
            WorkspaceView::ExactControl => match self.exact_control.as_mut() {
                Some(workspace) => {
                    egui::ScrollArea::vertical()
                        .id_salt("exact_control_workspace_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| workspace.show(ui));
                }
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.colored_label(
                            egui::Color32::RED,
                            self.exact_control_error
                                .as_deref()
                                .unwrap_or("exact control workspace is unavailable"),
                        );
                    });
                }
            },
        });

        #[cfg(target_arch = "wasm32")]
        self.persist_exact_control_workspace();
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
