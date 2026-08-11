//! Dedicated browser control worker and rendering-realm supervisor.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use alumina_clock::{ClockFlags, ClockObservation};
use alumina_interface_client::clock::{ClockProbeError, DeviceClockModel};
use alumina_interface_client::http::AuthenticatedHttpSession;
use alumina_interface_client::wasm::{
    BrowserClockError, BrowserFetchError, DeviceOrigin, drive_clock_probe_in_worker,
    open_authenticated_session_in_worker, worker_origin,
};
use alumina_interface_client::worker::{
    ClockEstimateSnapshot, ClockHistoryRecord, DeviceConnectionRequest, DeviceSessionPhase,
    DeviceSessionSnapshot, MAXIMUM_CLOCK_HISTORY, WORKER_SCHEMA_VERSION, WorkerCommand,
    WorkerCommandEnvelope, WorkerEvent, WorkerEventEnvelope,
};
use alumina_protocol::Digest;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    DedicatedWorkerGlobalScope, ErrorEvent, MessageEvent, Worker, WorkerGlobalScope, WorkerOptions,
    WorkerType,
};

const CONTROL_WORKER_URL: &str = "alumina-worker.js";
const WORKER_TICK_MS: i32 = 100;
const MAXIMUM_COMMAND_JSON_BYTES: usize = 8 * 1024;
const MAXIMUM_UI_DIAGNOSTICS: usize = 16;
const MAXIMUM_RETRY_MS: u32 = 30_000;

struct DeviceState {
    connection_id: u64,
    label: String,
    origin_text: String,
    origin: DeviceOrigin,
    secret: Vec<u8>,
    sampling: alumina_interface_client::worker::ClockSamplingPolicy,
    generation: u64,
    session: Option<AuthenticatedHttpSession>,
    clock: DeviceClockModel,
    history: VecDeque<ClockHistoryRecord>,
    phase: DeviceSessionPhase,
    consecutive_failures: u32,
    estimate: Option<ClockEstimateSnapshot>,
    last_error: Option<String>,
    next_attempt_ms: f64,
}

impl DeviceState {
    fn from_request(
        mut request: DeviceConnectionRequest,
        origin: DeviceOrigin,
        generation: u64,
    ) -> Result<Self, String> {
        let estimator = request
            .sampling
            .estimator()
            .map_err(|error| error.to_string())?;
        let clock = DeviceClockModel::new(estimator).map_err(|error| error.to_string())?;
        Ok(Self {
            connection_id: request.connection_id,
            label: std::mem::take(&mut request.label),
            origin_text: std::mem::take(&mut request.origin),
            origin,
            secret: std::mem::take(&mut request.secret),
            sampling: request.sampling,
            generation,
            session: None,
            clock,
            history: VecDeque::with_capacity(MAXIMUM_CLOCK_HISTORY),
            phase: DeviceSessionPhase::Connecting,
            consecutive_failures: 0,
            estimate: None,
            last_error: None,
            next_attempt_ms: 0.0,
        })
    }

    fn snapshot(&self) -> DeviceSessionSnapshot {
        DeviceSessionSnapshot {
            connection_id: self.connection_id,
            label: self.label.clone(),
            origin: self.origin_text.clone(),
            generation: self.generation,
            phase: self.phase,
            boot_id: self.clock.boot_id().map(alumina_clock::BootId::as_bytes),
            accepted_samples: self.clock.accepted_samples(),
            rejected_samples: self.clock.rejected_samples(),
            consecutive_failures: self.consecutive_failures,
            estimate: self.estimate,
            history: self.history.iter().copied().collect(),
            last_error: self.last_error.clone(),
        }
    }

    fn schedule_success(&mut self, now_ms: f64) {
        self.consecutive_failures = 0;
        self.last_error = None;
        self.next_attempt_ms = now_ms + f64::from(self.sampling.heartbeat_interval_ms);
    }

