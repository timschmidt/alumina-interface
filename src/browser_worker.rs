//! Dedicated browser control worker and rendering-realm supervisor.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use alumina_board::GraphResourceAccess;
use alumina_capability::{BoardCapabilityLimits, decode_board_capability, decode_resource_id};
use alumina_clock::{ClockFlags, ClockObservation};
use alumina_diagnostics::transport::{
    DiagnosticTransportLimits, SubscriptionId, TelemetrySubscribeFlags, TelemetrySubscribeRequest,
    WaveformConfigureFlags, WaveformConfigureRequest, decode_telemetry_event,
    decode_telemetry_subscribe, encode_telemetry_subscribe, encode_waveform_configure,
    telemetry_event_encoded_len, telemetry_subscribe_encoded_len, waveform_configure_encoded_len,
};
use alumina_diagnostics::{
    CaptureId, DiagnosticContext, DiagnosticLimits, DigitalCaptureView, DigitalTriggerCondition,
    decode_digital_capture, resource_overview_encoded_len,
};
use alumina_interface_client::capability::{CapabilityDownloadMachine, CapabilityDownloadPhase};
use alumina_interface_client::clock::{ClockProbeError, DeviceClockModel};
use alumina_interface_client::diagnostics::{
    TelemetrySubscriptionMachine, WaveformCaptureMachine, WaveformClientPhase,
};
use alumina_interface_client::health::RuntimeHealthModel;
use alumina_interface_client::http::{AuthenticatedHttpSession, DeviceIdentity};
use alumina_interface_client::wasm::{
    BrowserCapabilityError, BrowserClockError, BrowserFetchError, BrowserHealthError,
    BrowserTelemetryError, BrowserWaveformError, DeviceOrigin, drive_capability_step_in_worker,
    drive_clock_probe_in_worker, drive_runtime_health_in_worker, drive_telemetry_step_in_worker,
    drive_waveform_step_in_worker, fetch_device_identity_in_worker,
    open_authenticated_session_in_worker, worker_origin,
};
use alumina_interface_client::worker::{
    CapabilityDownloadPhaseSnapshot, CapabilityIdentitySnapshot, ClockEstimateSnapshot,
    ClockHistoryRecord, DeviceConnectionRequest, DeviceIdentitySnapshot, DeviceSessionPhase,
    DeviceSessionSnapshot, MAXIMUM_CLOCK_HISTORY, MAXIMUM_WORKER_DIAGNOSTIC_BYTES,
    RuntimeHealthWorkerSnapshot, TelemetryPhaseSnapshot, WORKER_SCHEMA_VERSION,
    WorkerCapabilityDocument, WorkerCommand, WorkerCommandEnvelope, WorkerEvent,
    WorkerEventEnvelope, WorkerTelemetryDocument, WorkerWaveformDocument, WorkerWaveformRequest,
};
use alumina_interface_core::board_explorer::{
    BoardExplorerSnapshot, build_board_explorer_snapshot,
};
use alumina_protocol::{DeviceCycle, Digest};
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
const CAPABILITY_RANGES_PER_HEARTBEAT: usize = 4;
const WAVEFORM_OPERATIONS_PER_HEARTBEAT: usize = 8;
const WAVEFORM_TRANSITION_CAPACITY: u32 = 64;
const WAVEFORM_CHUNK_BYTES: u32 = 168;
const WAVEFORM_ARM_HORIZON_SECONDS: u64 = 30;
const TELEMETRY_RESOURCE_LIMIT: usize = 4;
const TELEMETRY_UPDATES_PER_SECOND: u64 = 10;

