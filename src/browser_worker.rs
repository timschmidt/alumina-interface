//! Dedicated browser control worker and rendering-realm supervisor.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use alumina_board::{
    DiagnosticObservationKind, DigitalCaptureConfigureFlags, DigitalCaptureSourceKind,
    DigitalCaptureTriggerSet, ResourceId, SupportLevel,
};
use alumina_capability::{
    BoardCapabilityLimits, DiagnosticOverviewCapability, decode_board_capability,
    decode_resource_id,
};
use alumina_clock::{ClockFlags, ClockObservation};
use alumina_diagnostics::transport::{
    DiagnosticTransportLimits, SubscriptionId, TelemetrySubscribeFlags, TelemetrySubscribeRequest,
    WaveformConfigureFlags, WaveformConfigureRequest, decode_telemetry_event,
    decode_telemetry_subscribe, encode_telemetry_subscribe, encode_waveform_configure,
    telemetry_event_encoded_len, telemetry_subscribe_encoded_len, waveform_configure_encoded_len,
};
use alumina_diagnostics::{
    CaptureId, DIGITAL_CAPTURE_VERSION, DiagnosticContext, DiagnosticLimits,
    DigitalAcquisitionSource, DigitalCaptureView, DigitalTriggerCondition,
    RESOURCE_OVERVIEW_VERSION, decode_digital_capture, digital_capture_encoded_len,
    resource_overview_encoded_len,
};
use alumina_interface_client::capability::{CapabilityDownloadMachine, CapabilityDownloadPhase};
use alumina_interface_client::clock::{ClockProbeError, DeviceClockModel, MonotonicTimeBounds};
use alumina_interface_client::configuration::ConfigurationStatusModel;
use alumina_interface_client::diagnostics::{
    TelemetrySubscriptionMachine, WaveformCaptureMachine, WaveformClientPhase,
};
use alumina_interface_client::health::RuntimeHealthModel;
use alumina_interface_client::http::{AuthenticatedHttpSession, DeviceIdentity};
use alumina_interface_client::wasm::{
    BrowserCapabilityError, BrowserClockError, BrowserConfigurationError, BrowserFetchError,
    BrowserHealthError, BrowserTelemetryError, BrowserWaveformError, DeviceOrigin,
    drive_capability_step_in_worker, drive_clock_probe_in_worker,
    drive_configuration_status_in_worker, drive_runtime_health_in_worker,
    drive_telemetry_step_in_worker, drive_waveform_step_in_worker, fetch_device_identity_in_worker,
    fetch_pending_request_in_worker, open_authenticated_session_in_worker, worker_origin,
};
use alumina_interface_client::worker::{
    CapabilityDownloadPhaseSnapshot, CapabilityIdentitySnapshot, ClockEstimateSnapshot,
    ClockHistoryRecord, ConfigurationWorkerSnapshot, DeviceConnectionRequest,
    DeviceIdentitySnapshot, DeviceSessionPhase, DeviceSessionSnapshot, MAXIMUM_CLOCK_HISTORY,
    MAXIMUM_WORKER_DIAGNOSTIC_BYTES, RuntimeHealthWorkerSnapshot, TelemetryPhaseSnapshot,
    WORKER_SCHEMA_VERSION, WorkerCachedJobPhaseSnapshot, WorkerCachedJobRequest,
    WorkerCachedJobSnapshot, WorkerCapabilityDocument, WorkerCommand, WorkerCommandEnvelope,
    WorkerEvent, WorkerEventEnvelope, WorkerJobExecutionMode, WorkerTelemetryDocument,
    WorkerWaveformDocument, WorkerWaveformRequest,
};
use alumina_interface_core::board_explorer::{
    BoardExplorerSnapshot, build_board_explorer_snapshot,
};
use alumina_job::JobCommitId;
use alumina_protocol::{DeviceCycle, Digest};
use alumina_storage::sha256;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    DedicatedWorkerGlobalScope, ErrorEvent, MessageEvent, Worker, WorkerGlobalScope, WorkerOptions,
    WorkerType,
};

use crate::distributed_schedule::{ParticipantStartInput, ParticipantStartTiming};
use crate::live_job::{LiveCachedJob, LiveJobOperation, LiveJobParticipantBinding};

const CONTROL_WORKER_URL: &str = "alumina-worker.js";
const WORKER_TICK_MS: i32 = 100;
const MAXIMUM_COMMAND_JSON_BYTES: usize = 48 * 1024 * 1024;
const MAXIMUM_UI_DIAGNOSTICS: usize = 16;
const MAXIMUM_RETRY_MS: u32 = 30_000;
const CAPABILITY_RANGES_PER_HEARTBEAT: usize = 4;
const WAVEFORM_OPERATIONS_PER_HEARTBEAT: usize = 8;
const TELEMETRY_RESOURCE_LIMIT: usize = 4;
const JOB_START_LEAD_NS: u64 = 5_000_000_000;
const JOB_CONFIRMATION_LEAD_SECONDS: u64 = 3;
const JOB_ABORT_GUARD_LEAD_SECONDS: u64 = 1;
const JOB_LEASE_SLACK_SECONDS: u64 = 30;