    fn schedule_failure(&mut self, now_ms: f64, error: String) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.phase = DeviceSessionPhase::RetryWaiting;
        self.estimate = None;
        self.last_error = Some(error);
        let exponent = self.consecutive_failures.saturating_sub(1).min(5);
        let multiplier = 1_u32 << exponent;
        let retry_ms = self
            .sampling
            .heartbeat_interval_ms
            .saturating_mul(multiplier)
            .min(MAXIMUM_RETRY_MS);
        self.next_attempt_ms = now_ms + f64::from(retry_ms);
    }

    fn record_observation(&mut self, observation: ClockObservation) {
        let response = observation.response;
        let record = ClockHistoryRecord {
            probe_id: response.probe_id,
            ui_send_ns: response.ui_send_ns,
            ui_receive_ns: observation.ui_receive_ns,
            causal_span_ns: observation
                .ui_receive_ns
                .saturating_sub(response.ui_send_ns),
            receive_cycle: response.receive_cycle.0,
            transmit_cycle: response.transmit_cycle.0,
            processing_cycles: response
                .transmit_cycle
                .0
                .saturating_sub(response.receive_cycle.0),
            frequency_hz: response.frequency_hz,
            flags: response.flags.0,
            missed_deadlines: response.missed_deadlines,
            command_queue_free: response.command_queue_free,
            work_queue_depth: response.work_queue_depth,
        };
        if self.history.len() == MAXIMUM_CLOCK_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(record);
        self.estimate = self
            .clock
            .estimate_at(
                observation.ui_receive_ns,
                self.sampling.maximum_uncertainty_cycles,
            )
            .ok()
            .map(|estimate| ClockEstimateSnapshot {
                ui_ns: estimate.ui_ns,
                earliest_cycle: estimate.earliest_cycle.0,
                midpoint_cycle: estimate.midpoint_cycle.0,
                latest_cycle: estimate.latest_cycle.0,
                uncertainty_cycles: estimate.uncertainty_cycles,
            });
        let healthy = response.flags.contains(ClockFlags::DEADLINE_HEALTHY)
            && !response.flags.contains(ClockFlags::SAFETY_UNHEALTHY);
        self.phase = if !healthy {
            DeviceSessionPhase::DeviceUnhealthy
        } else if self.estimate.is_some() {
            DeviceSessionPhase::ClockQualified
        } else {
            DeviceSessionPhase::Sampling
        };
    }
}

impl Drop for DeviceState {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

enum DeviceEntry {
    Idle(Box<DeviceState>),
    Busy {
        generation: u64,
        snapshot: DeviceSessionSnapshot,
    },
}

impl DeviceEntry {
    fn snapshot(&self) -> DeviceSessionSnapshot {
        match self {
            Self::Idle(state) => state.snapshot(),
            Self::Busy { snapshot, .. } => snapshot.clone(),
        }
    }
}

struct ControlWorkerRuntime {
    scope: DedicatedWorkerGlobalScope,
    devices: BTreeMap<u64, DeviceEntry>,
    next_generation: u64,
}

impl ControlWorkerRuntime {
    fn new(scope: DedicatedWorkerGlobalScope) -> Self {
        Self {
            scope,
            devices: BTreeMap::new(),
            next_generation: 1,
        }
    }

    fn allocate_generation(&mut self) -> Option<u64> {
        let generation = self.next_generation;
        self.next_generation = generation.checked_add(1)?;
        Some(generation)
    }
}

type SharedWorkerRuntime = Rc<RefCell<ControlWorkerRuntime>>;

/// Installs the dedicated worker message loop and automatic heartbeat timer.
///
/// # Errors
///
/// Returns a JavaScript error when the inherited origin or worker timer cannot
/// be installed. Per-device failures are emitted as redacted worker events.
pub fn install_control_worker(scope: &DedicatedWorkerGlobalScope) -> Result<(), JsValue> {
    let worker_scope: &WorkerGlobalScope = scope.as_ref();
    let scope_origin = worker_origin(worker_scope)
        .map_err(|error| JsValue::from_str(&error.to_string()))?
        .as_str()
        .to_owned();
    let runtime = Rc::new(RefCell::new(ControlWorkerRuntime::new(scope.clone())));

    let message_runtime = Rc::clone(&runtime);
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        receive_worker_command(&message_runtime, &event);
    });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let tick_runtime = Rc::clone(&runtime);
    let on_tick = Closure::<dyn FnMut()>::new(move || launch_due_devices(&tick_runtime));
    scope.set_interval_with_callback_and_timeout_and_arguments_0(
        on_tick.as_ref().unchecked_ref(),
        WORKER_TICK_MS,
    )?;
    on_tick.forget();

    emit_worker_event(&runtime, WorkerEvent::Ready { scope_origin });
    Ok(())
}