struct DeviceState {
    connection_id: u64,
    label: String,
    origin_text: String,
    origin: DeviceOrigin,
    secret: Vec<u8>,
    sampling: alumina_interface_client::worker::ClockSamplingPolicy,
    generation: u64,
    session: Option<AuthenticatedHttpSession>,
    identity: Option<DeviceIdentity>,
    clock: DeviceClockModel,
    runtime_health: RuntimeHealthModel,
    capability: CapabilityDownloadMachine,
    capability_event_published: bool,
    history: VecDeque<ClockHistoryRecord>,
    phase: DeviceSessionPhase,
    consecutive_failures: u32,
    estimate: Option<ClockEstimateSnapshot>,
    last_error: Option<String>,
    next_attempt_ms: f64,
    runtime_health_consecutive_failures: u32,
    runtime_health_last_error: Option<String>,
    next_runtime_health_attempt_ms: f64,
    capability_consecutive_failures: u32,
    capability_last_error: Option<String>,
    telemetry: Option<TelemetrySubscriptionMachine>,
    pending_telemetry_event: Option<Vec<u8>>,
    telemetry_consecutive_failures: u32,
    telemetry_last_error: Option<String>,
    waveform: Option<WaveformCaptureMachine>,
    pending_waveform_request: Option<WorkerWaveformRequest>,
    waveform_event_published: bool,
    waveform_consecutive_failures: u32,
    waveform_last_error: Option<String>,
    next_capture_sequence: u64,
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
        let capability = CapabilityDownloadMachine::new(BoardCapabilityLimits::interactive())
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection_id: request.connection_id,
            label: std::mem::take(&mut request.label),
            origin_text: std::mem::take(&mut request.origin),
            origin,
            secret: std::mem::take(&mut request.secret),
            sampling: request.sampling,
            generation,
            session: None,
            identity: None,
            clock,
            runtime_health: RuntimeHealthModel::new(),
            capability,
            capability_event_published: false,
            history: VecDeque::with_capacity(MAXIMUM_CLOCK_HISTORY),
            phase: DeviceSessionPhase::Connecting,
            consecutive_failures: 0,
            estimate: None,
            last_error: None,
            next_attempt_ms: 0.0,
            runtime_health_consecutive_failures: 0,
            runtime_health_last_error: None,
            next_runtime_health_attempt_ms: 0.0,
            capability_consecutive_failures: 0,
            capability_last_error: None,
            telemetry: None,
            pending_telemetry_event: None,
            telemetry_consecutive_failures: 0,
            telemetry_last_error: None,
            waveform: None,
            pending_waveform_request: None,
            waveform_event_published: false,
            waveform_consecutive_failures: 0,
            waveform_last_error: None,
            next_capture_sequence: 1,
        })
    }

    fn snapshot(&self) -> DeviceSessionSnapshot {
        let (
            telemetry_phase,
            telemetry_subscription_id,
            telemetry_subscription_digest,
            telemetry_event_sequence,
            telemetry_dropped_events,
        ) = self
            .telemetry
            .as_ref()
            .map_or((None, None, None, 0, 0), |telemetry| {
                let reference = telemetry.reference();
                let progress = telemetry.event_progress();
                (
                    Some(telemetry.phase().into()),
                    Some(reference.subscription_id.get()),
                    Some(reference.subscription_digest.0),
                    progress.map_or(0, |progress| progress.event_sequence),
                    progress.map_or(0, |progress| progress.dropped_events),
                )
            });
        let (waveform_phase, waveform_capture_id, waveform_received_bytes, waveform_total_bytes) =
            self.waveform
                .as_ref()
                .map_or((None, None, 0, 0), |waveform| {
                    let (received, total) = match waveform.phase() {
                        WaveformClientPhase::Downloading {
                            received_bytes,
                            total_bytes,
                        } => (received_bytes, total_bytes),
                        WaveformClientPhase::Complete => {
                            let bytes = waveform
                                .record()
                                .and_then(|record| u32::try_from(record.len()).ok())
                                .unwrap_or(0);
                            (bytes, bytes)
                        }
                        _ => (0, 0),
                    };
                    (
                        Some(waveform.phase().into()),
                        Some(waveform.reference().capture_id.as_bytes()),
                        received,
                        total,
                    )
                });
        DeviceSessionSnapshot {
            connection_id: self.connection_id,
            label: self.label.clone(),
            origin: self.origin_text.clone(),
            generation: self.generation,
            phase: self.phase,
            boot_id: self.clock.boot_id().map(alumina_clock::BootId::as_bytes),
            device_identity: self
                .identity
                .as_ref()
                .map(DeviceIdentitySnapshot::from_identity),
            accepted_samples: self.clock.accepted_samples(),
            rejected_samples: self.clock.rejected_samples(),
            consecutive_failures: self.consecutive_failures,
            estimate: self.estimate,
            history: self.history.iter().copied().collect(),
            last_error: self.last_error.clone(),
            runtime_health_availability: self.runtime_health.availability().into(),
            runtime_health: self
                .runtime_health
                .latest()
                .map(RuntimeHealthWorkerSnapshot::from_view),
            runtime_health_consecutive_failures: self.runtime_health_consecutive_failures,
            runtime_health_last_error: self.runtime_health_last_error.clone(),
            capability_phase: self.capability.phase().into(),
            capability_received_bytes: self.capability.progress().received_bytes,
            capability_identity: self
                .capability
                .identity()
                .map(CapabilityIdentitySnapshot::from_identity),
            capability_consecutive_failures: self.capability_consecutive_failures,
            capability_last_error: self.capability_last_error.clone(),
            telemetry_phase,
            telemetry_subscription_id,
            telemetry_subscription_digest,
            telemetry_event_sequence,
            telemetry_dropped_events,
            telemetry_consecutive_failures: self.telemetry_consecutive_failures,
            telemetry_last_error: self.telemetry_last_error.clone(),
            waveform_phase,
            waveform_capture_id,
            waveform_received_bytes,
            waveform_total_bytes,
            waveform_consecutive_failures: self.waveform_consecutive_failures,
            waveform_last_error: self.waveform_last_error.clone(),
        }
    }

    fn schedule_success(&mut self, now_ms: f64) {
        self.consecutive_failures = 0;
        self.last_error = None;
        self.next_attempt_ms = now_ms + f64::from(self.sampling.heartbeat_interval_ms);
    }

    fn schedule_failure(&mut self, now_ms: f64, error: &str) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.phase = DeviceSessionPhase::RetryWaiting;
        self.estimate = None;
        self.last_error = Some(bounded_diagnostic(error));
        let exponent = self.consecutive_failures.saturating_sub(1).min(5);
        let multiplier = 1_u32 << exponent;
        let retry_ms = self
            .sampling
            .heartbeat_interval_ms
            .saturating_mul(multiplier)
            .min(MAXIMUM_RETRY_MS);
        self.next_attempt_ms = now_ms + f64::from(retry_ms);
    }

    fn runtime_health_due(&self, now_ms: f64) -> bool {
        self.next_runtime_health_attempt_ms <= now_ms
    }

    fn schedule_runtime_health_attempt(&mut self, now_ms: f64) {
        self.next_runtime_health_attempt_ms =
            now_ms + f64::from(self.sampling.runtime_health_interval_ms);
    }

    fn record_runtime_health_success(&mut self) {
        self.runtime_health_consecutive_failures = 0;
        self.runtime_health_last_error = None;
    }

    fn record_runtime_health_failure(&mut self, error: &str) {
        self.runtime_health_consecutive_failures =
            self.runtime_health_consecutive_failures.saturating_add(1);
        self.runtime_health_last_error = Some(bounded_diagnostic(error));
    }

    fn reset_runtime_health_evidence(&mut self) {
        self.runtime_health.reset();
    }

    fn reset_runtime_health_for_new_boot(&mut self) {
        self.reset_runtime_health_evidence();
        self.runtime_health_consecutive_failures = 0;
        self.runtime_health_last_error = None;
    }

    fn record_capability_success(&mut self) {
        self.capability_consecutive_failures = 0;
        self.capability_last_error = None;
    }

    fn record_capability_failure(&mut self, error: &str) {
        self.capability_consecutive_failures =
            self.capability_consecutive_failures.saturating_add(1);
        self.capability_last_error = Some(bounded_diagnostic(error));
    }

    fn reset_capability_for_new_boot(&mut self) {
        self.capability.reset();
        self.capability_event_published = false;
        self.capability_consecutive_failures = 0;
        self.capability_last_error = None;
        self.reset_telemetry_for_new_boot();
    }

    fn record_telemetry_success(&mut self) {
        self.telemetry_consecutive_failures = 0;
        self.telemetry_last_error = None;
    }

    fn record_telemetry_failure(&mut self, error: &str) {
        self.telemetry_consecutive_failures = self.telemetry_consecutive_failures.saturating_add(1);
        self.telemetry_last_error = Some(bounded_diagnostic(error));
    }

    fn reset_telemetry_for_new_boot(&mut self) {
        self.telemetry = None;
        self.pending_telemetry_event = None;
        self.telemetry_consecutive_failures = 0;
        self.telemetry_last_error = None;
    }

    fn start_telemetry(&mut self) -> Result<bool, String> {
        if self.telemetry.is_some() {
            return Ok(true);
        }
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| "device identity has not been acquired".to_owned())?;
        let capability_identity = self
            .capability
            .identity()
            .ok_or_else(|| "board capability identity is unavailable".to_owned())?;
        if self.capability.phase() != CapabilityDownloadPhase::Complete
            || capability_identity != identity.capability()
        {
            return Err("device identity and complete capability are not reconciled".to_owned());
        }
        let document = self
            .capability
            .document()
            .ok_or_else(|| "complete capability bytes are unavailable".to_owned())?;
        let capability = decode_board_capability(document, BoardCapabilityLimits::interactive())
            .map_err(|error| format!("board capability rejected: {error:?}"))?;
        if capability.board_id() != identity.board_id() {
            return Err("public board identity does not match capability bytes".to_owned());
        }
        let mut resources: Vec<_> = capability
            .graph()
            .resources()
            .filter(|resource| resource.access == GraphResourceAccess::StableBooleanInput)
            .map(|resource| resource.resource)
            .take(TELEMETRY_RESOURCE_LIMIT)
            .collect();
        resources.sort_unstable();
        resources.dedup();
        if resources.is_empty() {
            return Ok(false);
        }

        let boot_id = self
            .clock
            .boot_id()
            .ok_or_else(|| "authenticated boot identity is unavailable".to_owned())?;
        let latest = self
            .history
            .back()
            .ok_or_else(|| "authenticated clock evidence is unavailable".to_owned())?;
        let overview_bytes = resource_overview_encoded_len(resources.len())
            .map_err(|error| format!("telemetry overview length rejected: {error}"))?;
        let event_bytes = telemetry_event_encoded_len(overview_bytes)
            .map_err(|error| format!("telemetry event length rejected: {error}"))?;
        let encoded_len = telemetry_subscribe_encoded_len(resources.len())
            .map_err(|error| format!("telemetry subscribe length rejected: {error}"))?;
        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(encoded_len)
            .map_err(|_| "telemetry subscription allocation failed".to_owned())?;
        encoded.resize(encoded_len, 0);
        let context = DiagnosticContext {
            device_id: identity.device_id(),
            boot_id,
            capability: capability_identity,
            config_digest: Digest::ZERO,
            clock_frequency_hz: latest.frequency_hz,
        };
        let used = encode_telemetry_subscribe(
            &TelemetrySubscribeRequest {
                subscription_id: SubscriptionId::new(self.generation)
                    .map_err(|error| format!("telemetry identity rejected: {error}"))?,
                context,
                flags: TelemetrySubscribeFlags(TelemetrySubscribeFlags::LATEST_ONLY),
                minimum_period_cycles: latest
                    .frequency_hz
                    .checked_div(TELEMETRY_UPDATES_PER_SECOND)
                    .unwrap_or(0)
                    .max(1),
                maximum_event_bytes: u32::try_from(event_bytes)
                    .map_err(|_| "telemetry event length does not fit protocol".to_owned())?,
                resources: &resources,
            },
            &mut encoded,
            DiagnosticTransportLimits::native_control(),
        )
        .map_err(|error| format!("telemetry subscription rejected: {error}"))?;
        encoded.truncate(used);
        self.telemetry = Some(
            TelemetrySubscriptionMachine::new(
                encoded,
                DiagnosticTransportLimits::native_control(),
                DiagnosticLimits::interactive(),
            )
            .map_err(|error| error.to_string())?,
        );
        self.pending_telemetry_event = None;
        self.record_telemetry_success();
        Ok(true)
    }

    fn record_waveform_success(&mut self) {
        self.waveform_consecutive_failures = 0;
        self.waveform_last_error = None;
    }

    fn record_waveform_failure(&mut self, error: &str) {
        self.waveform_consecutive_failures = self.waveform_consecutive_failures.saturating_add(1);
        self.waveform_last_error = Some(bounded_diagnostic(error));
    }

    fn reset_waveform_for_new_boot(&mut self) {
        self.waveform = None;
        self.pending_waveform_request = None;
        self.waveform_event_published = false;
        self.waveform_consecutive_failures = 0;
        self.waveform_last_error = None;
    }

    fn request_waveform(&mut self, request: &WorkerWaveformRequest) -> Result<(), String> {
        request.validate().map_err(|error| error.to_string())?;
        match self.waveform.as_ref().map(WaveformCaptureMachine::phase) {
            None | Some(WaveformClientPhase::Stopped) => self.start_waveform(request),
            Some(WaveformClientPhase::Complete) => {
                self.waveform
                    .as_mut()
                    .expect("complete waveform exists")
                    .request_stop()
                    .map_err(|error| error.to_string())?;
                self.pending_waveform_request = Some(request.clone());
                self.record_waveform_success();
                Ok(())
            }
            Some(_) => Err("a waveform acquisition is already active".to_owned()),
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one linear audit surface binds every capture field to live identity and capability authority"
    )]
    fn start_waveform(&mut self, request: &WorkerWaveformRequest) -> Result<(), String> {
        request.validate().map_err(|error| error.to_string())?;
        if self
            .waveform
            .as_ref()
            .is_some_and(|waveform| waveform.phase() != WaveformClientPhase::Stopped)
        {
            return Err("prior waveform evidence has not been released".to_owned());
        }
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| "device identity has not been acquired".to_owned())?;
        let capability_identity = self
            .capability
            .identity()
            .ok_or_else(|| "board capability identity is unavailable".to_owned())?;
        if self.capability.phase() != CapabilityDownloadPhase::Complete
            || capability_identity != identity.capability()
        {
            return Err("device identity and complete capability are not reconciled".to_owned());
        }
        let document = self
            .capability
            .document()
            .ok_or_else(|| "complete capability bytes are unavailable".to_owned())?;
        let capability = decode_board_capability(document, BoardCapabilityLimits::interactive())
            .map_err(|error| format!("board capability rejected: {error:?}"))?;
        if capability.board_id() != identity.board_id() {
            return Err("public board identity does not match capability bytes".to_owned());
        }

        let mut channels = Vec::new();
        channels
            .try_reserve_exact(request.channels.len())
            .map_err(|_| "waveform channel allocation failed".to_owned())?;
        for encoded in &request.channels {
            let resource = decode_resource_id(encoded)
                .map_err(|_| "waveform resource selector is invalid".to_owned())?;
            let readable = capability.graph().resources().any(|candidate| {
                candidate.resource == resource
                    && candidate.access == GraphResourceAccess::StableBooleanInput
            });
            if !readable {
                return Err(format!(
                    "resource {resource:?} is not an admitted stable Boolean input"
                ));
            }
            channels.push(resource);
        }

        let boot_id = self
            .clock
            .boot_id()
            .ok_or_else(|| "authenticated boot identity is unavailable".to_owned())?;
        let latest = self
            .history
            .back()
            .ok_or_else(|| "authenticated clock evidence is unavailable".to_owned())?;
        let maximum_duration = latest
            .frequency_hz
            .checked_mul(2)
            .ok_or_else(|| "clock frequency cannot bound capture duration".to_owned())?;
        if request.duration_cycles > maximum_duration {
            return Err("waveform duration exceeds the two-second interactive bound".to_owned());
        }
        let latest_trigger_cycle = latest
            .transmit_cycle
            .checked_add(
                latest
                    .frequency_hz
                    .checked_mul(WAVEFORM_ARM_HORIZON_SECONDS)
                    .ok_or_else(|| "waveform arm horizon overflowed".to_owned())?,
            )
            .ok_or_else(|| "waveform arm deadline overflowed".to_owned())?;
        let capture_sequence = self.next_capture_sequence;
        self.next_capture_sequence = capture_sequence
            .checked_add(1)
            .ok_or_else(|| "waveform attempt identity is exhausted".to_owned())?;
        let mut capture_bytes = [0_u8; 16];
        capture_bytes[..8].copy_from_slice(&self.generation.to_le_bytes());
        capture_bytes[8..].copy_from_slice(&capture_sequence.to_le_bytes());
        let capture_id = CaptureId::new(capture_bytes)
            .map_err(|_| "waveform attempt identity is invalid".to_owned())?;
        let context = DiagnosticContext {
            device_id: identity.device_id(),
            boot_id,
            capability: capability_identity,
            config_digest: Digest::ZERO,
            clock_frequency_hz: latest.frequency_hz,
        };
        let encoded_len = waveform_configure_encoded_len(channels.len())
            .map_err(|error| format!("waveform configure length rejected: {error}"))?;
        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(encoded_len)
            .map_err(|_| "waveform configure allocation failed".to_owned())?;
        encoded.resize(encoded_len, 0);
        let used = encode_waveform_configure(
            &WaveformConfigureRequest {
                capture_id,
                context,
                flags: WaveformConfigureFlags(WaveformConfigureFlags::EDGE_TIMESTAMPS),
                requested_pretrigger_cycles: 0,
                requested_posttrigger_cycles: request.duration_cycles,
                earliest_trigger_cycle: DeviceCycle(latest.transmit_cycle),
                latest_trigger_cycle: DeviceCycle(latest_trigger_cycle),
                transition_capacity: WAVEFORM_TRANSITION_CAPACITY,
                maximum_chunk_bytes: WAVEFORM_CHUNK_BYTES,
                trigger_channel_index: u16::MAX,
                trigger_condition: DigitalTriggerCondition::Immediate,
                channels: &channels,
            },
            &mut encoded,
            DiagnosticTransportLimits::native_control(),
        )
        .map_err(|error| format!("waveform configure rejected: {error}"))?;
        encoded.truncate(used);
        self.waveform = Some(
            WaveformCaptureMachine::new(
                encoded,
                DiagnosticTransportLimits::native_control(),
                DiagnosticLimits::interactive(),
            )
            .map_err(|error| error.to_string())?,
        );
        self.pending_waveform_request = None;
        self.waveform_event_published = false;
        self.record_waveform_success();
        Ok(())
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