struct PassiveTelemetrySelection {
    resources: Vec<ResourceId>,
    encoded_request_bytes: usize,
    encoded_event_bytes: usize,
    nominal_period_micros: u32,
}

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
    configuration: ConfigurationStatusModel,
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
    configuration_consecutive_failures: u32,
    configuration_last_error: Option<String>,
    next_configuration_attempt_ms: f64,
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
            configuration: ConfigurationStatusModel::new(),
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
            configuration_consecutive_failures: 0,
            configuration_last_error: None,
            next_configuration_attempt_ms: 0.0,
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
            configuration_availability: self.configuration.availability().into(),
            configuration: self
                .configuration
                .latest()
                .map(ConfigurationWorkerSnapshot::from_status),
            configuration_consecutive_failures: self.configuration_consecutive_failures,
            configuration_last_error: self.configuration_last_error.clone(),
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

    fn configuration_due(&self, now_ms: f64) -> bool {
        self.next_configuration_attempt_ms <= now_ms
    }

    fn schedule_configuration_attempt(&mut self, now_ms: f64) {
        self.next_configuration_attempt_ms =
            now_ms + f64::from(self.sampling.runtime_health_interval_ms);
    }

    fn record_configuration_success(&mut self) {
        self.configuration_consecutive_failures = 0;
        self.configuration_last_error = None;
    }

    fn record_configuration_failure(&mut self, error: &str) {
        self.configuration_consecutive_failures =
            self.configuration_consecutive_failures.saturating_add(1);
        self.configuration_last_error = Some(bounded_diagnostic(error));
    }

    fn reset_configuration_for_new_boot(&mut self) {
        self.configuration.reset();
        self.configuration_consecutive_failures = 0;
        self.configuration_last_error = None;
        self.next_configuration_attempt_ms = 0.0;
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
        let Some(selection) = select_passive_telemetry(capability.diagnostic_overview())? else {
            return Ok(false);
        };

        let boot_id = self
            .clock
            .boot_id()
            .ok_or_else(|| "authenticated boot identity is unavailable".to_owned())?;
        let latest = self
            .history
            .back()
            .ok_or_else(|| "authenticated clock evidence is unavailable".to_owned())?;
        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(selection.encoded_request_bytes)
            .map_err(|_| "telemetry subscription allocation failed".to_owned())?;
        encoded.resize(selection.encoded_request_bytes, 0);
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
                minimum_period_cycles: cycles_for_micros_ceil(
                    latest.frequency_hz,
                    selection.nominal_period_micros,
                )
                .ok_or_else(|| "telemetry cadence does not fit device cycles".to_owned())?
                .max(1),
                maximum_event_bytes: u32::try_from(selection.encoded_event_bytes)
                    .map_err(|_| "telemetry event length does not fit protocol".to_owned())?,
                resources: &selection.resources,
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

        let capture_capability = capability.digital_capture();
        if !capture_capability.is_implemented() {
            return Err("this firmware image composes no digital-capture provider".to_owned());
        }
        if capture_capability.schema_version() != DIGITAL_CAPTURE_VERSION {
            return Err("firmware digital-capture schema is not supported by this UI".to_owned());
        }
        let (capture_configure_flags, capture_trigger_kinds) =
            capture_capability.configure_policy();
        if !capture_configure_flags.contains(DigitalCaptureConfigureFlags::EDGE_TIMESTAMPS)
            || !capture_trigger_kinds.contains(DigitalCaptureTriggerSet::IMMEDIATE)
        {
            return Err(
                "firmware digital-capture policy does not admit immediate edge timestamps"
                    .to_owned(),
            );
        }
        let (maximum_channels, maximum_transitions) = capture_capability.shape_limits();
        if request.channels.len() > usize::from(maximum_channels) {
            return Err("waveform selection exceeds the firmware channel budget".to_owned());
        }

        let mut channels = Vec::new();
        channels
            .try_reserve_exact(request.channels.len())
            .map_err(|_| "waveform channel allocation failed".to_owned())?;
        let mut uses_software = false;
        for encoded in &request.channels {
            let resource = decode_resource_id(encoded)
                .map_err(|_| "waveform resource selector is invalid".to_owned())?;
            let capture_source = capture_capability
                .resources()
                .find(|candidate| {
                    candidate.resource == resource && candidate.support >= SupportLevel::Compiles
                })
                .map(|candidate| candidate.source);
            let Some(capture_source) = capture_source else {
                return Err(format!(
                    "resource {resource:?} is not admitted by the digital-capture provider"
                ));
            };
            uses_software |= capture_source == DigitalCaptureSourceKind::Software;
            channels.push(resource);
        }
        let mut waveform_flags = DigitalCaptureConfigureFlags::EDGE_TIMESTAMPS;
        if uses_software {
            waveform_flags |= DigitalCaptureConfigureFlags::ALLOW_SOFTWARE;
        }
        if waveform_flags & !capture_configure_flags.0 != 0 {
            return Err("waveform selection requires an unavailable acquisition path".to_owned());
        }

        let boot_id = self
            .clock
            .boot_id()
            .ok_or_else(|| "authenticated boot identity is unavailable".to_owned())?;
        let latest = self
            .history
            .back()
            .ok_or_else(|| "authenticated clock evidence is unavailable".to_owned())?;
        let (_, record_budget, capability_chunk_bytes) = capture_capability.byte_limits();
        let (_, maximum_duration_micros, arm_horizon_micros) = capture_capability.timing_micros();
        let maximum_duration =
            cycles_for_micros_floor(latest.frequency_hz, maximum_duration_micros)
                .ok_or_else(|| "clock frequency cannot bound capture duration".to_owned())?;
        if request.duration_cycles > maximum_duration {
            return Err("waveform duration exceeds the firmware capability".to_owned());
        }
        let arm_horizon_cycles =
            cycles_for_micros_floor(latest.frequency_hz, arm_horizon_micros)
                .ok_or_else(|| "waveform arm horizon does not fit device cycles".to_owned())?;
        let latest_trigger_cycle = latest
            .transmit_cycle
            .checked_add(arm_horizon_cycles)
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
        let (configure_budget, _, _) = capture_capability.byte_limits();
        if u32::try_from(encoded_len)
            .ok()
            .is_none_or(|bytes| bytes > configure_budget)
        {
            return Err("waveform configuration exceeds the firmware byte budget".to_owned());
        }
        let transport_limits = DiagnosticTransportLimits::native_control();
        let transition_capacity =
            maximum_transitions.min(transport_limits.maximum_waveform_transitions);
        let maximum_chunk_bytes =
            capability_chunk_bytes.min(transport_limits.maximum_waveform_chunk_bytes);
        let maximum_record_bytes = digital_capture_encoded_len(
            channels.len(),
            usize::try_from(transition_capacity)
                .map_err(|_| "waveform transition budget does not fit this UI".to_owned())?,
        )
        .map_err(|error| format!("waveform record budget rejected: {error}"))?;
        if transition_capacity == 0
            || maximum_chunk_bytes == 0
            || u32::try_from(maximum_record_bytes)
                .ok()
                .is_none_or(|bytes| {
                    bytes > record_budget || bytes > transport_limits.maximum_waveform_record_bytes
                })
        {
            return Err("waveform record exceeds the shared fixed-memory budget".to_owned());
        }
        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(encoded_len)
            .map_err(|_| "waveform configure allocation failed".to_owned())?;
        encoded.resize(encoded_len, 0);
        let used = encode_waveform_configure(
            &WaveformConfigureRequest {
                capture_id,
                context,
                flags: WaveformConfigureFlags(waveform_flags),
                requested_pretrigger_cycles: 0,
                requested_posttrigger_cycles: request.duration_cycles,
                earliest_trigger_cycle: DeviceCycle(latest.transmit_cycle),
                latest_trigger_cycle: DeviceCycle(latest_trigger_cycle),
                transition_capacity,
                maximum_chunk_bytes,
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

fn select_passive_telemetry(
    overview: DiagnosticOverviewCapability<'_>,
) -> Result<Option<PassiveTelemetrySelection>, String> {
    if !overview.is_implemented() {
        return Ok(None);
    }
    if overview.schema_version() != RESOURCE_OVERVIEW_VERSION {
        return Err("firmware diagnostic overview schema is not supported by this UI".to_owned());
    }
    let resource_limit = usize::from(overview.maximum_resources()).min(TELEMETRY_RESOURCE_LIMIT);
    let mut resources: Vec<_> = overview
        .resources()
        .filter(|resource| {
            resource.observation == DiagnosticObservationKind::StableBooleanInput
                && resource.support >= SupportLevel::Compiles
        })
        .map(|resource| resource.resource)
        .take(resource_limit)
        .collect();
    resources.sort_unstable();
    resources.dedup();
    if resources.is_empty() {
        return Ok(None);
    }

    let overview_bytes = resource_overview_encoded_len(resources.len())
        .map_err(|error| format!("telemetry overview length rejected: {error}"))?;
    let encoded_event_bytes = telemetry_event_encoded_len(overview_bytes)
        .map_err(|error| format!("telemetry event length rejected: {error}"))?;
    let encoded_request_bytes = telemetry_subscribe_encoded_len(resources.len())
        .map_err(|error| format!("telemetry subscribe length rejected: {error}"))?;
    let (request_budget, event_budget) = overview.telemetry_bytes();
    if u32::try_from(encoded_request_bytes)
        .ok()
        .is_none_or(|bytes| bytes > request_budget)
        || u32::try_from(encoded_event_bytes)
            .ok()
            .is_none_or(|bytes| bytes > event_budget)
    {
        return Err("telemetry selection exceeds the firmware capability budget".to_owned());
    }
    Ok(Some(PassiveTelemetrySelection {
        resources,
        encoded_request_bytes,
        encoded_event_bytes,
        nominal_period_micros: overview.timing_micros().0,
    }))
}

fn cycles_for_micros_ceil(frequency_hz: u64, micros: u32) -> Option<u64> {
    let numerator = u128::from(frequency_hz)
        .checked_mul(u128::from(micros))?
        .checked_add(999_999)?;
    u64::try_from(numerator / 1_000_000).ok()
}

fn cycles_for_micros_floor(frequency_hz: u64, micros: u32) -> Option<u64> {
    let numerator = u128::from(frequency_hz).checked_mul(u128::from(micros))?;
    u64::try_from(numerator / 1_000_000).ok()
}

const fn acquisition_source(source: DigitalAcquisitionSource) -> Option<DigitalCaptureSourceKind> {
    match source {
        DigitalAcquisitionSource::Simulated => Some(DigitalCaptureSourceKind::Simulated),
        DigitalAcquisitionSource::Rmt => Some(DigitalCaptureSourceKind::Rmt),
        DigitalAcquisitionSource::Pcnt => Some(DigitalCaptureSourceKind::Pcnt),
        DigitalAcquisitionSource::Dma => Some(DigitalCaptureSourceKind::Dma),
        DigitalAcquisitionSource::Software => Some(DigitalCaptureSourceKind::Software),
        DigitalAcquisitionSource::ExternalAnalyzer => None,
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
        snapshot: Box<DeviceSessionSnapshot>,
    },
}

enum JobEntry {
    Idle(Box<LiveCachedJob>),
    Busy {
        job_id: u64,
        snapshot: Box<WorkerCachedJobSnapshot>,
    },
}

impl JobEntry {
    fn snapshot(&self) -> WorkerCachedJobSnapshot {
        match self {
            Self::Idle(job) => job.snapshot(),
            Self::Busy { snapshot, .. } => snapshot.as_ref().clone(),
        }
    }

    fn binding(&self, connection_id: u64) -> bool {
        match self {
            Self::Idle(job) => job.binding(connection_id).is_some(),
            Self::Busy { snapshot, .. } => snapshot
                .participants
                .iter()
                .any(|participant| participant.connection_id == connection_id),
        }
    }
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
    job: Option<JobEntry>,
    pending_start_job_id: Option<u64>,
}

impl ControlWorkerRuntime {
    fn new(scope: DedicatedWorkerGlobalScope) -> Self {
        Self {
            scope,
            devices: BTreeMap::new(),
            next_generation: 1,
            job: None,
            pending_start_job_id: None,
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
    let on_tick = Closure::<dyn FnMut()>::new(move || {
        // A staged job must make bounded forward progress even when heartbeat,
        // health, and telemetry intervals coincide with every worker tick.
        launch_job_step(&tick_runtime);
        launch_due_devices(&tick_runtime);
    });
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
        reject_command(runtime, connection_id, &error.to_string());
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
        WorkerCommand::StageCachedJob { request } => stage_cached_job(runtime, *request),
        WorkerCommand::StartCachedJob { job_id } => start_cached_job(runtime, job_id),
        WorkerCommand::StopCachedJob { job_id } => stop_cached_job(runtime, job_id),
        WorkerCommand::ClearCachedJob { job_id } => clear_cached_job(runtime, job_id),
        WorkerCommand::Disconnect { connection_id } => disconnect_device(runtime, connection_id),
    }
}

const fn command_connection_id(command: &WorkerCommand) -> Option<u64> {
    match command {
        WorkerCommand::Configure { request } => Some(request.connection_id),
        WorkerCommand::CaptureWaveform { request } => Some(request.connection_id),
        WorkerCommand::ProbeNow { connection_id } | WorkerCommand::Disconnect { connection_id } => {
            Some(*connection_id)
        }
        WorkerCommand::StageCachedJob { .. }
        | WorkerCommand::StartCachedJob { .. }
        | WorkerCommand::StopCachedJob { .. }
        | WorkerCommand::ClearCachedJob { .. } => None,
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

fn stage_cached_job(runtime: &SharedWorkerRuntime, request: WorkerCachedJobRequest) {
    let job_id = request.job_id;
    let job = match LiveCachedJob::try_new(request) {
        Ok(job) => job,
        Err(error) => {
            reject_command(
                runtime,
                None,
                &format!("cached job {job_id} rejected: {error}"),
            );
            return;
        }
    };
    let result = {
        let mut runtime_ref = runtime.borrow_mut();
        if runtime_ref.job.is_some() {
            Err("clear the retained cached job before staging another".to_owned())
        } else {
            validate_staged_job(&runtime_ref, &job).map(|()| {
                runtime_ref.job = Some(JobEntry::Idle(Box::new(job)));
            })
        }
    };
    if let Err(error) = result {
        reject_command(runtime, None, &format!("cached job {job_id}: {error}"));
        return;
    }
    publish_job_snapshot(runtime);
    launch_job_step(runtime);
}

fn validate_staged_job(runtime: &ControlWorkerRuntime, job: &LiveCachedJob) -> Result<(), String> {
    let bindings: Vec<_> = job.bindings().collect();
    for binding in bindings {
        let snapshot = runtime
            .devices
            .get(&binding.connection_id)
            .map(DeviceEntry::snapshot)
            .ok_or_else(|| format!("connection {} does not exist", binding.connection_id))?;
        validate_compiled_binding(&snapshot, binding)?;
        let identity = snapshot.device_identity.as_ref().ok_or_else(|| {
            format!(
                "connection {} has no stable identity",
                binding.connection_id
            )
        })?;
        match job.execution_mode() {
            WorkerJobExecutionMode::SimulationOnly if !identity.board_id.starts_with("sim-") => {
                return Err(format!(
                    "connection {} is not an explicitly simulated board",
                    binding.connection_id
                ));
            }
            WorkerJobExecutionMode::Hardware if identity.board_id.starts_with("sim-") => {
                return Err(format!(
                    "connection {} is simulated and cannot be staged as hardware",
                    binding.connection_id
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_compiled_binding(
    snapshot: &DeviceSessionSnapshot,
    binding: LiveJobParticipantBinding,
) -> Result<(), String> {
    validate_binding_identity(snapshot, binding)?;
    if snapshot.phase != DeviceSessionPhase::ClockQualified {
        return Err(format!(
            "connection {} does not have a qualified healthy clock",
            binding.connection_id
        ));
    }
    Ok(())
}

fn validate_binding_identity(
    snapshot: &DeviceSessionSnapshot,
    binding: LiveJobParticipantBinding,
) -> Result<(), String> {
    let identity = snapshot
        .device_identity
        .as_ref()
        .ok_or_else(|| "stable device identity is unavailable".to_owned())?;
    let configuration = snapshot
        .configuration
        .ok_or_else(|| "active configuration identity is unavailable".to_owned())?;
    let capability = identity
        .capability
        .identity()
        .map_err(|error| error.to_string())?;
    if snapshot.connection_id != binding.connection_id
        || snapshot.generation != binding.generation
        || identity.device_id != binding.device_id.0
        || snapshot.boot_id != Some(binding.boot_id.as_bytes())
        || capability.digest != binding.capability_digest
        || configuration.active_digest != binding.config_digest.0
        || !configuration.jobs_authorized
    {
        return Err(format!(
            "connection {} no longer matches its compiled device, boot, capability, or configuration",
            binding.connection_id
        ));
    }
    Ok(())
}

fn start_cached_job(runtime: &SharedWorkerRuntime, job_id: u64) {
    let result = {
        let mut runtime_ref = runtime.borrow_mut();
        if runtime_ref.pending_start_job_id.is_some() {
            Err("a cached-job start request is already pending".to_owned())
        } else {
            match runtime_ref.job.as_ref() {
                None => Err("no cached job is retained".to_owned()),
                Some(JobEntry::Busy { .. }) => {
                    Err(format!("cached job {job_id} is busy; retry start"))
                }
                Some(JobEntry::Idle(job)) if job.job_id() != job_id => {
                    Err(format!("cached job {job_id} is not retained"))
                }
                Some(JobEntry::Idle(job)) if job.phase() != WorkerCachedJobPhaseSnapshot::Ready => {
                    Err(format!(
                        "cached job {job_id} is {:?}, not ready",
                        job.phase()
                    ))
                }
                Some(JobEntry::Idle(_)) => {
                    runtime_ref.pending_start_job_id = Some(job_id);
                    Ok(())
                }
            }
        }
    };
    if let Err(error) = result {
        reject_command(runtime, None, &error);
        return;
    }
    launch_job_step(runtime);
}

fn begin_pending_job_start(runtime: &SharedWorkerRuntime) -> Result<bool, String> {
    let mut runtime_ref = runtime.borrow_mut();
    let Some(job_id) = runtime_ref.pending_start_job_id else {
        return Ok(false);
    };
    let Some(entry) = runtime_ref.job.take() else {
        runtime_ref.pending_start_job_id = None;
        return Err("pending cached-job start has no retained job".to_owned());
    };
    let JobEntry::Idle(mut job) = entry else {
        runtime_ref.job = Some(entry);
        return Ok(false);
    };
    if job.job_id() != job_id || job.phase() != WorkerCachedJobPhaseSnapshot::Ready {
        runtime_ref.pending_start_job_id = None;
        runtime_ref.job = Some(JobEntry::Idle(job));
        return Err(format!(
            "pending cached job {job_id} no longer has ready retained state"
        ));
    }
    if job.bindings().any(|binding| {
        !matches!(
            runtime_ref.devices.get(&binding.connection_id),
            Some(DeviceEntry::Idle(_))
        )
    }) {
        runtime_ref.job = Some(JobEntry::Idle(job));
        return Ok(false);
    }

    let result = (|| {
        let now_ui_ns = worker_monotonic_ns(&runtime_ref.scope)
            .ok_or_else(|| "worker monotonic clock is unavailable".to_owned())?;
        let target_ui_ns = now_ui_ns
            .checked_add(JOB_START_LEAD_NS)
            .ok_or_else(|| "future start epoch overflowed".to_owned())?;
        let inputs = participant_start_inputs(&runtime_ref, &job, target_ui_ns)?;
        job.begin_start(now_ui_ns, target_ui_ns, &inputs)
            .map_err(|error| format!("cached job {job_id} start rejected: {error}"))
    })();
    runtime_ref.pending_start_job_id = None;
    if let Err(error) = &result {
        job.mark_fault(error);
    }
    runtime_ref.job = Some(JobEntry::Idle(job));
    result.map(|()| true)
}

fn participant_start_inputs<'a>(
    runtime: &'a ControlWorkerRuntime,
    job: &LiveCachedJob,
    target_ui_ns: u64,
) -> Result<Vec<ParticipantStartInput<'a>>, String> {
    let mut inputs = Vec::new();
    inputs
        .try_reserve_exact(job.bindings().count())
        .map_err(|_| "participant timing allocation failed".to_owned())?;
    for binding in job.bindings() {
        let state = match runtime.devices.get(&binding.connection_id) {
            Some(DeviceEntry::Idle(state)) => state.as_ref(),
            Some(DeviceEntry::Busy { .. }) => {
                return Err(format!(
                    "connection {} is busy; retry deterministic start",
                    binding.connection_id
                ));
            }
            None => {
                return Err(format!(
                    "connection {} no longer exists",
                    binding.connection_id
                ));
            }
        };
        validate_start_eligibility(state, binding, job.execution_mode())?;
        let heartbeat = state.clock.latest_response().ok_or_else(|| {
            format!(
                "connection {} has no accepted heartbeat",
                binding.connection_id
            )
        })?;
        let frequency_hz = heartbeat.frequency_hz;
        let duration_cycles = ceil_product_ratio(
            job.identity().duration_ticks,
            frequency_hz,
            job.identity().global_timebase_hz,
        )
        .ok_or_else(|| "job duration does not fit the device clock".to_owned())?;
        let lease_slack = frequency_hz
            .checked_mul(JOB_LEASE_SLACK_SECONDS)
            .ok_or_else(|| "job lease margin overflowed".to_owned())?;
        let timing = ParticipantStartTiming {
            maximum_uncertainty_cycles: state.sampling.maximum_uncertainty_cycles,
            required_sync_tolerance_cycles: state.sampling.maximum_uncertainty_cycles,
            confirmation_lead_cycles: frequency_hz
                .checked_mul(JOB_CONFIRMATION_LEAD_SECONDS)
                .ok_or_else(|| "confirmation lead overflowed".to_owned())?,
            abort_guard_lead_cycles: frequency_hz
                .checked_mul(JOB_ABORT_GUARD_LEAD_SECONDS)
                .ok_or_else(|| "abort guard lead overflowed".to_owned())?,
            lease_cycles: duration_cycles
                .checked_add(lease_slack)
                .ok_or_else(|| "job lease overflowed".to_owned())?,
            commit_id: job_commit_id(job.job_id(), binding, target_ui_ns)?,
        };
        inputs.push(ParticipantStartInput {
            device_id: binding.device_id,
            clock: &state.clock,
            timing,
        });
    }
    Ok(inputs)
}

fn validate_start_eligibility(
    state: &DeviceState,
    binding: LiveJobParticipantBinding,
    execution_mode: WorkerJobExecutionMode,
) -> Result<(), String> {
    validate_compiled_binding(&state.snapshot(), binding)?;
    if state.session.is_none() || state.clock.boot_id() != Some(binding.boot_id) {
        return Err(format!(
            "connection {} has no current authenticated boot session",
            binding.connection_id
        ));
    }
    let identity = state.identity.as_ref().ok_or_else(|| {
        format!(
            "connection {} has no current device identity",
            binding.connection_id
        )
    })?;
    match execution_mode {
        WorkerJobExecutionMode::SimulationOnly => {
            if !identity.board_id().starts_with("sim-") {
                return Err(format!(
                    "connection {} is not an explicitly simulated board",
                    binding.connection_id
                ));
            }
        }
        WorkerJobExecutionMode::Hardware => {
            if !identity.credential_source().production_armable() {
                return Err(format!(
                    "connection {} lacks a device-stored production credential",
                    binding.connection_id
                ));
            }
            let document = state.capability.document().ok_or_else(|| {
                format!(
                    "connection {} has no complete capability document",
                    binding.connection_id
                )
            })?;
            let capability =
                decode_board_capability(document, BoardCapabilityLimits::interactive())
                    .map_err(|error| format!("board capability rejected: {error:?}"))?;
            if !capability.armable() {
                return Err(format!(
                    "connection {} capability is intentionally non-armable",
                    binding.connection_id
                ));
            }
        }
    }
    Ok(())
}

fn job_commit_id(
    job_id: u64,
    binding: LiveJobParticipantBinding,
    target_ui_ns: u64,
) -> Result<JobCommitId, String> {
    let mut transcript = [0_u8; 40];
    transcript[0..8].copy_from_slice(&job_id.to_le_bytes());
    transcript[8..16].copy_from_slice(&binding.connection_id.to_le_bytes());
    transcript[16..24].copy_from_slice(&target_ui_ns.to_le_bytes());
    transcript[24..40].copy_from_slice(&binding.device_id.0);
    let digest = sha256(&transcript).digest;
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.0[..16]);
    JobCommitId::new(bytes).map_err(|error| format!("commit identity rejected: {error:?}"))
}

fn ceil_product_ratio(value: u64, multiplier: u64, divisor: u64) -> Option<u64> {
    let divisor = u128::from(divisor);
    if divisor == 0 {
        return None;
    }
    let numerator = u128::from(value)
        .checked_mul(u128::from(multiplier))?
        .checked_add(divisor - 1)?;
    u64::try_from(numerator / divisor).ok()
}

fn stop_cached_job(runtime: &SharedWorkerRuntime, job_id: u64) {
    let result = {
        let mut runtime_ref = runtime.borrow_mut();
        if runtime_ref.pending_start_job_id == Some(job_id) {
            runtime_ref.pending_start_job_id = None;
        }
        match runtime_ref.job.as_mut() {
            Some(JobEntry::Idle(job)) if job.job_id() == job_id => job
                .request_stop()
                .map_err(|error| format!("cached job {job_id} cannot stop: {error}")),
            Some(JobEntry::Busy { job_id: active, .. }) if *active == job_id => {
                Err(format!("cached job {job_id} is busy; retry stop"))
            }
            Some(_) => Err(format!("cached job {job_id} is not retained")),
            None => Err("no cached job is retained".to_owned()),
        }
    };
    if let Err(error) = result {
        reject_command(runtime, None, &error);
        return;
    }
    publish_job_snapshot(runtime);
    launch_job_step(runtime);
}

fn clear_cached_job(runtime: &SharedWorkerRuntime, job_id: u64) {
    let removed = {
        let mut runtime_ref = runtime.borrow_mut();
        let removable = matches!(
            runtime_ref.job.as_ref(),
            Some(JobEntry::Idle(job)) if job.job_id() == job_id && job.terminal()
        );
        if removable {
            runtime_ref.job = None;
            runtime_ref.pending_start_job_id = None;
        }
        removable
    };
    if removed {
        emit_worker_event(runtime, WorkerEvent::JobRemoved { job_id });
    } else {
        reject_command(
            runtime,
            None,
            &format!("cached job {job_id} is absent, busy, or nonterminal"),
        );
    }
}

fn configure_device(runtime: &SharedWorkerRuntime, request: DeviceConnectionRequest) {
    let connection_id = request.connection_id;
    if runtime
        .borrow()
        .job
        .as_ref()
        .is_some_and(|job| job.binding(connection_id))
    {
        reject_command(
            runtime,
            Some(connection_id),
            "clear the retained cached job before replacing a participant connection",
        );
        return;
    }
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
    if runtime
        .borrow()
        .job
        .as_ref()
        .is_some_and(|job| job.binding(connection_id))
    {
        reject_command(
            runtime,
            Some(connection_id),
            "clear the retained cached job before disconnecting a participant",
        );
        return;
    }
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
        let pending_start = runtime_ref.pending_start_job_id.is_some();
        let due = runtime_ref
            .devices
            .iter()
            .filter_map(|(connection_id, entry)| {
                if pending_start
                    && runtime_ref
                        .job
                        .as_ref()
                        .is_some_and(|job| job.binding(*connection_id))
                {
                    return None;
                }
                match entry {
                    DeviceEntry::Idle(state) if state.next_attempt_ms <= now_ms => {
                        Some(*connection_id)
                    }
                    _ => None,
                }
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

fn launch_job_step(runtime: &SharedWorkerRuntime) {
    match begin_pending_job_start(runtime) {
        Ok(true) => publish_job_snapshot(runtime),
        Ok(false) => {}
        Err(error) => {
            publish_job_snapshot(runtime);
            reject_command(runtime, None, &format!("cached job start faulted: {error}"));
            return;
        }
    }
    match take_live_job_task(runtime) {
        Ok(Some((scope, job, state, operation))) => {
            let task_runtime = Rc::clone(runtime);
            spawn_local(async move {
                let connection_id = operation.binding.connection_id;
                let generation = operation.binding.generation;
                let job_id = job.job_id();
                let (job, state) = perform_live_job_operation(&scope, job, state, operation).await;
                finish_live_job_step(&task_runtime, job_id, connection_id, generation, job, state);
            });
        }
        Ok(None) => {}
        Err(error) => {
            publish_job_snapshot(runtime);
            reject_command(runtime, None, &format!("cached job faulted: {error}"));
        }
    }
}

fn take_live_job_task(
    runtime: &SharedWorkerRuntime,
) -> Result<
    Option<(
        DedicatedWorkerGlobalScope,
        LiveCachedJob,
        DeviceState,
        LiveJobOperation,
    )>,
    String,
> {
    let mut runtime_ref = runtime.borrow_mut();
    let Some(entry) = runtime_ref.job.take() else {
        return Ok(None);
    };
    let JobEntry::Idle(mut job) = entry else {
        runtime_ref.job = Some(entry);
        return Ok(None);
    };
    if job.terminal() {
        runtime_ref.job = Some(JobEntry::Idle(job));
        return Ok(None);
    }

    let bindings: Vec<_> = job.bindings().collect();
    for binding in bindings {
        let Some(DeviceEntry::Idle(state)) = runtime_ref.devices.get(&binding.connection_id) else {
            runtime_ref.job = Some(JobEntry::Idle(job));
            return Ok(None);
        };
        if state.session.is_none() || state.phase != DeviceSessionPhase::ClockQualified {
            runtime_ref.job = Some(JobEntry::Idle(job));
            return Ok(None);
        }
        if let Err(error) = validate_binding_identity(&state.snapshot(), binding) {
            job.mark_fault(&error);
            runtime_ref.job = Some(JobEntry::Idle(job));
            return Err(error);
        }
    }

    let operation = match prepare_next_job_operation(&runtime_ref, &mut job) {
        Ok(Some(operation)) => operation,
        Ok(None) => {
            runtime_ref.job = Some(JobEntry::Idle(job));
            return Ok(None);
        }
        Err(error) => {
            job.mark_fault(&error);
            runtime_ref.job = Some(JobEntry::Idle(job));
            return Err(error);
        }
    };
    let connection_id = operation.binding.connection_id;
    let Some(device_entry) = runtime_ref.devices.remove(&connection_id) else {
        let _ = job.abandon_pending(connection_id);
        job.mark_fault("compiled participant connection disappeared");
        runtime_ref.job = Some(JobEntry::Idle(job));
        return Err("compiled participant connection disappeared".to_owned());
    };
    let DeviceEntry::Idle(state) = device_entry else {
        runtime_ref.devices.insert(connection_id, device_entry);
        let _ = job.abandon_pending(connection_id);
        runtime_ref.job = Some(JobEntry::Idle(job));
        return Ok(None);
    };
    if let Err(error) = validate_binding_identity(&state.snapshot(), operation.binding) {
        let _ = job.abandon_pending(connection_id);
        job.mark_fault(&error);
        runtime_ref
            .devices
            .insert(connection_id, DeviceEntry::Idle(state));
        runtime_ref.job = Some(JobEntry::Idle(job));
        return Err(error);
    }

    let scope = runtime_ref.scope.clone();
    let job_id = job.job_id();
    let job_snapshot = job.snapshot();
    let generation = state.generation;
    let device_snapshot = state.snapshot();
    runtime_ref.job = Some(JobEntry::Busy {
        job_id,
        snapshot: Box::new(job_snapshot),
    });
    runtime_ref.devices.insert(
        connection_id,
        DeviceEntry::Busy {
            generation,
            snapshot: Box::new(device_snapshot),
        },
    );
    Ok(Some((scope, *job, *state, operation)))
}

fn prepare_next_job_operation(
    runtime: &ControlWorkerRuntime,
    job: &mut LiveCachedJob,
) -> Result<Option<LiveJobOperation>, String> {
    if let Some(operation) = job.next_operation().map_err(|error| error.to_string())? {
        return Ok(Some(operation));
    }
    match job.phase() {
        WorkerCachedJobPhaseSnapshot::Installed => {
            let now_ui_ns = worker_monotonic_ns(&runtime.scope)
                .ok_or_else(|| "worker monotonic clock is unavailable".to_owned())?;
            let target_ui_ns = job
                .target_ui_ns()
                .ok_or_else(|| "installed job has no shared start epoch".to_owned())?;
            let inputs = participant_start_inputs(runtime, job, target_ui_ns)?;
            job.begin_confirmation(now_ui_ns, &inputs)
                .map_err(|error| error.to_string())?;
        }
        WorkerCachedJobPhaseSnapshot::Confirmed | WorkerCachedJobPhaseSnapshot::Irrevocable => job
            .begin_status_round()
            .map_err(|error| error.to_string())?,
        _ => return Ok(None),
    }
    job.next_operation().map_err(|error| error.to_string())
}

async fn perform_live_job_operation(
    scope: &DedicatedWorkerGlobalScope,
    mut job: LiveCachedJob,
    mut state: DeviceState,
    operation: LiveJobOperation,
) -> (LiveCachedJob, DeviceState) {
    let connection_id = operation.binding.connection_id;
    let Some(session) = state.session.as_mut() else {
        let _ = job.abandon_pending(connection_id);
        job.record_failure("authenticated participant session is unavailable");
        return (job, state);
    };
    let request = match session.begin_request_for_config(
        operation.operation,
        &operation.body,
        operation.frame_config_digest,
        &state.secret,
    ) {
        Ok(request) => request,
        Err(error) => {
            let _ = job.abandon_pending(connection_id);
            job.record_failure(&format!("job request construction failed: {error}"));
            return (job, state);
        }
    };
    let worker_scope: &WorkerGlobalScope = scope.as_ref();
    let response = fetch_pending_request_in_worker(
        worker_scope,
        &state.origin,
        session,
        &request,
        &state.secret,
    )
    .await;
    match response {
        Ok(response) => {
            if let Err(error) = job.accept_response(connection_id, &response) {
                job.record_failure(&format!("job response rejected: {error}"));
            }
        }
        Err(error) => {
            let _ = job.abandon_pending(connection_id);
            if job_session_must_reopen(&error) {
                state.session = None;
            }
            job.record_failure(&format!("job fetch failed: {error}"));
        }
    }
    (job, state)
}

fn finish_live_job_step(
    runtime: &SharedWorkerRuntime,
    job_id: u64,
    connection_id: u64,
    generation: u64,
    job: LiveCachedJob,
    state: DeviceState,
) {
    let restored = {
        let mut runtime_ref = runtime.borrow_mut();
        let job_matches = matches!(
            runtime_ref.job.as_ref(),
            Some(JobEntry::Busy { job_id: active, .. }) if *active == job_id
        );
        let device_matches = matches!(
            runtime_ref.devices.get(&connection_id),
            Some(DeviceEntry::Busy { generation: active, .. }) if *active == generation
        );
        if job_matches && device_matches {
            runtime_ref.job = Some(JobEntry::Idle(Box::new(job)));
            runtime_ref
                .devices
                .insert(connection_id, DeviceEntry::Idle(Box::new(state)));
            true
        } else {
            false
        }
    };
    if restored {
        publish_snapshot(runtime, connection_id);
        publish_job_snapshot(runtime);
    }
}

const fn job_session_must_reopen(error: &BrowserFetchError) -> bool {
    matches!(
        error,
        BrowserFetchError::DocumentOrigin
            | BrowserFetchError::Session(_)
            | BrowserFetchError::HttpStatus(_)
            | BrowserFetchError::MissingHeader(_)
            | BrowserFetchError::Media(_)
    )
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
                state.reset_configuration_for_new_boot();
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
            state.reset_configuration_for_new_boot();
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
            probe_configuration_status(&scope, worker_scope, &mut state, now_ms).await;
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
                state.reset_configuration_for_new_boot();
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

async fn probe_configuration_status(
    scope: &DedicatedWorkerGlobalScope,
    worker_scope: &WorkerGlobalScope,
    state: &mut DeviceState,
    fallback_now_ms: f64,
) {
    if state.session.is_none() || !state.configuration_due(fallback_now_ms) {
        return;
    }
    let result = drive_configuration_status_in_worker(
        worker_scope,
        &state.origin,
        state
            .session
            .as_mut()
            .expect("clock probe retained its authenticated session"),
        &mut state.configuration,
        &state.secret,
    )
    .await;
    let sampled_now_ms = worker_now_ms(scope);
    state.schedule_configuration_attempt(if sampled_now_ms.is_finite() {
        sampled_now_ms
    } else {
        fallback_now_ms
    });
    match result {
        Ok(_) => state.record_configuration_success(),
        Err(error) => {
            if configuration_session_must_reopen(&error) {
                state.session = None;
            }
            state.record_configuration_failure(&error.to_string());
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

const fn configuration_session_must_reopen(error: &BrowserConfigurationError) -> bool {
    matches!(
        error,
        BrowserConfigurationError::ConfigurationIdentity
            | BrowserConfigurationError::Session(_)
            | BrowserConfigurationError::Fetch(
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

fn worker_monotonic_ns(scope: &DedicatedWorkerGlobalScope) -> Option<u64> {
    MonotonicTimeBounds::from_milliseconds(worker_now_ms(scope), 1)
        .ok()
        .map(MonotonicTimeBounds::latest_ns)
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

fn publish_job_snapshot(runtime: &SharedWorkerRuntime) {
    let snapshot = runtime.borrow().job.as_ref().map(JobEntry::snapshot);
    if let Some(snapshot) = snapshot {
        emit_worker_event(
            runtime,
            WorkerEvent::JobSnapshot {
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
    /// Complete replacement state for the worker-owned cached job, if any.
    pub job: Option<WorkerCachedJobSnapshot>,
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
    job: Option<WorkerCachedJobSnapshot>,
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
            job: None,
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
                .diagnostic_observations()
                .iter()
                .any(|observation| {
                    observation.observation == DiagnosticObservationKind::StableBooleanInput
                })
            {
                return Err(
                    "telemetry resource lacks passive stable Boolean observation authority",
                );
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
        let provider = capability.board.digital_capture();
        if !provider.is_implemented() || provider.schema_version != DIGITAL_CAPTURE_VERSION {
            return Err("matching digital-capture capability is absent or unsupported");
        }
        if waveform.record().len()
            > usize::try_from(provider.record_bytes)
                .map_err(|_| "digital-capture record budget does not fit this UI")?
            || capture.channel_count() > usize::from(provider.maximum_channels)
            || capture.transition_count()
                > usize::try_from(provider.maximum_transitions)
                    .map_err(|_| "digital-capture transition budget does not fit this UI")?
            || capture.retention().0 > provider.maximum_transitions
        {
            return Err("capture exceeds the immutable fixed-memory capability");
        }
        let (requested_pretrigger, requested_posttrigger) = capture.requested_window_cycles();
        let requested_duration = requested_pretrigger
            .checked_add(requested_posttrigger)
            .ok_or("capture requested duration overflowed")?;
        let maximum_pretrigger = cycles_for_micros_floor(
            context.clock_frequency_hz,
            provider.maximum_pretrigger_micros,
        )
        .ok_or("capture pretrigger capability does not fit device cycles")?;
        let maximum_duration =
            cycles_for_micros_floor(context.clock_frequency_hz, provider.maximum_duration_micros)
                .ok_or("capture duration capability does not fit device cycles")?;
        if requested_pretrigger > maximum_pretrigger || requested_duration > maximum_duration {
            return Err("capture timing exceeds the immutable capability");
        }
        let trigger_bit = match capture.trigger().2 {
            DigitalTriggerCondition::Immediate => DigitalCaptureTriggerSet::IMMEDIATE,
            DigitalTriggerCondition::Rising => DigitalCaptureTriggerSet::RISING,
            DigitalTriggerCondition::Falling => DigitalCaptureTriggerSet::FALLING,
            DigitalTriggerCondition::Either => DigitalCaptureTriggerSet::EITHER,
        };
        if !provider.trigger_kinds.contains(trigger_bit) {
            return Err("capture trigger is absent from the immutable capability");
        }
        for channel in capture.channels() {
            let resource = capability
                .board
                .resource(channel.resource)
                .ok_or("capture names a resource absent from the board capability")?;
            let expected = resource
                .digital_capture()
                .ok_or("capture resource lacks digital acquisition authority")?;
            if acquisition_source(channel.source) != Some(expected.source) {
                return Err("capture acquisition source contradicts the immutable capability");
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
            WorkerEvent::JobSnapshot { snapshot } => self.job = Some(*snapshot),
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
            WorkerEvent::JobRemoved { job_id } => {
                if self.job.as_ref().is_some_and(|job| job.job_id == job_id) {
                    self.job = None;
                }
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
            job: self.job.clone(),
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