fn receive_worker_command(runtime: &SharedWorkerRuntime, event: &MessageEvent) {
    let Some(json) = event.data().as_string() else {
        reject_command(runtime, None, "worker command must be a JSON string");
        return;
    };
    if json.len() > MAXIMUM_COMMAND_JSON_BYTES {
        reject_command(
            runtime,
            None,
            "worker command exceeds its bounded JSON allowance",
        );
        return;
    }
    let envelope: WorkerCommandEnvelope = if let Ok(envelope) = serde_json::from_str(&json) {
        envelope
    } else {
        reject_command(runtime, None, "worker command JSON is not canonical");
        return;
    };
    let connection_id = command_connection_id(&envelope.command);
    if let Err(error) = envelope.validate_version() {
        reject_command(runtime, Some(connection_id), &error.to_string());
        return;
    }
    match envelope.command {
        WorkerCommand::Configure { request } => configure_device(runtime, request),
        WorkerCommand::ProbeNow { connection_id } => {
            if connection_id == 0 {
                reject_command(
                    runtime,
                    Some(connection_id),
                    "connection identity must be nonzero",
                );
            } else {
                launch_device_step(runtime, connection_id, true);
            }
        }
        WorkerCommand::Disconnect { connection_id } => disconnect_device(runtime, connection_id),
    }
}

const fn command_connection_id(command: &WorkerCommand) -> u64 {
    match command {
        WorkerCommand::Configure { request } => request.connection_id,
        WorkerCommand::ProbeNow { connection_id } | WorkerCommand::Disconnect { connection_id } => {
            *connection_id
        }
    }
}

fn configure_device(runtime: &SharedWorkerRuntime, request: DeviceConnectionRequest) {
    let connection_id = request.connection_id;
    if let Err(error) = request.validate() {
        reject_command(runtime, Some(connection_id), &error.to_string());
        return;
    }
    let origin = match DeviceOrigin::parse(&request.origin) {
        Ok(origin) => origin,
        Err(error) => {
            reject_command(runtime, Some(connection_id), &error.to_string());
            return;
        }
    };
    let generation = { runtime.borrow_mut().allocate_generation() };
    let Some(generation) = generation else {
        emit_worker_event(
            runtime,
            WorkerEvent::Fatal {
                message: "worker session generation is exhausted".to_owned(),
            },
        );
        return;
    };
    let state = match DeviceState::from_request(request, origin, generation) {
        Ok(state) => state,
        Err(error) => {
            reject_command(runtime, Some(connection_id), &error);
            return;
        }
    };
    runtime
        .borrow_mut()
        .devices
        .insert(connection_id, DeviceEntry::Idle(Box::new(state)));
    publish_snapshot(runtime, connection_id);
    launch_device_step(runtime, connection_id, false);
}

fn disconnect_device(runtime: &SharedWorkerRuntime, connection_id: u64) {
    let removed = runtime
        .borrow_mut()
        .devices
        .remove(&connection_id)
        .is_some();
    if removed {
        emit_worker_event(runtime, WorkerEvent::Removed { connection_id });
    } else {
        reject_command(runtime, Some(connection_id), "connection does not exist");
    }
}