fn bounded_diagnostic(diagnostic: &str) -> String {
    let diagnostic = match diagnostic.trim() {
        "" => "unspecified worker failure",
        canonical => canonical,
    };
    if diagnostic.len() <= MAXIMUM_WORKER_DIAGNOSTIC_BYTES {
        return diagnostic.to_owned();
    }
    let mut boundary = MAXIMUM_WORKER_DIAGNOSTIC_BYTES;
    while !diagnostic.is_char_boundary(boundary) {
        boundary -= 1;
    }
    diagnostic[..boundary].to_owned()
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
        snapshot: Box<DeviceSessionSnapshot>,
    },
}

impl DeviceEntry {
    fn snapshot(&self) -> DeviceSessionSnapshot {
        match self {
            Self::Idle(state) => state.snapshot(),
            Self::Busy { snapshot, .. } => snapshot.as_ref().clone(),
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
        WorkerCommand::CaptureWaveform { request } => {
            start_waveform_capture(runtime, &request);
        }
        WorkerCommand::Disconnect { connection_id } => disconnect_device(runtime, connection_id),
    }
}

const fn command_connection_id(command: &WorkerCommand) -> u64 {
    match command {
        WorkerCommand::Configure { request } => request.connection_id,
        WorkerCommand::CaptureWaveform { request } => request.connection_id,
        WorkerCommand::ProbeNow { connection_id } | WorkerCommand::Disconnect { connection_id } => {
            *connection_id
        }
    }
}

fn start_waveform_capture(runtime: &SharedWorkerRuntime, request: &WorkerWaveformRequest) {
    let connection_id = request.connection_id;
    if let Err(error) = request.validate() {
        reject_command(runtime, Some(connection_id), &error.to_string());
        return;
    }
    let result = {
        let mut runtime_ref = runtime.borrow_mut();
        match runtime_ref.devices.get_mut(&connection_id) {
            Some(DeviceEntry::Idle(state)) => state.request_waveform(request),
            Some(DeviceEntry::Busy { .. }) => {
                Err("connection is busy; retry the capture request".to_owned())
            }
            None => Err("connection does not exist".to_owned()),
        }
    };
    if let Err(error) = result {
        reject_command(runtime, Some(connection_id), &error);
        return;
    }
    publish_snapshot(runtime, connection_id);
    launch_device_step(runtime, connection_id, false);
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
                    snapshot: Box::new(snapshot),
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
        match open_authenticated_session_in_worker(worker_scope, &state.origin, Digest::ZERO).await
        {
            Ok(session) => fetch_device_identity_in_worker(worker_scope, &state.origin)
                .await
                .map(|identity| (session, identity)),
            Err(error) => Err(error),
        };
    let now_ms = worker_now_ms(&scope);
    match result {
        Ok((session, identity)) => {
            let identity_changed = state
                .identity
                .as_ref()
                .is_some_and(|previous| previous != &identity);
            if identity_changed {
                let _ = state.clock.reset();
                state.history.clear();
                state.estimate = None;
                state.reset_runtime_health_for_new_boot();
                state.reset_capability_for_new_boot();
                state.reset_waveform_for_new_boot();
            } else if state
                .capability
                .identity()
                .is_some_and(|capability| capability != identity.capability())
            {
                state.reset_capability_for_new_boot();
                state.reset_waveform_for_new_boot();
            }
            state.session = Some(session);
            state.identity = Some(identity);
            state.reset_runtime_health_evidence();
            state.phase = DeviceSessionPhase::Sampling;
            state.schedule_success(now_ms);
            state.next_attempt_ms = now_ms;
        }
        Err(error) => state.schedule_failure(now_ms, &error.to_string()),
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
            probe_runtime_health(&scope, worker_scope, &mut state, now_ms).await;
            download_capability(worker_scope, &mut state).await;
            if state.session.is_some()
                && state.capability.phase() == CapabilityDownloadPhase::Complete
                && let Err(error) = capability_reconciles_identity(&state)
            {
                state.record_capability_failure(error);
                state.session = None;
            }
            if state.session.is_some()
                && state.capability.phase() == CapabilityDownloadPhase::Complete
            {
                match state.start_telemetry() {
                    Ok(true) => drive_state_telemetry(worker_scope, &mut state).await,
                    Ok(false) => {}
                    Err(error) => state.record_telemetry_failure(&error),
                }
            }
            if state.session.is_some() && state.waveform.is_some() {
                drive_waveform_burst(worker_scope, &mut state).await;
            }
        }
        Err(error) => {
            if clock_model_must_reset(&error) {
                let _ = state.clock.reset();
                state.history.clear();
                state.reset_runtime_health_for_new_boot();
                state.reset_capability_for_new_boot();
                state.reset_waveform_for_new_boot();
                state.identity = None;
            }
            if session_must_reopen(&error) {
                state.session = None;
            }
            state.schedule_failure(now_ms, &error.to_string());
        }
    }
    state
}