fn launch_due_devices(runtime: &SharedWorkerRuntime) {
    let (now_ms, due): (f64, Vec<u64>) = {
        let runtime_ref = runtime.borrow();
        let now_ms = worker_now_ms(&runtime_ref.scope);
        let due = runtime_ref
            .devices
            .iter()
            .filter_map(|(connection_id, entry)| match entry {
                DeviceEntry::Idle(state) if state.next_attempt_ms <= now_ms => Some(*connection_id),
                _ => None,
            })
            .collect();
        (now_ms, due)
    };
    if !now_ms.is_finite() {
        return;
    }
    for connection_id in due {
        launch_device_step(runtime, connection_id, false);
    }
}

fn launch_device_step(runtime: &SharedWorkerRuntime, connection_id: u64, reject_busy: bool) {
    let Some(state) = take_idle_device(runtime, connection_id) else {
        if reject_busy {
            reject_command(
                runtime,
                Some(connection_id),
                "connection is busy or does not exist",
            );
        }
        return;
    };
    let generation = state.generation;
    let task_runtime = Rc::clone(runtime);
    if state.session.is_some() {
        spawn_local(async move {
            let state = probe_device(&task_runtime, state).await;
            finish_device_step(&task_runtime, connection_id, generation, state, false);
        });
    } else {
        spawn_local(async move {
            let state = connect_device(&task_runtime, state).await;
            let probe_immediately = state.session.is_some();
            finish_device_step(
                &task_runtime,
                connection_id,
                generation,
                state,
                probe_immediately,
            );
        });
    }
}

fn take_idle_device(runtime: &SharedWorkerRuntime, connection_id: u64) -> Option<DeviceState> {
    let mut runtime_ref = runtime.borrow_mut();
    let entry = runtime_ref.devices.remove(&connection_id)?;
    match entry {
        DeviceEntry::Idle(state) => {
            let generation = state.generation;
            let snapshot = state.snapshot();
            runtime_ref.devices.insert(
                connection_id,
                DeviceEntry::Busy {
                    generation,
                    snapshot,
                },
            );
            Some(*state)
        }
        busy @ DeviceEntry::Busy { .. } => {
            runtime_ref.devices.insert(connection_id, busy);
            None
        }
    }
}

async fn connect_device(runtime: &SharedWorkerRuntime, mut state: DeviceState) -> DeviceState {
    state.phase = DeviceSessionPhase::Connecting;
    let scope = runtime.borrow().scope.clone();
    let worker_scope: &WorkerGlobalScope = scope.as_ref();
    let result =
        open_authenticated_session_in_worker(worker_scope, &state.origin, Digest::ZERO).await;
    let now_ms = worker_now_ms(&scope);
    match result {
        Ok(session) => {
            state.session = Some(session);
            state.phase = DeviceSessionPhase::Sampling;
            state.schedule_success(now_ms);
            state.next_attempt_ms = now_ms;
        }
        Err(error) => state.schedule_failure(now_ms, error.to_string()),
    }
    state
}

async fn probe_device(runtime: &SharedWorkerRuntime, mut state: DeviceState) -> DeviceState {
    let Some(session) = state.session.as_mut() else {
        return state;
    };
    let scope = runtime.borrow().scope.clone();
    let worker_scope: &WorkerGlobalScope = scope.as_ref();
    let result = drive_clock_probe_in_worker(
        worker_scope,
        &state.origin,
        session,
        &mut state.clock,
        state.sampling.maximum_timer_error_ns,
        &state.secret,
    )
    .await;
    let now_ms = worker_now_ms(&scope);
    match result {
        Ok(observation) => {
            state.record_observation(observation);
            state.schedule_success(now_ms);
        }
        Err(error) => {
            if clock_model_must_reset(&error) {
                let _ = state.clock.reset();
                state.history.clear();
            }
            if session_must_reopen(&error) {
                state.session = None;
            }
            state.schedule_failure(now_ms, error.to_string());
        }
    }
    state
}

const fn clock_model_must_reset(error: &BrowserClockError) -> bool {
    matches!(
        error,
        BrowserClockError::Clock(ClockProbeError::BootChanged { .. })
    )
}

const fn session_must_reopen(error: &BrowserClockError) -> bool {
    matches!(
        error,
        BrowserClockError::Session(_)
            | BrowserClockError::Clock(ClockProbeError::BootChanged { .. })
            | BrowserClockError::Fetch(
                BrowserFetchError::Session(_)
                    | BrowserFetchError::HttpStatus(_)
                    | BrowserFetchError::MissingHeader(_),
            )
    )
}

fn finish_device_step(
    runtime: &SharedWorkerRuntime,
    connection_id: u64,
    generation: u64,
    state: DeviceState,
    probe_immediately: bool,
) {
    let restored = {
        let mut runtime_ref = runtime.borrow_mut();
        let current_generation = runtime_ref.devices.get(&connection_id).and_then(|entry| {
            if let DeviceEntry::Busy { generation, .. } = entry {
                Some(*generation)
            } else {
                None
            }
        });
        if current_generation == Some(generation) {
            runtime_ref
                .devices
                .insert(connection_id, DeviceEntry::Idle(Box::new(state)));
            true
        } else {
            false
        }
    };
    if !restored {
        return;
    }
    publish_snapshot(runtime, connection_id);
    if probe_immediately {
        launch_device_step(runtime, connection_id, false);
    }
}

fn worker_now_ms(scope: &DedicatedWorkerGlobalScope) -> f64 {
    let worker_scope: &WorkerGlobalScope = scope.as_ref();
    worker_scope
        .performance()
        .map_or(f64::NAN, |clock| clock.now())
}

fn publish_snapshot(runtime: &SharedWorkerRuntime, connection_id: u64) {
    let snapshot = runtime
        .borrow()
        .devices
        .get(&connection_id)
        .map(DeviceEntry::snapshot);
    if let Some(snapshot) = snapshot {
        emit_worker_event(runtime, WorkerEvent::Snapshot { snapshot });
    }
}

fn reject_command(runtime: &SharedWorkerRuntime, connection_id: Option<u64>, message: &str) {
    emit_worker_event(
        runtime,
        WorkerEvent::CommandRejected {
            connection_id,
            message: message.to_owned(),
        },
    );
}

fn emit_worker_event(runtime: &SharedWorkerRuntime, event: WorkerEvent) {
    let envelope = WorkerEventEnvelope::current(event);
    let Ok(json) = serde_json::to_string(&envelope) else {
        return;
    };
    let scope = runtime.borrow().scope.clone();
    let _ = scope.post_message(&JsValue::from_str(&json));
}

/// Rendering-realm lifecycle for the dedicated control worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorLifecycle {
    /// Module loading and WASM bootstrap are in progress.
    Starting,
    /// Worker acknowledged the exact schema and reported its inherited origin.
    Ready {
        /// Actual origin attached to authenticated CORS requests.
        scope_origin: String,
    },
    /// Worker bootstrap or browser transport failed globally.
    Failed(String),
}

/// Immutable rendering-realm copy of worker state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisorView {
    /// Current global worker lifecycle.
    pub lifecycle: SupervisorLifecycle,
    /// Stable connection snapshots sorted by UI-local identity.
    pub devices: Vec<DeviceSessionSnapshot>,
    /// Oldest-to-newest bounded supervision diagnostics.
    pub diagnostics: Vec<String>,
}

struct MutableSupervisorView {
    lifecycle: SupervisorLifecycle,
    devices: BTreeMap<u64, DeviceSessionSnapshot>,
    diagnostics: VecDeque<String>,
}

impl Default for MutableSupervisorView {
    fn default() -> Self {
        Self {
            lifecycle: SupervisorLifecycle::Starting,
            devices: BTreeMap::new(),
            diagnostics: VecDeque::new(),
        }
    }
}

impl MutableSupervisorView {
    fn push_diagnostic(&mut self, message: String) {
        if self.diagnostics.len() == MAXIMUM_UI_DIAGNOSTICS {
            self.diagnostics.pop_front();
        }
        self.diagnostics.push_back(message);
    }