async fn probe_runtime_health(
    scope: &DedicatedWorkerGlobalScope,
    worker_scope: &WorkerGlobalScope,
    state: &mut DeviceState,
    fallback_now_ms: f64,
) {
    if !state.runtime_health_due(fallback_now_ms) {
        return;
    }
    let health_result = drive_runtime_health_in_worker(
        worker_scope,
        &state.origin,
        state
            .session
            .as_mut()
            .expect("clock probe retained its authenticated session"),
        &mut state.runtime_health,
        &state.secret,
    )
    .await;
    let sampled_now_ms = worker_now_ms(scope);
    state.schedule_runtime_health_attempt(if sampled_now_ms.is_finite() {
        sampled_now_ms
    } else {
        fallback_now_ms
    });
    match health_result {
        Ok(_) => state.record_runtime_health_success(),
        Err(error) => {
            if health_session_must_reopen(&error) {
                state.session = None;
            }
            state.record_runtime_health_failure(&error.to_string());
        }
    }
}

async fn download_capability(worker_scope: &WorkerGlobalScope, state: &mut DeviceState) {
    if state.session.is_none() || state.capability.phase() == CapabilityDownloadPhase::Complete {
        return;
    }
    for _ in 0..CAPABILITY_RANGES_PER_HEARTBEAT {
        let result = drive_capability_step_in_worker(
            worker_scope,
            &state.origin,
            state
                .session
                .as_mut()
                .expect("capability burst retains its authenticated session"),
            &mut state.capability,
            &state.secret,
        )
        .await;
        match result {
            Ok(phase) => {
                state.record_capability_success();
                if phase == CapabilityDownloadPhase::Complete {
                    break;
                }
            }
            Err(error) => {
                if capability_session_must_reopen(&error) {
                    state.session = None;
                }
                state.record_capability_failure(&error.to_string());
                break;
            }
        }
    }
}