    fn apply(&mut self, envelope: WorkerEventEnvelope) {
        if envelope.schema_version != WORKER_SCHEMA_VERSION {
            self.lifecycle = SupervisorLifecycle::Failed(
                "control worker uses an incompatible message schema".to_owned(),
            );
            return;
        }
        match envelope.event {
            WorkerEvent::Ready { scope_origin } => {
                self.lifecycle = SupervisorLifecycle::Ready { scope_origin };
            }
            WorkerEvent::Snapshot { snapshot } => {
                self.devices.insert(snapshot.connection_id, snapshot);
            }
            WorkerEvent::Removed { connection_id } => {
                self.devices.remove(&connection_id);
            }
            WorkerEvent::CommandRejected {
                connection_id,
                message,
            } => {
                let prefix = connection_id.map_or_else(
                    || "worker".to_owned(),
                    |identity| format!("connection {identity}"),
                );
                self.push_diagnostic(format!("{prefix}: {message}"));
            }
            WorkerEvent::Fatal { message } => {
                self.lifecycle = SupervisorLifecycle::Failed(message);
            }
        }
    }

    fn snapshot(&self) -> SupervisorView {
        SupervisorView {
            lifecycle: self.lifecycle.clone(),
            devices: self.devices.values().cloned().collect(),
            diagnostics: self.diagnostics.iter().cloned().collect(),
        }
    }
}

/// UI-thread owner of one dedicated worker and its redacted replacement snapshots.
pub struct BrowserWorkerSupervisor {
    worker: Worker,
    view: Rc<RefCell<MutableSupervisorView>>,
    on_message: Closure<dyn FnMut(MessageEvent)>,
    on_error: Closure<dyn FnMut(ErrorEvent)>,
}

impl BrowserWorkerSupervisor {
    /// Creates and supervises the module worker built from the same WASM binary.
    ///
    /// # Errors
    ///
    /// Returns the browser's worker-construction failure.
    pub fn new(context: &eframe::egui::Context) -> Result<Self, JsValue> {
        let options = WorkerOptions::new();
        options.set_type(WorkerType::Module);
        options.set_name("alumina-control");
        let worker = Worker::new_with_options(CONTROL_WORKER_URL, &options)?;
        let view = Rc::new(RefCell::new(MutableSupervisorView::default()));

        let message_view = Rc::clone(&view);
        let message_context = context.clone();
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let Some(json) = event.data().as_string() else {
                message_view
                    .borrow_mut()
                    .push_diagnostic("worker event was not a JSON string".to_owned());
                message_context.request_repaint();
                return;
            };
            match serde_json::from_str::<WorkerEventEnvelope>(&json) {
                Ok(envelope) => message_view.borrow_mut().apply(envelope),
                Err(_) => message_view
                    .borrow_mut()
                    .push_diagnostic("worker event JSON was rejected".to_owned()),
            }
            message_context.request_repaint();
        });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let error_view = Rc::clone(&view);
        let error_context = context.clone();
        let on_error = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
            error_view.borrow_mut().lifecycle = SupervisorLifecycle::Failed(event.message());
            error_context.request_repaint();
        });
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        Ok(Self {
            worker,
            view,
            on_message,
            on_error,
        })
    }

    /// Sends one typed command after exact schema serialization.
    ///
    /// # Errors
    ///
    /// Returns serialization or browser message-delivery failure without
    /// retaining the command's secret in diagnostic state.
    pub fn send(&self, command: WorkerCommand) -> Result<(), String> {
        let envelope = WorkerCommandEnvelope::current(command);
        let json = serde_json::to_string(&envelope)
            .map_err(|_| "worker command serialization failed".to_owned())?;
        let result = self
            .worker
            .post_message(&JsValue::from_str(&json))
            .map_err(|_| "browser rejected the worker command".to_owned());
        let mut json = json.into_bytes();
        json.fill(0);
        result
    }

    /// Clones one coherent redacted rendering-realm view.
    #[must_use]
    pub fn view(&self) -> SupervisorView {
        self.view.borrow().snapshot()
    }
}

impl Drop for BrowserWorkerSupervisor {
    fn drop(&mut self) {
        self.worker.set_onmessage(None);
        self.worker.set_onerror(None);
        self.worker.terminate();
        let _ = (&self.on_message, &self.on_error);
    }
}