fn capability_reconciles_identity(state: &DeviceState) -> Result<(), &'static str> {
    let identity = state
        .identity
        .as_ref()
        .ok_or("public device identity is unavailable")?;
    let capability_identity = state
        .capability
        .identity()
        .ok_or("complete capability lacks an identity")?;
    if capability_identity != identity.capability() {
        return Err("public identity does not match signed capability identity");
    }
    let document = state
        .capability
        .document()
        .ok_or("complete capability lacks canonical bytes")?;
    let capability = decode_board_capability(document, BoardCapabilityLimits::interactive())
        .map_err(|_| "complete capability cannot be decoded")?;
    if capability.board_id() != identity.board_id() {
        return Err("public board ID does not match signed capability bytes");
    }
    Ok(())
}

async fn drive_state_waveform(
    worker_scope: &WorkerGlobalScope,
    state: &mut DeviceState,
) -> Result<WaveformClientPhase, BrowserWaveformError> {
    let session = state
        .session
        .as_mut()
        .expect("caller checked authenticated session");
    let waveform = state
        .waveform
        .as_mut()
        .expect("caller checked waveform lifecycle");
    drive_waveform_step_in_worker(
        worker_scope,
        &state.origin,
        session,
        waveform,
        &state.secret,
    )
    .await
}

async fn drive_state_telemetry(worker_scope: &WorkerGlobalScope, state: &mut DeviceState) {
    let result = drive_telemetry_step_in_worker(
        worker_scope,
        &state.origin,
        state
            .session
            .as_mut()
            .expect("caller checked authenticated session"),
        state
            .telemetry
            .as_mut()
            .expect("start_telemetry admitted a subscription"),
        &state.secret,
    )
    .await;
    match result {
        Ok(update) => {
            state.record_telemetry_success();
            if let Some(event) = update.event {
                state.pending_telemetry_event = Some(event);
            }
        }
        Err(error) => {
            if telemetry_session_must_reopen(&error) {
                state.session = None;
            }
            state.record_telemetry_failure(&error.to_string());
        }
    }
}

async fn drive_waveform_burst(worker_scope: &WorkerGlobalScope, state: &mut DeviceState) {
    for _ in 0..WAVEFORM_OPERATIONS_PER_HEARTBEAT {
        if let Err(error) = install_pending_waveform(state) {
            state.record_waveform_failure(&error);
            break;
        }
        let phase = state
            .waveform
            .as_ref()
            .expect("caller checked waveform existence")
            .phase();
        if phase == WaveformClientPhase::Configured
            && let Err(error) = state
                .waveform
                .as_mut()
                .expect("caller checked waveform existence")
                .request_arm()
        {
            state.record_waveform_failure(&error.to_string());
            break;
        }
        let before = state
            .waveform
            .as_ref()
            .expect("caller checked waveform existence")
            .phase();
        if matches!(
            before,
            WaveformClientPhase::Complete | WaveformClientPhase::Stopped
        ) {
            break;
        }
        match drive_state_waveform(worker_scope, state).await {
            Ok(after) => {
                state.record_waveform_success();
                if let Err(error) = install_pending_waveform(state) {
                    state.record_waveform_failure(&error);
                    break;
                }
                if after == WaveformClientPhase::Armed && before == WaveformClientPhase::Armed {
                    break;
                }
            }
            Err(error) => {
                if waveform_session_must_reopen(&error) {
                    state.session = None;
                }
                state.record_waveform_failure(&error.to_string());
                break;
            }
        }
    }
}

fn install_pending_waveform(state: &mut DeviceState) -> Result<(), String> {
    if state
        .waveform
        .as_ref()
        .is_none_or(|waveform| waveform.phase() != WaveformClientPhase::Stopped)
    {
        return Ok(());
    }
    let Some(request) = state.pending_waveform_request.take() else {
        return Ok(());
    };
    state.start_waveform(&request)
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
                    | BrowserFetchError::MissingHeader(_)
                    | BrowserFetchError::Media(_),
            )
    )
}

const fn health_session_must_reopen(error: &BrowserHealthError) -> bool {
    matches!(
        error,
        BrowserHealthError::ConfigurationIdentity
            | BrowserHealthError::Session(_)
            | BrowserHealthError::Fetch(
                BrowserFetchError::DocumentOrigin
                    | BrowserFetchError::Session(_)
                    | BrowserFetchError::HttpStatus(_)
                    | BrowserFetchError::MissingHeader(_)
                    | BrowserFetchError::Media(_),
            )
    )
}

const fn capability_session_must_reopen(error: &BrowserCapabilityError) -> bool {
    matches!(
        error,
        BrowserCapabilityError::ConfigurationIdentity
            | BrowserCapabilityError::Session(_)
            | BrowserCapabilityError::Fetch(
                BrowserFetchError::DocumentOrigin
                    | BrowserFetchError::Session(_)
                    | BrowserFetchError::HttpStatus(_)
                    | BrowserFetchError::MissingHeader(_)
                    | BrowserFetchError::Media(_),
            )
    )
}

const fn waveform_session_must_reopen(error: &BrowserWaveformError) -> bool {
    matches!(
        error,
        BrowserWaveformError::ConfigurationIdentity
            | BrowserWaveformError::Session(_)
            | BrowserWaveformError::Fetch(
                BrowserFetchError::DocumentOrigin
                    | BrowserFetchError::Session(_)
                    | BrowserFetchError::HttpStatus(_)
                    | BrowserFetchError::MissingHeader(_)
                    | BrowserFetchError::Media(_),
            )
    )
}

const fn telemetry_session_must_reopen(error: &BrowserTelemetryError) -> bool {
    matches!(
        error,
        BrowserTelemetryError::ConfigurationIdentity
            | BrowserTelemetryError::Session(_)
            | BrowserTelemetryError::Fetch(
                BrowserFetchError::DocumentOrigin
                    | BrowserFetchError::Session(_)
                    | BrowserFetchError::HttpStatus(_)
                    | BrowserFetchError::MissingHeader(_)
                    | BrowserFetchError::Media(_),
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
    publish_capability_document(runtime, connection_id);
    publish_telemetry_document(runtime, connection_id);
    publish_waveform_document(runtime, connection_id);
    publish_snapshot(runtime, connection_id);
    if probe_immediately {
        launch_device_step(runtime, connection_id, false);
    }
}

fn publish_telemetry_document(runtime: &SharedWorkerRuntime, connection_id: u64) {
    let transfer = {
        let mut runtime_ref = runtime.borrow_mut();
        let Some(DeviceEntry::Idle(state)) = runtime_ref.devices.get_mut(&connection_id) else {
            return;
        };
        let Some(event) = state.pending_telemetry_event.take() else {
            return;
        };
        let Some(telemetry) = state.telemetry.as_ref() else {
            return;
        };
        WorkerTelemetryDocument::try_new(
            state.connection_id,
            state.generation,
            telemetry.request_bytes().to_vec(),
            event,
        )
    };
    match transfer {
        Ok(telemetry) => emit_worker_event(
            runtime,
            WorkerEvent::TelemetryDocument {
                telemetry: Box::new(telemetry),
            },
        ),
        Err(error) => emit_worker_event(
            runtime,
            WorkerEvent::Fatal {
                message: format!("validated telemetry transfer failed: {error}"),
            },
        ),
    }
}

fn publish_waveform_document(runtime: &SharedWorkerRuntime, connection_id: u64) {
    let transfer = {
        let mut runtime_ref = runtime.borrow_mut();
        let Some(DeviceEntry::Idle(state)) = runtime_ref.devices.get_mut(&connection_id) else {
            return;
        };
        if state.waveform_event_published {
            return;
        }
        let Some(waveform) = state.waveform.as_ref() else {
            return;
        };
        let Some(record) = waveform.record() else {
            return;
        };
        let transfer =
            WorkerWaveformDocument::try_new(state.connection_id, state.generation, record.to_vec());
        if transfer.is_ok() {
            state.waveform_event_published = true;
        }
        transfer
    };
    match transfer {
        Ok(waveform) => emit_worker_event(
            runtime,
            WorkerEvent::WaveformDocument {
                waveform: Box::new(waveform),
            },
        ),
        Err(error) => emit_worker_event(
            runtime,
            WorkerEvent::Fatal {
                message: format!("validated waveform transfer failed: {error}"),
            },
        ),
    }
}

fn publish_capability_document(runtime: &SharedWorkerRuntime, connection_id: u64) {
    let transfer = {
        let mut runtime_ref = runtime.borrow_mut();
        let Some(DeviceEntry::Idle(state)) = runtime_ref.devices.get_mut(&connection_id) else {
            return;
        };
        if state.capability_event_published
            || state.capability.phase() != CapabilityDownloadPhase::Complete
        {
            return;
        }
        let Some(identity) = state.capability.identity() else {
            return;
        };
        let Some(document) = state.capability.document() else {
            return;
        };
        let transfer = WorkerCapabilityDocument::try_new(
            state.connection_id,
            state.generation,
            identity,
            document.to_vec(),
        );
        if transfer.is_ok() {
            state.capability_event_published = true;
        }
        transfer
    };
    match transfer {
        Ok(capability) => emit_worker_event(
            runtime,
            WorkerEvent::CapabilityDocument {
                capability: Box::new(capability),
            },
        ),
        Err(error) => emit_worker_event(
            runtime,
            WorkerEvent::Fatal {
                message: format!("validated capability transfer failed: {error}"),
            },
        ),
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
        emit_worker_event(
            runtime,
            WorkerEvent::Snapshot {
                snapshot: Box::new(snapshot),
            },
        );
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
    /// Canonical connected-board documents decoded once per matching generation.
    pub capabilities: Vec<ConnectedCapabilityView>,
    /// Bounded exact overview history per connected telemetry subscription.
    pub telemetry: Vec<ConnectedTelemetryView>,
    /// Latest complete capability- and boot-bound digital capture per connection.
    pub waveforms: Vec<ConnectedWaveformView>,
    /// Oldest-to-newest bounded supervision diagnostics.
    pub diagnostics: Vec<String>,
}

/// Rendering-realm telemetry evidence independently rebound to live authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedTelemetryView {
    connection_id: u64,
    generation: u64,
    subscription: Vec<u8>,
    events: VecDeque<Vec<u8>>,
}

impl ConnectedTelemetryView {
    /// UI-local connection identity owning this subscription.
    #[must_use]
    pub const fn connection_id(&self) -> u64 {
        self.connection_id
    }

    /// Worker generation in which the subscription was created.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Number of exact canonical events retained for plots and status history.
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Reborrows the newest independently validated telemetry event.
    #[must_use]
    pub fn latest(&self) -> Option<alumina_diagnostics::transport::TelemetryEventView<'_>> {
        let subscription = decode_telemetry_subscribe(
            &self.subscription,
            DiagnosticTransportLimits::native_control(),
        )
        .ok()?;
        self.events.back().and_then(|event| {
            decode_telemetry_event(event, subscription, DiagnosticLimits::interactive()).ok()
        })
    }

    /// Iterates oldest-to-newest exact canonical event history.
    pub fn events(
        &self,
    ) -> impl Iterator<Item = alumina_diagnostics::transport::TelemetryEventView<'_>> + '_ {
        let subscription = decode_telemetry_subscribe(
            &self.subscription,
            DiagnosticTransportLimits::native_control(),
        )
        .ok();
        self.events.iter().filter_map(move |event| {
            subscription.and_then(|subscription| {
                decode_telemetry_event(event, subscription, DiagnosticLimits::interactive()).ok()
            })
        })
    }
}

/// Rendering-realm waveform evidence independently rebound to live session facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedWaveformView {
    connection_id: u64,
    generation: u64,
    capture_id: [u8; 16],
    record: Vec<u8>,
}

impl ConnectedWaveformView {
    /// UI-local connection identity owning this capture.
    #[must_use]
    pub const fn connection_id(&self) -> u64 {
        self.connection_id
    }

    /// Worker generation in which the capture was acquired.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Exact capture attempt retained by this evidence record.
    #[must_use]
    pub const fn capture_id(&self) -> [u8; 16] {
        self.capture_id
    }

    /// Reborrows the already validated canonical digital-capture record.
    ///
    /// # Panics
    ///
    /// Panics only if immutable bytes retained after successful supervisor
    /// admission are corrupted by an internal programming error.
    #[must_use]
    pub fn capture(&self) -> DigitalCaptureView<'_> {
        decode_digital_capture(&self.record, DiagnosticLimits::interactive())
            .expect("supervisor retained an independently validated capture")
    }

    /// Exact canonical record length.
    #[must_use]
    pub fn record_bytes(&self) -> usize {
        self.record.len()
    }
}

/// Rendering-realm board authority derived from one validated worker transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedCapabilityView {
    connection_id: u64,
    generation: u64,
    board: BoardExplorerSnapshot,
}

impl ConnectedCapabilityView {
    /// UI-local connection identity owning this board authority.
    #[must_use]
    pub const fn connection_id(&self) -> u64 {
        self.connection_id
    }

    /// Worker generation that acquired and authenticated the document.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Complete board-name-independent explorer snapshot.
    #[must_use]
    pub const fn board(&self) -> &BoardExplorerSnapshot {
        &self.board
    }
}

struct MutableSupervisorView {
    lifecycle: SupervisorLifecycle,
    devices: BTreeMap<u64, DeviceSessionSnapshot>,
    capabilities: BTreeMap<u64, ConnectedCapabilityView>,
    telemetry: BTreeMap<u64, ConnectedTelemetryView>,
    waveforms: BTreeMap<u64, ConnectedWaveformView>,
    diagnostics: VecDeque<String>,
}

impl Default for MutableSupervisorView {
    fn default() -> Self {
        Self {
            lifecycle: SupervisorLifecycle::Starting,
            devices: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            telemetry: BTreeMap::new(),
            waveforms: BTreeMap::new(),
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

    fn validate_telemetry_transfer(
        &self,
        telemetry: &WorkerTelemetryDocument,
    ) -> Result<(), &'static str> {
        let snapshot = self
            .devices
            .get(&telemetry.connection_id)
            .filter(|snapshot| snapshot.generation == telemetry.generation)
            .ok_or("session generation is stale or absent")?;
        let capability = self
            .capabilities
            .get(&telemetry.connection_id)
            .filter(|capability| capability.generation == telemetry.generation)
            .ok_or("matching complete board capability is absent")?;
        let identity = snapshot
            .device_identity
            .as_ref()
            .ok_or("public device identity is absent")?;
        let device_id = identity
            .validate()
            .map_err(|_| "public device identity is invalid")?;
        let subscription = decode_telemetry_subscribe(
            telemetry.subscription(),
            DiagnosticTransportLimits::native_control(),
        )
        .map_err(|_| "canonical telemetry subscription decoding failed")?;
        let event = decode_telemetry_event(
            telemetry.event(),
            subscription,
            DiagnosticLimits::interactive(),
        )
        .map_err(|_| "canonical telemetry event decoding failed")?;
        let context = subscription.context();
        if context.device_id != device_id
            || context.capability != capability.board.identity()
            || context.config_digest != Digest::ZERO
            || snapshot.boot_id != Some(context.boot_id.as_bytes())
            || identity.capability.identity().ok() != Some(context.capability)
            || snapshot.telemetry_phase != Some(TelemetryPhaseSnapshot::Active)
            || snapshot.telemetry_subscription_id != Some(subscription.subscription_id().get())
            || snapshot.telemetry_subscription_digest != Some(subscription.digest().0)
            || event.event_sequence() <= snapshot.telemetry_event_sequence
            || event.dropped_events() < snapshot.telemetry_dropped_events
        {
            return Err("telemetry context or progress does not match live device authority");
        }
        if snapshot
            .history
            .last()
            .is_none_or(|latest| latest.frequency_hz != context.clock_frequency_hz)
        {
            return Err("telemetry clock frequency does not match signed heartbeat evidence");
        }
        for sample in event.overview().samples() {
            let resource = capability
                .board
                .resource(sample.resource)
                .ok_or("telemetry names a resource absent from the board capability")?;
            if !resource
                .graph_accesses()
                .iter()
                .any(|access| access.access == GraphResourceAccess::StableBooleanInput)
            {
                return Err("telemetry resource lacks stable Boolean input authority");
            }
        }
        Ok(())
    }

    fn apply_telemetry_transfer(&mut self, telemetry: &WorkerTelemetryDocument) {
        let connection_id = telemetry.connection_id;
        let generation = telemetry.generation;
        if let Err(error) = self.validate_telemetry_transfer(telemetry) {
            self.push_diagnostic(format!(
                "connection {connection_id}: telemetry transfer rejected: {error}"
            ));
            return;
        }
        let subscription = decode_telemetry_subscribe(
            telemetry.subscription(),
            DiagnosticTransportLimits::native_control(),
        )
        .expect("validated telemetry transfer retains its subscription");
        let event = decode_telemetry_event(
            telemetry.event(),
            subscription,
            DiagnosticLimits::interactive(),
        )
        .expect("validated telemetry transfer retains its event");
        let entry = self
            .telemetry
            .entry(connection_id)
            .or_insert_with(|| ConnectedTelemetryView {
                connection_id,
                generation,
                subscription: telemetry.subscription().to_vec(),
                events: VecDeque::with_capacity(MAXIMUM_CLOCK_HISTORY),
            });
        let compatible = entry.generation == generation
            && entry.subscription == telemetry.subscription()
            && entry.latest().is_none_or(|previous| {
                event.event_sequence() > previous.event_sequence()
                    && event.dropped_events() >= previous.dropped_events()
                    && event.overview().snapshot_cycle().0 > previous.overview().snapshot_cycle().0
            });
        if !compatible {
            self.push_diagnostic(format!(
                "connection {connection_id}: telemetry history fork rejected"
            ));
            return;
        }
        if entry.events.len() == MAXIMUM_CLOCK_HISTORY {
            entry.events.pop_front();
        }
        entry.events.push_back(telemetry.event().to_vec());
    }

    fn validate_waveform_transfer(
        &self,
        waveform: &WorkerWaveformDocument,
    ) -> Result<(), &'static str> {
        let snapshot = self
            .devices
            .get(&waveform.connection_id)
            .filter(|snapshot| snapshot.generation == waveform.generation)
            .ok_or("session generation is stale or absent")?;
        let capability = self
            .capabilities
            .get(&waveform.connection_id)
            .filter(|capability| capability.generation == waveform.generation)
            .ok_or("matching complete board capability is absent")?;
        let identity = snapshot
            .device_identity
            .as_ref()
            .ok_or("public device identity is absent")?;
        let device_id = identity
            .validate()
            .map_err(|_| "public device identity is invalid")?;
        let capture = decode_digital_capture(waveform.record(), DiagnosticLimits::interactive())
            .map_err(|_| "canonical capture decoding failed")?;
        let context = capture.context();
        if context.device_id != device_id
            || context.capability != capability.board.identity()
            || context.config_digest != Digest::ZERO
            || snapshot.boot_id != Some(context.boot_id.as_bytes())
            || identity.capability.identity().ok() != Some(context.capability)
            || snapshot.waveform_capture_id != Some(capture.capture_id().as_bytes())
        {
            return Err("capture context does not match live device authority");
        }
        if snapshot
            .history
            .last()
            .is_none_or(|latest| latest.frequency_hz != context.clock_frequency_hz)
        {
            return Err("capture clock frequency does not match signed heartbeat evidence");
        }
        for channel in capture.channels() {
            let resource = capability
                .board
                .resource(channel.resource)
                .ok_or("capture names a resource absent from the board capability")?;
            if !resource
                .graph_accesses()
                .iter()
                .any(|access| access.access == GraphResourceAccess::StableBooleanInput)
            {
                return Err("capture resource lacks stable Boolean input authority");
            }
        }
        Ok(())
    }

    fn apply_waveform_transfer(&mut self, waveform: &WorkerWaveformDocument) {
        let connection_id = waveform.connection_id;
        let generation = waveform.generation;
        match self.validate_waveform_transfer(waveform) {
            Ok(()) => {
                let capture_id =
                    decode_digital_capture(waveform.record(), DiagnosticLimits::interactive())
                        .expect("validated waveform transfer remains canonical")
                        .capture_id()
                        .as_bytes();
                self.waveforms.insert(
                    connection_id,
                    ConnectedWaveformView {
                        connection_id,
                        generation,
                        capture_id,
                        record: waveform.record().to_vec(),
                    },
                );
            }
            Err(error) => self.push_diagnostic(format!(
                "connection {connection_id}: waveform transfer rejected: {error}"
            )),
        }
    }

    fn apply_snapshot(&mut self, snapshot: DeviceSessionSnapshot) {
        if self
            .capabilities
            .get(&snapshot.connection_id)
            .is_some_and(|capability| {
                capability.generation != snapshot.generation
                    || snapshot.capability_phase != CapabilityDownloadPhaseSnapshot::Complete
                    || snapshot
                        .capability_identity
                        .and_then(|identity| identity.identity().ok())
                        != Some(capability.board.identity())
            })
        {
            self.capabilities.remove(&snapshot.connection_id);
        }
        if self
            .telemetry
            .get(&snapshot.connection_id)
            .is_some_and(|telemetry| {
                let subscription = decode_telemetry_subscribe(
                    &telemetry.subscription,
                    DiagnosticTransportLimits::native_control(),
                )
                .ok();
                telemetry.generation != snapshot.generation
                    || snapshot.telemetry_phase != Some(TelemetryPhaseSnapshot::Active)
                    || subscription.is_none_or(|subscription| {
                        snapshot.telemetry_subscription_id
                            != Some(subscription.subscription_id().get())
                            || snapshot.telemetry_subscription_digest
                                != Some(subscription.digest().0)
                    })
            })
        {
            self.telemetry.remove(&snapshot.connection_id);
        }
        if self
            .waveforms
            .get(&snapshot.connection_id)
            .is_some_and(|waveform| {
                waveform.generation != snapshot.generation
                    || snapshot.waveform_capture_id != Some(waveform.capture_id)
            })
        {
            self.waveforms.remove(&snapshot.connection_id);
        }
        self.devices.insert(snapshot.connection_id, snapshot);
    }

    fn apply(&mut self, envelope: WorkerEventEnvelope) {
        if let Err(error) = envelope.validate() {
            if envelope.schema_version == WORKER_SCHEMA_VERSION {
                self.push_diagnostic(format!("control worker snapshot rejected: {error}"));
            } else {
                self.lifecycle = SupervisorLifecycle::Failed(
                    "control worker uses an incompatible message schema".to_owned(),
                );
            }
            return;
        }
        match envelope.event {
            WorkerEvent::Ready { scope_origin } => {
                self.lifecycle = SupervisorLifecycle::Ready { scope_origin };
            }
            WorkerEvent::Snapshot { snapshot } => self.apply_snapshot(*snapshot),
            WorkerEvent::CapabilityDocument { capability } => {
                let connection_id = capability.connection_id;
                let generation = capability.generation;
                let matching_session = self
                    .devices
                    .get(&connection_id)
                    .is_some_and(|snapshot| snapshot.generation == generation);
                if !matching_session {
                    self.push_diagnostic(format!(
                        "connection {connection_id}: stale capability generation {generation} ignored"
                    ));
                    return;
                }
                let Ok(identity) = capability.identity.identity() else {
                    self.push_diagnostic(format!(
                        "connection {connection_id}: capability identity projection failed"
                    ));
                    return;
                };
                match build_board_explorer_snapshot(
                    capability.document(),
                    identity,
                    BoardCapabilityLimits::interactive(),
                ) {
                    Ok(board) => {
                        self.capabilities.insert(
                            connection_id,
                            ConnectedCapabilityView {
                                connection_id,
                                generation,
                                board,
                            },
                        );
                    }
                    Err(error) => self.push_diagnostic(format!(
                        "connection {connection_id}: capability explorer rejected: {error}"
                    )),
                }
            }
            WorkerEvent::WaveformDocument { waveform } => {
                self.apply_waveform_transfer(waveform.as_ref());
            }
            WorkerEvent::TelemetryDocument { telemetry } => {
                self.apply_telemetry_transfer(telemetry.as_ref());
            }
            WorkerEvent::Removed { connection_id } => {
                self.devices.remove(&connection_id);
                self.capabilities.remove(&connection_id);
                self.telemetry.remove(&connection_id);
                self.waveforms.remove(&connection_id);
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
            capabilities: self.capabilities.values().cloned().collect(),
            telemetry: self.telemetry.values().cloned().collect(),
            waveforms: self.waveforms.values().cloned().collect(),
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
