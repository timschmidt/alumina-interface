//! Versioned UI/worker messages and redacted live clock diagnostics.

use std::fmt;

use alumina_capability::{
    BoardCapabilityLimits, CAPABILITY_DOCUMENT_HEADER_BYTES, CapabilityIdentity,
    decode_board_capability, decode_resource_id,
};
use alumina_clock::ClockEstimationPolicy;
use alumina_diagnostics::{DiagnosticLimits, decode_digital_capture};
use alumina_protocol::{DeviceCycle, DeviceId, Digest};
use alumina_runtime::health::{
    RuntimeHealthFlags, RuntimeHealthSnapshot as WireRuntimeHealthSnapshot,
};
use alumina_runtime::stack::{StackDomain, StackWatermarkFlags, StackWatermarkSnapshot};
use serde::{Deserialize, Serialize};

use crate::capability::CapabilityDownloadPhase;
use crate::diagnostics::WaveformClientPhase;
use crate::health::{RuntimeHealthAvailability, RuntimeHealthView};
use crate::http::{DeviceCredentialSource, DeviceIdentity};

/// Exact JSON message schema shared by the browser UI and its control worker.
pub const WORKER_SCHEMA_VERSION: u16 = 4;
/// Maximum clock-history records retained and copied into one UI snapshot.
pub const MAXIMUM_CLOCK_HISTORY: usize = 64;
/// Maximum UTF-8 bytes retained in one worker diagnostic field.
pub const MAXIMUM_WORKER_DIAGNOSTIC_BYTES: usize = 512;

const MAXIMUM_LABEL_BYTES: usize = 64;
const MAXIMUM_ORIGIN_BYTES: usize = 256;
const MAXIMUM_SECRET_BYTES: usize = 256;
const MAXIMUM_CAPTURE_CHANNELS: usize = 4;
const MAXIMUM_CAPTURE_DURATION_CYCLES: u64 = 60_000_000_000;

/// Bounded capability-selected one-shot digital-capture request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerWaveformRequest {
    /// UI-local connection owning the already authenticated device session.
    pub connection_id: u64,
    /// Strictly increasing canonical four-byte resource selectors.
    pub channels: Vec<[u8; 4]>,
    /// Exact immediate-capture duration in device cycles.
    pub duration_cycles: u64,
}

impl WorkerWaveformRequest {
    /// Validates transport bounds and canonical resource ordering before I/O.
    ///
    /// Capability membership is checked separately inside the worker against
    /// the complete document acquired for this exact session generation.
    pub fn validate(&self) -> Result<(), WorkerContractError> {
        if self.connection_id == 0
            || self.channels.is_empty()
            || self.channels.len() > MAXIMUM_CAPTURE_CHANNELS
            || self.duration_cycles == 0
            || self.duration_cycles > MAXIMUM_CAPTURE_DURATION_CYCLES
        {
            return Err(WorkerContractError::WaveformRequest);
        }
        let mut previous = None;
        for encoded in &self.channels {
            let resource =
                decode_resource_id(encoded).map_err(|_| WorkerContractError::WaveformRequest)?;
            if previous.is_some_and(|prior| prior >= resource) {
                return Err(WorkerContractError::WaveformRequest);
            }
            previous = Some(resource);
        }
        Ok(())
    }
}

/// Policy for causal heartbeat acquisition and exact affine-clock admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockSamplingPolicy {
    /// Maximum accepted browser/device causal span.
    pub maximum_round_trip_ns: u64,
    /// Maximum device receive-to-transmit work admitted into one sample.
    pub maximum_device_processing_cycles: u64,
    /// Symmetric oscillator envelope around the device-declared frequency.
    pub maximum_drift_ppm: u32,
    /// Maximum age of the newest accepted sample.
    pub maximum_sample_age_ns: u64,
    /// Furthest future UI epoch that may be mapped.
    pub maximum_schedule_horizon_ns: u64,
    /// Closest future UI epoch that may be mapped.
    pub minimum_schedule_lead_ns: u64,
    /// Minimum intersecting causal observations required for a model.
    pub minimum_samples: u8,
    /// Conservative error around each browser monotonic timer read.
    pub maximum_timer_error_ns: u64,
    /// Maximum interval radius allowed in UI clock diagnostics.
    pub maximum_uncertainty_cycles: u64,
    /// Requested interval between automatic worker probes.
    pub heartbeat_interval_ms: u32,
    /// Minimum interval between passive runtime-health polls.
    pub runtime_health_interval_ms: u32,
}

impl ClockSamplingPolicy {
    /// Conservative initial Wi-Fi policy; machine-specific qualification may tighten it.
    pub const CONSERVATIVE_WIFI: Self = Self {
        maximum_round_trip_ns: 250_000_000,
        maximum_device_processing_cycles: 1_000_000,
        maximum_drift_ppm: 250,
        maximum_sample_age_ns: 5_000_000_000,
        maximum_schedule_horizon_ns: 30_000_000_000,
        minimum_schedule_lead_ns: 500_000_000,
        minimum_samples: 4,
        maximum_timer_error_ns: 2_000_000,
        maximum_uncertainty_cycles: 250_000,
        heartbeat_interval_ms: 1_000,
        runtime_health_interval_ms: 1_000,
    };

    /// Converts the wire policy into the exact estimator policy after validation.
    ///
    /// # Errors
    ///
    /// Rejects invalid estimator bounds, a zero uncertainty budget, or an
    /// automatic probe interval outside 100 ms through 60 seconds, or a
    /// runtime-health interval outside 1 through 60 seconds.
    pub fn estimator(self) -> Result<ClockEstimationPolicy, WorkerContractError> {
        let estimator = ClockEstimationPolicy {
            maximum_round_trip_ns: self.maximum_round_trip_ns,
            maximum_device_processing_cycles: self.maximum_device_processing_cycles,
            maximum_drift_ppm: self.maximum_drift_ppm,
            maximum_sample_age_ns: self.maximum_sample_age_ns,
            maximum_schedule_horizon_ns: self.maximum_schedule_horizon_ns,
            minimum_schedule_lead_ns: self.minimum_schedule_lead_ns,
            minimum_samples: self.minimum_samples,
        };
        estimator
            .validate()
            .map_err(|_| WorkerContractError::ClockPolicy)?;
        if self.maximum_timer_error_ns == 0 || self.maximum_uncertainty_cycles == 0 {
            return Err(WorkerContractError::ClockPolicy);
        }
        if !(100..=60_000).contains(&self.heartbeat_interval_ms) {
            return Err(WorkerContractError::HeartbeatInterval);
        }
        if !(1_000..=60_000).contains(&self.runtime_health_interval_ms) {
            return Err(WorkerContractError::RuntimeHealthInterval);
        }
        Ok(estimator)
    }
}

impl Default for ClockSamplingPolicy {
    fn default() -> Self {
        Self::CONSERVATIVE_WIFI
    }
}

/// User-authorized connection parameters transferred once into the worker.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceConnectionRequest {
    /// UI-local stable connection identity.
    pub connection_id: u64,
    /// Operator-facing device label, never used as machine identity.
    pub label: String,
    /// Canonical path-free HTTP(S) device origin.
    pub origin: String,
    /// HMAC secret. Worker events never echo these bytes.
    pub secret: Vec<u8>,
    /// Explicit heartbeat and affine-clock bounds.
    pub sampling: ClockSamplingPolicy,
}

impl DeviceConnectionRequest {
    /// Validates bounded non-secret fields and clock policy before browser I/O.
    ///
    /// URL canonicalization is additionally enforced by the browser adapter.
    ///
    /// # Errors
    ///
    /// Rejects zero identity, empty/oversized labels, oversized origins,
    /// empty/oversized secrets, or an invalid sampling policy.
    pub fn validate(&self) -> Result<(), WorkerContractError> {
        if self.connection_id == 0 {
            return Err(WorkerContractError::ConnectionIdentity);
        }
        if self.label.is_empty()
            || self.label.len() > MAXIMUM_LABEL_BYTES
            || self.label.trim() != self.label
        {
            return Err(WorkerContractError::Label);
        }
        if self.origin.is_empty() || self.origin.len() > MAXIMUM_ORIGIN_BYTES {
            return Err(WorkerContractError::Origin);
        }
        if self.secret.is_empty() || self.secret.len() > MAXIMUM_SECRET_BYTES {
            return Err(WorkerContractError::Secret);
        }
        self.sampling.estimator()?;
        Ok(())
    }
}

impl fmt::Debug for DeviceConnectionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceConnectionRequest")
            .field("connection_id", &self.connection_id)
            .field("label", &self.label)
            .field("origin", &self.origin)
            .field("secret", &"[redacted]")
            .field("sampling", &self.sampling)
            .finish()
    }
}

impl Drop for DeviceConnectionRequest {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

/// One command delivered from the rendering/UI realm to the control worker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerCommand {
    /// Create or atomically replace one device session.
    Configure {
        /// Complete bounded connection request.
        request: DeviceConnectionRequest,
    },
    /// Trigger an immediate clock probe without changing automatic sampling.
    ProbeNow {
        /// UI-local stable connection identity.
        connection_id: u64,
    },
    /// Start one immediate, input-only digital capture on capability-selected resources.
    CaptureWaveform {
        /// Complete bounded capture request; the worker supplies trusted context.
        request: WorkerWaveformRequest,
    },
    /// Remove one session and erase its worker-owned secret/model.
    Disconnect {
        /// UI-local stable connection identity.
        connection_id: u64,
    },
}

/// Versioned command envelope. Unknown schema versions must be rejected.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCommandEnvelope {
    /// Exact [`WORKER_SCHEMA_VERSION`] expected by this source tree.
    pub schema_version: u16,
    /// Typed worker command.
    pub command: WorkerCommand,
}

impl WorkerCommandEnvelope {
    /// Wraps one command in the current exact schema.
    #[must_use]
    pub const fn current(command: WorkerCommand) -> Self {
        Self {
            schema_version: WORKER_SCHEMA_VERSION,
            command,
        }
    }

    /// Enforces the exact worker/UI schema before applying a command.
    ///
    /// # Errors
    ///
    /// Rejects every schema other than the current one.
    pub const fn validate_version(&self) -> Result<(), WorkerContractError> {
        if self.schema_version == WORKER_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(WorkerContractError::SchemaVersion)
        }
    }
}

/// Worker-owned lifecycle for one live device connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceSessionPhase {
    /// Authentication discovery is in flight.
    Connecting,
    /// A session exists but has too few intersecting clock observations.
    Sampling,
    /// The exact causal model currently satisfies the declared policy.
    ClockQualified,
    /// The latest heartbeat reports an unhealthy deadline or safety state.
    DeviceUnhealthy,
    /// The last attempt failed and automatic retry remains scheduled.
    RetryWaiting,
}

/// Conservative clock interval projected at a browser-worker observation time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockEstimateSnapshot {
    /// Browser-worker monotonic instant mapped by the estimator.
    pub ui_ns: u64,
    /// Earliest possible local device cycle at `ui_ns`.
    pub earliest_cycle: u64,
    /// Integer representative midpoint; never a substitute for the bounds.
    pub midpoint_cycle: u64,
    /// Latest possible local device cycle at `ui_ns`.
    pub latest_cycle: u64,
    /// Maximum distance from midpoint to either interval endpoint.
    pub uncertainty_cycles: u64,
}

/// One authenticated causal observation retained for operator diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockHistoryRecord {
    /// Unique model-local heartbeat identity.
    pub probe_id: u64,
    /// Conservative lower browser send bound.
    pub ui_send_ns: u64,
    /// Conservative upper browser receive bound.
    pub ui_receive_ns: u64,
    /// Complete causal span including timer widening and asymmetric Wi-Fi delay.
    pub causal_span_ns: u64,
    /// Device cycle when the service task accepted the request.
    pub receive_cycle: u64,
    /// Device cycle immediately before the response was emitted.
    pub transmit_cycle: u64,
    /// Device-side processing span in local cycles.
    pub processing_cycles: u64,
    /// Device-declared monotonic frequency.
    pub frequency_hz: u64,
    /// Raw validated heartbeat flags.
    pub flags: u16,
    /// Cumulative real-time deadline misses for this boot.
    pub missed_deadlines: u64,
    /// Remaining bounded real-time command slots.
    pub command_queue_free: u32,
    /// Current service-to-real-time work queue depth.
    pub work_queue_depth: u32,
}

/// Worker-visible support state for the passive runtime-health operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeHealthAvailabilitySnapshot {
    /// No authenticated health result has been accepted in this session.
    #[default]
    Unobserved,
    /// Firmware explicitly returned the canonical unsupported response.
    Unsupported,
    /// A complete independently validated snapshot is retained.
    Available,
}

impl From<RuntimeHealthAvailability> for RuntimeHealthAvailabilitySnapshot {
    fn from(value: RuntimeHealthAvailability) -> Self {
        match value {
            RuntimeHealthAvailability::Unobserved => Self::Unobserved,
            RuntimeHealthAvailability::Unsupported => Self::Unsupported,
            RuntimeHealthAvailability::Available => Self::Available,
        }
    }
}

/// Exact occupancy of one firmware queue, without a derived percentage.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueHealthSnapshot {
    /// Entries currently owned by the queue.
    pub depth: u16,
    /// Fixed queue capacity reported by this firmware image.
    pub capacity: u16,
}

impl QueueHealthSnapshot {
    /// Remaining admission credits after independent validation.
    pub const fn free(self) -> u16 {
        self.capacity.saturating_sub(self.depth)
    }

    fn validate(self) -> Result<(), WorkerContractError> {
        if self.capacity == 0 || self.depth > self.capacity {
            Err(WorkerContractError::RuntimeHealthSnapshot)
        } else {
            Ok(())
        }
    }
}

/// Stable JSON name for the executor that owns a measured stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorStackDomainSnapshot {
    /// Wi-Fi, HTTP, storage, and other service work on core 0.
    ServiceCore,
    /// Deterministic real-time work on core 1.
    RealtimeCore,
}

impl ExecutorStackDomainSnapshot {
    const fn wire(self) -> StackDomain {
        match self {
            Self::ServiceCore => StackDomain::ServiceCore,
            Self::RealtimeCore => StackDomain::RealtimeCore,
        }
    }
}

impl From<StackDomain> for ExecutorStackDomainSnapshot {
    fn from(value: StackDomain) -> Self {
        match value {
            StackDomain::ServiceCore => Self::ServiceCore,
            StackDomain::RealtimeCore => Self::RealtimeCore,
        }
    }
}

/// Exact validated facts for one executor stack measurement epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorStackSnapshot {
    /// Executor domain that owns this allocation.
    pub domain: ExecutorStackDomainSnapshot,
    /// Raw validated ASWM V1 flags.
    pub flags: u16,
    /// Complete linker or HAL allocation including the low exclusion.
    pub allocated_bytes: u32,
    /// Guard and policy bytes deliberately omitted from canary access.
    pub excluded_low_bytes: u32,
    /// Bytes painted below the initialization-time current-SP reserve.
    pub painted_bytes: u32,
    /// Smallest observed free prefix within the monitored allocation.
    pub minimum_headroom_bytes: u32,
    /// Number of bounded scan passes attempted in this epoch.
    pub samples: u32,
    /// Number of scans that reached the then-current low-water mark.
    pub completed_sweeps: u32,
    /// Device cycle at which this partial measurement epoch began.
    pub epoch_cycle: u64,
    /// Device cycle of the newest bounded sample.
    pub sampled_at: u64,
}

impl ExecutorStackSnapshot {
    fn from_wire(snapshot: StackWatermarkSnapshot) -> Self {
        Self {
            domain: snapshot.domain.into(),
            flags: snapshot.flags.0,
            allocated_bytes: snapshot.allocated_bytes,
            excluded_low_bytes: snapshot.excluded_low_bytes,
            painted_bytes: snapshot.painted_bytes,
            minimum_headroom_bytes: snapshot.minimum_headroom_bytes,
            samples: snapshot.samples,
            completed_sweeps: snapshot.completed_sweeps,
            epoch_cycle: snapshot.epoch_cycle.0,
            sampled_at: snapshot.sampled_at.0,
        }
    }

    const fn wire(self) -> StackWatermarkSnapshot {
        StackWatermarkSnapshot {
            domain: self.domain.wire(),
            flags: StackWatermarkFlags(self.flags),
            allocated_bytes: self.allocated_bytes,
            excluded_low_bytes: self.excluded_low_bytes,
            painted_bytes: self.painted_bytes,
            minimum_headroom_bytes: self.minimum_headroom_bytes,
            samples: self.samples,
            completed_sweeps: self.completed_sweeps,
            epoch_cycle: DeviceCycle(self.epoch_cycle),
            sampled_at: DeviceCycle(self.sampled_at),
        }
    }

    /// Bytes covered by the measurement policy after the low exclusion.
    pub const fn monitored_bytes(self) -> u32 {
        self.allocated_bytes.saturating_sub(self.excluded_low_bytes)
    }

    /// Conservative maximum use observed in this partial epoch.
    pub const fn observed_maximum_used_bytes(self) -> u32 {
        self.monitored_bytes()
            .saturating_sub(self.minimum_headroom_bytes)
    }

    /// Upper bytes never painted and therefore deliberately charged as used.
    pub const fn unpainted_bytes(self) -> u32 {
        self.monitored_bytes().saturating_sub(self.painted_bytes)
    }

    /// Whether at least one incremental scan reached the current low-water mark.
    pub const fn has_completed_sweep(self) -> bool {
        self.flags & StackWatermarkFlags::COMPLETE_SWEEP != 0
    }

    /// Whether stack activity before measurement initialization remains unknown.
    pub const fn is_partial_boot_epoch(self) -> bool {
        self.flags & StackWatermarkFlags::PARTIAL_BOOT_EPOCH != 0
    }

    /// Age of this executor observation at the enclosing response cycle.
    pub const fn sample_age_cycles(self, snapshot_cycle: u64) -> u64 {
        snapshot_cycle.saturating_sub(self.sampled_at)
    }
}

/// Versioned JSON projection of one independently validated AHLT V1 response.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHealthWorkerSnapshot {
    /// Service-core cycle at which the firmware formed this response.
    pub snapshot_cycle: u64,
    /// Ordered service-to-real-time command occupancy.
    pub command_queue: QueueHealthSnapshot,
    /// Deterministic real-time work-block occupancy.
    pub work_queue: QueueHealthSnapshot,
    /// Lossy real-time-to-service telemetry occupancy.
    pub telemetry_queue: QueueHealthSnapshot,
    /// Same-response service-core executor measurement.
    pub service_stack: ExecutorStackSnapshot,
    /// Latest real-time-core measurement, if one exists in this boot.
    pub realtime_stack: Option<ExecutorStackSnapshot>,
    /// Whether firmware classed the present real-time report as fresh.
    pub realtime_stack_fresh: bool,
}

impl RuntimeHealthWorkerSnapshot {
    /// Projects a validated native health view into its strict worker schema.
    #[must_use]
    pub fn from_view(view: RuntimeHealthView) -> Self {
        let snapshot = view.snapshot();
        Self {
            snapshot_cycle: snapshot.snapshot_cycle.0,
            command_queue: QueueHealthSnapshot {
                depth: snapshot.command_queue_depth,
                capacity: snapshot.command_queue_capacity,
            },
            work_queue: QueueHealthSnapshot {
                depth: snapshot.work_queue_depth,
                capacity: snapshot.work_queue_capacity,
            },
            telemetry_queue: QueueHealthSnapshot {
                depth: snapshot.telemetry_queue_depth,
                capacity: snapshot.telemetry_queue_capacity,
            },
            service_stack: ExecutorStackSnapshot::from_wire(snapshot.service_stack),
            realtime_stack: view
                .realtime_stack()
                .map(|stack| ExecutorStackSnapshot::from_wire(stack.snapshot())),
            realtime_stack_fresh: view.realtime_stack_fresh(),
        }
    }

    /// Reconstructs and validates the native queue, stack, domain, and freshness rules.
    ///
    /// # Errors
    ///
    /// Rejects a malformed or internally inconsistent worker JSON projection.
    pub fn validate(self) -> Result<(), WorkerContractError> {
        self.command_queue.validate()?;
        self.work_queue.validate()?;
        self.telemetry_queue.validate()?;
        let (realtime_stack, present) = self.realtime_stack.map_or_else(
            || {
                (
                    StackWatermarkSnapshot::unavailable(StackDomain::RealtimeCore),
                    false,
                )
            },
            |stack| (stack.wire(), true),
        );
        let flags = (u8::from(present) * RuntimeHealthFlags::REALTIME_STACK_PRESENT)
            | (u8::from(self.realtime_stack_fresh) * RuntimeHealthFlags::REALTIME_STACK_FRESH);
        WireRuntimeHealthSnapshot {
            flags: RuntimeHealthFlags(flags),
            snapshot_cycle: DeviceCycle(self.snapshot_cycle),
            command_queue_depth: self.command_queue.depth,
            command_queue_capacity: self.command_queue.capacity,
            work_queue_depth: self.work_queue.depth,
            work_queue_capacity: self.work_queue.capacity,
            telemetry_queue_depth: self.telemetry_queue.depth,
            telemetry_queue_capacity: self.telemetry_queue.capacity,
            service_stack: self.service_stack.wire(),
            realtime_stack,
        }
        .validate()
        .map_err(|_| WorkerContractError::RuntimeHealthSnapshot)
    }
}

/// Strict JSON lifecycle for authenticated canonical capability acquisition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDownloadPhaseSnapshot {
    /// No signed range has established an immutable identity yet.
    Discovering,
    /// A stable identity and contiguous prefix have been retained.
    Downloading,
    /// Every byte passed canonical decoding and SHA-256 identity validation.
    Complete,
}

impl From<CapabilityDownloadPhase> for CapabilityDownloadPhaseSnapshot {
    fn from(value: CapabilityDownloadPhase) -> Self {
        match value {
            CapabilityDownloadPhase::Discovering => Self::Discovering,
            CapabilityDownloadPhase::Downloading => Self::Downloading,
            CapabilityDownloadPhase::Complete => Self::Complete,
        }
    }
}

/// JSON-safe exact identity of one canonical board document.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityIdentitySnapshot {
    /// Complete canonical document length.
    pub document_bytes: u32,
    /// SHA-256 over every canonical document byte.
    pub digest: [u8; 32],
}

impl CapabilityIdentitySnapshot {
    /// Projects one native capability identity without changing any bits.
    #[must_use]
    pub const fn from_identity(identity: CapabilityIdentity) -> Self {
        Self {
            document_bytes: identity.byte_len,
            digest: identity.digest.0,
        }
    }

    /// Reconstructs and validates the native identity.
    ///
    /// # Errors
    ///
    /// Rejects a zero digest or a document length outside the browser policy.
    pub fn identity(self) -> Result<CapabilityIdentity, WorkerContractError> {
        let limits = BoardCapabilityLimits::interactive();
        let minimum = u32::try_from(CAPABILITY_DOCUMENT_HEADER_BYTES)
            .expect("fixed capability header length fits u32");
        if self.document_bytes < minimum
            || self.document_bytes > limits.maximum_document_bytes
            || self.digest.iter().all(|byte| *byte == 0)
        {
            return Err(WorkerContractError::CapabilityProgress);
        }
        Ok(CapabilityIdentity {
            byte_len: self.document_bytes,
            digest: alumina_protocol::Digest(self.digest),
        })
    }
}

/// JSON-safe non-secret credential provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSourceSnapshot {
    /// Repository development fallback; never production-armable.
    DevelopmentFallback,
    /// Secret supplied outside source control when the image was built.
    BuildProvisioned,
    /// Unique credential loaded from transactional device storage.
    DeviceStored,
}

impl From<DeviceCredentialSource> for CredentialSourceSnapshot {
    fn from(value: DeviceCredentialSource) -> Self {
        match value {
            DeviceCredentialSource::DevelopmentFallback => Self::DevelopmentFallback,
            DeviceCredentialSource::BuildProvisioned => Self::BuildProvisioned,
            DeviceCredentialSource::DeviceStored => Self::DeviceStored,
        }
    }
}

impl CredentialSourceSnapshot {
    /// Whether this provenance is eligible for production arming.
    pub const fn production_armable(self) -> bool {
        matches!(self, Self::DeviceStored)
    }
}

/// Strict rendering-safe projection of public device identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentitySnapshot {
    /// Stable board package ID claimed by the running image.
    pub board_id: String,
    /// Stable physical or explicitly simulated device identity.
    pub device_id: [u8; 16],
    /// Non-secret credential provenance.
    pub credential_source: CredentialSourceSnapshot,
    /// Exact capability identity claimed by the running image.
    pub capability: CapabilityIdentitySnapshot,
}

impl DeviceIdentitySnapshot {
    /// Projects a validated public response into the worker schema.
    #[must_use]
    pub fn from_identity(identity: &DeviceIdentity) -> Self {
        Self {
            board_id: identity.board_id().to_owned(),
            device_id: identity.device_id().0,
            credential_source: identity.credential_source().into(),
            capability: CapabilityIdentitySnapshot::from_identity(identity.capability()),
        }
    }

    /// Reconstructs the stable device selector after validating every field.
    pub fn validate(&self) -> Result<DeviceId, WorkerContractError> {
        if self.board_id.is_empty()
            || self.board_id.len() > MAXIMUM_LABEL_BYTES
            || !self.board_id.as_bytes().iter().copied().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
            || self.device_id.iter().all(|byte| *byte == 0)
        {
            return Err(WorkerContractError::DeviceIdentity);
        }
        self.capability
            .identity()
            .map_err(|_| WorkerContractError::DeviceIdentity)?;
        Ok(DeviceId(self.device_id))
    }
}

/// Rendering-safe waveform lifecycle retained with one device snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveformCapturePhaseSnapshot {
    /// Canonical configure mutation is pending or being reconciled.
    Configuring,
    /// Configuration is known and an explicit diagnostic arm is next.
    Configured,
    /// Diagnostic acquisition arm is pending or being reconciled.
    Arming,
    /// Input-only acquisition is armed and awaiting completion.
    Armed,
    /// Exact retained ranges are being assembled.
    Downloading,
    /// Complete canonical capture bytes were independently validated.
    Complete,
    /// Capture release is pending or being reconciled.
    Stopping,
    /// Device confirmed capture release.
    Stopped,
}

impl From<WaveformClientPhase> for WaveformCapturePhaseSnapshot {
    fn from(value: WaveformClientPhase) -> Self {
        match value {
            WaveformClientPhase::Configuring | WaveformClientPhase::ReconcilingConfigure => {
                Self::Configuring
            }
            WaveformClientPhase::Configured => Self::Configured,
            WaveformClientPhase::Arming | WaveformClientPhase::ReconcilingArm => Self::Arming,
            WaveformClientPhase::Armed => Self::Armed,
            WaveformClientPhase::Downloading { .. } => Self::Downloading,
            WaveformClientPhase::Complete => Self::Complete,
            WaveformClientPhase::Stopping | WaveformClientPhase::ReconcilingStop => Self::Stopping,
            WaveformClientPhase::Stopped => Self::Stopped,
        }
    }
}

/// One-time credential-free canonical capture transferred from worker to UI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerWaveformDocument {
    /// UI-local connection identity owning the capture.
    pub connection_id: u64,
    /// Exact worker session generation that acquired the bytes.
    pub generation: u64,
    /// Complete canonical `ALMDIG01` record.
    record: Vec<u8>,
}

impl WorkerWaveformDocument {
    /// Constructs and independently validates one worker-to-UI transfer.
    pub fn try_new(
        connection_id: u64,
        generation: u64,
        record: Vec<u8>,
    ) -> Result<Self, WorkerContractError> {
        let transfer = Self {
            connection_id,
            generation,
            record,
        };
        transfer.validate()?;
        Ok(transfer)
    }

    /// Complete immutable canonical capture bytes.
    pub fn record(&self) -> &[u8] {
        &self.record
    }

    /// Revalidates session identity and canonical capture content.
    pub fn validate(&self) -> Result<(), WorkerContractError> {
        if self.connection_id == 0 || self.generation == 0 {
            return Err(WorkerContractError::WaveformDocument);
        }
        let capture = decode_digital_capture(&self.record, DiagnosticLimits::interactive())
            .map_err(|_| WorkerContractError::WaveformDocument)?;
        if capture.context().config_digest != Digest::ZERO {
            return Err(WorkerContractError::WaveformDocument);
        }
        Ok(())
    }
}

/// One-time, credential-free canonical document transferred from worker to UI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCapabilityDocument {
    /// UI-local connection identity owning this document.
    pub connection_id: u64,
    /// Exact worker session generation that acquired the bytes.
    pub generation: u64,
    /// Stable identity repeated separately for bounded preflight.
    pub identity: CapabilityIdentitySnapshot,
    /// Complete canonical `ALMCAP02` bytes.
    document: Vec<u8>,
}

impl WorkerCapabilityDocument {
    /// Constructs and independently validates a one-time worker transfer.
    ///
    /// # Errors
    ///
    /// Rejects zero session identity, mismatched lengths/digests, or any invalid
    /// canonical capability document.
    pub fn try_new(
        connection_id: u64,
        generation: u64,
        identity: CapabilityIdentity,
        document: Vec<u8>,
    ) -> Result<Self, WorkerContractError> {
        let transfer = Self {
            connection_id,
            generation,
            identity: CapabilityIdentitySnapshot::from_identity(identity),
            document,
        };
        transfer.validate()?;
        Ok(transfer)
    }

    /// Complete immutable capability bytes.
    pub fn document(&self) -> &[u8] {
        &self.document
    }

    /// Revalidates session identity and complete canonical document content.
    ///
    /// # Errors
    ///
    /// Rejects malformed or substituted worker JSON before it reaches UI state.
    pub fn validate(&self) -> Result<(), WorkerContractError> {
        if self.connection_id == 0 || self.generation == 0 {
            return Err(WorkerContractError::CapabilityDocument);
        }
        let identity = self
            .identity
            .identity()
            .map_err(|_| WorkerContractError::CapabilityDocument)?;
        if usize::try_from(identity.byte_len).ok() != Some(self.document.len()) {
            return Err(WorkerContractError::CapabilityDocument);
        }
        let capability =
            decode_board_capability(&self.document, BoardCapabilityLimits::interactive())
                .map_err(|_| WorkerContractError::CapabilityDocument)?;
        if capability.identity() != identity {
            return Err(WorkerContractError::CapabilityDocument);
        }
        Ok(())
    }
}

/// Redacted complete UI view of one worker-owned connection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSessionSnapshot {
    /// UI-local stable connection identity.
    pub connection_id: u64,
    /// Operator-facing label.
    pub label: String,
    /// Canonical device origin.
    pub origin: String,
    /// Monotonic worker generation, changed on atomic replacement.
    pub generation: u64,
    /// Current worker-owned lifecycle.
    pub phase: DeviceSessionPhase,
    /// Public boot identity learned from an authenticated heartbeat.
    pub boot_id: Option<[u8; 16]>,
    /// Strict stable public identity reconciled with signed capability data.
    pub device_identity: Option<DeviceIdentitySnapshot>,
    /// Number of intersecting observations admitted by the estimator.
    pub accepted_samples: u32,
    /// Number of authenticated observations rejected by exact clock policy.
    pub rejected_samples: u32,
    /// Consecutive connection/probe failures since the latest success.
    pub consecutive_failures: u32,
    /// Latest qualified current-time interval, if sufficient samples exist.
    pub estimate: Option<ClockEstimateSnapshot>,
    /// Oldest-to-newest bounded history, never containing credentials.
    pub history: Vec<ClockHistoryRecord>,
    /// Latest diagnostic failure text, if any.
    pub last_error: Option<String>,
    /// Support state of the passive runtime-health operation in this session.
    pub runtime_health_availability: RuntimeHealthAvailabilitySnapshot,
    /// Latest valid runtime-health evidence; retained across polling failures.
    pub runtime_health: Option<RuntimeHealthWorkerSnapshot>,
    /// Consecutive passive-health failures since the latest accepted result.
    pub runtime_health_consecutive_failures: u32,
    /// Latest passive-health failure text, independent from clock lifecycle.
    pub runtime_health_last_error: Option<String>,
    /// Current authenticated canonical capability acquisition phase.
    pub capability_phase: CapabilityDownloadPhaseSnapshot,
    /// Contiguous capability bytes retained by the worker.
    pub capability_received_bytes: u32,
    /// Stable complete document identity after the first accepted range.
    pub capability_identity: Option<CapabilityIdentitySnapshot>,
    /// Consecutive capability-range failures since the latest accepted range.
    pub capability_consecutive_failures: u32,
    /// Latest capability acquisition failure, independent from clock/health state.
    pub capability_last_error: Option<String>,
    /// Current input-only diagnostic acquisition lifecycle, when requested.
    pub waveform_phase: Option<WaveformCapturePhaseSnapshot>,
    /// Capture attempt identity retained while a waveform lifecycle exists.
    pub waveform_capture_id: Option<[u8; 16]>,
    /// Contiguous canonical capture bytes retained during range download.
    pub waveform_received_bytes: u32,
    /// Complete canonical capture length once reported by device status.
    pub waveform_total_bytes: u32,
    /// Consecutive waveform-operation failures since the latest accepted response.
    pub waveform_consecutive_failures: u32,
    /// Latest waveform failure, isolated from clock/health/capability state.
    pub waveform_last_error: Option<String>,
}

impl DeviceSessionSnapshot {
    /// Independently validates bounded worker state before rendering it.
    ///
    /// # Errors
    ///
    /// Rejects invalid identity, bounds, diagnostic relationships, or health state.
    pub fn validate(&self) -> Result<(), WorkerContractError> {
        if self.connection_id == 0
            || self.generation == 0
            || self.label.is_empty()
            || self.label.len() > MAXIMUM_LABEL_BYTES
            || self.label.trim() != self.label
            || self.origin.is_empty()
            || self.origin.len() > MAXIMUM_ORIGIN_BYTES
            || self.history.len() > MAXIMUM_CLOCK_HISTORY
            || !diagnostic_is_valid(self.last_error.as_deref())
            || !diagnostic_is_valid(self.runtime_health_last_error.as_deref())
            || !diagnostic_is_valid(self.capability_last_error.as_deref())
            || !diagnostic_is_valid(self.waveform_last_error.as_deref())
            || (self.consecutive_failures == 0) != self.last_error.is_none()
            || (self.runtime_health_consecutive_failures == 0)
                != self.runtime_health_last_error.is_none()
            || (self.capability_consecutive_failures == 0) != self.capability_last_error.is_none()
            || (self.waveform_consecutive_failures == 0) != self.waveform_last_error.is_none()
        {
            return Err(WorkerContractError::DeviceSnapshot);
        }
        if let Some(identity) = &self.device_identity {
            identity.validate()?;
        }
        match (self.runtime_health_availability, self.runtime_health) {
            (RuntimeHealthAvailabilitySnapshot::Available, Some(snapshot)) => {
                snapshot.validate()?;
            }
            (
                RuntimeHealthAvailabilitySnapshot::Unobserved
                | RuntimeHealthAvailabilitySnapshot::Unsupported,
                None,
            ) => {}
            _ => return Err(WorkerContractError::RuntimeHealthSnapshot),
        }
        match (self.capability_phase, self.capability_identity) {
            (CapabilityDownloadPhaseSnapshot::Discovering, None)
                if self.capability_received_bytes == 0 => {}
            (CapabilityDownloadPhaseSnapshot::Downloading, Some(identity)) => {
                let identity = identity.identity()?;
                if self.capability_received_bytes >= identity.byte_len {
                    return Err(WorkerContractError::CapabilityProgress);
                }
            }
            (CapabilityDownloadPhaseSnapshot::Complete, Some(identity)) => {
                let identity = identity.identity()?;
                if self.capability_received_bytes != identity.byte_len
                    || self.capability_consecutive_failures != 0
                {
                    return Err(WorkerContractError::CapabilityProgress);
                }
            }
            _ => return Err(WorkerContractError::CapabilityProgress),
        }
        if let Some(capability) = self.capability_identity {
            let identity = self
                .device_identity
                .as_ref()
                .ok_or(WorkerContractError::DeviceIdentity)?;
            if identity.capability != capability {
                return Err(WorkerContractError::DeviceIdentity);
            }
        }
        match (self.waveform_phase, self.waveform_capture_id) {
            (None, None) if self.waveform_received_bytes == 0 && self.waveform_total_bytes == 0 => {
            }
            (Some(phase), Some(capture_id)) if capture_id.iter().any(|byte| *byte != 0) => {
                if self.boot_id.is_none()
                    || self.device_identity.is_none()
                    || self.capability_phase != CapabilityDownloadPhaseSnapshot::Complete
                {
                    return Err(WorkerContractError::WaveformProgress);
                }
                match phase {
                    WaveformCapturePhaseSnapshot::Downloading => {
                        if self.waveform_total_bytes == 0
                            || self.waveform_received_bytes >= self.waveform_total_bytes
                        {
                            return Err(WorkerContractError::WaveformProgress);
                        }
                    }
                    WaveformCapturePhaseSnapshot::Complete => {
                        if self.waveform_total_bytes == 0
                            || self.waveform_received_bytes != self.waveform_total_bytes
                            || self.waveform_consecutive_failures != 0
                        {
                            return Err(WorkerContractError::WaveformProgress);
                        }
                    }
                    _ => {
                        if self.waveform_received_bytes != 0 || self.waveform_total_bytes != 0 {
                            return Err(WorkerContractError::WaveformProgress);
                        }
                    }
                }
            }
            _ => return Err(WorkerContractError::WaveformProgress),
        }
        Ok(())
    }
}

fn diagnostic_is_valid(diagnostic: Option<&str>) -> bool {
    diagnostic.is_none_or(|message| {
        !message.is_empty()
            && message.len() <= MAXIMUM_WORKER_DIAGNOSTIC_BYTES
            && message.trim() == message
    })
}

/// One event delivered by the control worker to the UI realm.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerEvent {
    /// Worker bootstrap completed and its inherited CORS origin is known.
    Ready {
        /// Actual worker origin used by authenticated requests.
        scope_origin: String,
    },
    /// Complete replacement snapshot for one connection.
    Snapshot {
        /// Redacted worker-owned state.
        snapshot: Box<DeviceSessionSnapshot>,
    },
    /// One complete canonical capability document, emitted once per generation.
    CapabilityDocument {
        /// Independently validated immutable bytes and owning session identity.
        capability: Box<WorkerCapabilityDocument>,
    },
    /// One complete canonical digital capture, emitted once per capture attempt.
    WaveformDocument {
        /// Independently validated retained record and owning session identity.
        waveform: Box<WorkerWaveformDocument>,
    },
    /// A disconnect erased a connection.
    Removed {
        /// UI-local stable connection identity.
        connection_id: u64,
    },
    /// One command was rejected without mutating the named session.
    CommandRejected {
        /// Connection identity when it could be recovered from the command.
        connection_id: Option<u64>,
        /// Bounded diagnostic text containing no credentials.
        message: String,
    },
    /// Worker bootstrap or message transport failed globally.
    Fatal {
        /// Bounded diagnostic text containing no credentials.
        message: String,
    },
}

/// Versioned worker event envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerEventEnvelope {
    /// Exact [`WORKER_SCHEMA_VERSION`] produced by this worker.
    pub schema_version: u16,
    /// Typed worker event.
    pub event: WorkerEvent,
}

impl WorkerEventEnvelope {
    /// Wraps one event in the current exact schema.
    #[must_use]
    pub const fn current(event: WorkerEvent) -> Self {
        Self {
            schema_version: WORKER_SCHEMA_VERSION,
            event,
        }
    }

    /// Enforces the exact worker/UI schema before applying an event.
    ///
    /// # Errors
    ///
    /// Rejects every schema other than the current one.
    pub const fn validate_version(&self) -> Result<(), WorkerContractError> {
        if self.schema_version == WORKER_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(WorkerContractError::SchemaVersion)
        }
    }

    /// Enforces the exact version and independently validates snapshot payloads.
    ///
    /// # Errors
    ///
    /// Rejects a mismatched version or invalid device snapshot.
    pub fn validate(&self) -> Result<(), WorkerContractError> {
        self.validate_version()?;
        if let WorkerEvent::Snapshot { snapshot } = &self.event {
            snapshot.validate()?;
        }
        if let WorkerEvent::CapabilityDocument { capability } = &self.event {
            capability.validate()?;
        }
        if let WorkerEvent::WaveformDocument { waveform } = &self.event {
            waveform.validate()?;
        }
        Ok(())
    }
}

/// Bounded worker command or event contract failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerContractError {
    /// UI and worker were built from different schemas.
    SchemaVersion,
    /// Connection identity used the reserved zero value.
    ConnectionIdentity,
    /// Operator-facing label was empty, padded, or oversized.
    Label,
    /// Device origin text was empty or oversized before URL parsing.
    Origin,
    /// HMAC secret was empty or exceeded the bounded worker allowance.
    Secret,
    /// Exact affine-clock estimator policy was invalid.
    ClockPolicy,
    /// Automatic heartbeat period was outside the supported range.
    HeartbeatInterval,
    /// Passive runtime-health period was outside the supported range.
    RuntimeHealthInterval,
    /// A redacted device snapshot violated worker bounds or state relationships.
    DeviceSnapshot,
    /// Runtime-health JSON violated native queue, stack, domain, or freshness rules.
    RuntimeHealthSnapshot,
    /// Capability acquisition progress or identity relationships were invalid.
    CapabilityProgress,
    /// A one-time capability document transfer failed canonical validation.
    CapabilityDocument,
    /// A bounded capture command was malformed or noncanonical.
    WaveformRequest,
    /// Waveform progress fields contradicted the retained lifecycle.
    WaveformProgress,
    /// A one-time capture transfer failed canonical validation.
    WaveformDocument,
    /// Public device identity projection was malformed or inconsistent.
    DeviceIdentity,
}

impl fmt::Display for WorkerContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaVersion => formatter.write_str("UI/worker schema version mismatch"),
            Self::ConnectionIdentity => formatter.write_str("connection identity must be nonzero"),
            Self::Label => formatter.write_str("device label is not bounded and canonical"),
            Self::Origin => formatter.write_str("device origin is not bounded"),
            Self::Secret => formatter.write_str("device authentication secret is not bounded"),
            Self::ClockPolicy => formatter.write_str("clock sampling policy is invalid"),
            Self::HeartbeatInterval => {
                formatter.write_str("heartbeat interval must be 100 through 60000 ms")
            }
            Self::RuntimeHealthInterval => {
                formatter.write_str("runtime health interval must be 1000 through 60000 ms")
            }
            Self::DeviceSnapshot => formatter.write_str("device snapshot is invalid"),
            Self::RuntimeHealthSnapshot => {
                formatter.write_str("runtime health snapshot is invalid")
            }
            Self::CapabilityProgress => {
                formatter.write_str("capability download progress is invalid")
            }
            Self::CapabilityDocument => {
                formatter.write_str("capability document transfer is invalid")
            }
            Self::WaveformRequest => formatter.write_str("waveform request is invalid"),
            Self::WaveformProgress => formatter.write_str("waveform progress is invalid"),
            Self::WaveformDocument => formatter.write_str("waveform document transfer is invalid"),
            Self::DeviceIdentity => formatter.write_str("device identity snapshot is invalid"),
        }
    }
}

impl std::error::Error for WorkerContractError {}

#[cfg(test)]
mod tests {
    use alumina_board::ResourceId;
    use alumina_capability::{
        MAX_CAPABILITY_CHUNK_BYTES, calculate_identity, encode_resource_id, read_verified_range,
    };
    use alumina_sim::diagnostics::tinybee_diagnostic_fixture_for_context;

    use super::*;

    fn tinybee_document() -> (CapabilityIdentity, Vec<u8>) {
        let identity = calculate_identity(&board_mks_tinybee::PACKAGE).unwrap();
        let mut document = vec![0_u8; usize::try_from(identity.byte_len).unwrap()];
        let mut offset = 0_u32;
        while offset < identity.byte_len {
            let mut chunk = [0_u8; MAX_CAPABILITY_CHUNK_BYTES];
            let read =
                read_verified_range(&board_mks_tinybee::PACKAGE, offset, &mut chunk).unwrap();
            let start = usize::try_from(offset).unwrap();
            let count = usize::from(read.byte_len);
            document[start..start + count].copy_from_slice(&chunk[..count]);
            offset += u32::from(read.byte_len);
        }
        (identity, document)
    }

    fn executor_stack(domain: ExecutorStackDomainSnapshot) -> ExecutorStackSnapshot {
        ExecutorStackSnapshot {
            domain,
            flags: StackWatermarkFlags::INITIALIZED
                | StackWatermarkFlags::PARTIAL_BOOT_EPOCH
                | StackWatermarkFlags::CURRENT_POINTER_BOUND,
            allocated_bytes: 32 * 1_024,
            excluded_low_bytes: 256,
            painted_bytes: 28 * 1_024,
            minimum_headroom_bytes: 20 * 1_024,
            samples: 4,
            completed_sweeps: 0,
            epoch_cycle: 10,
            sampled_at: 190,
        }
    }

    fn runtime_health() -> RuntimeHealthWorkerSnapshot {
        RuntimeHealthWorkerSnapshot {
            snapshot_cycle: 200,
            command_queue: QueueHealthSnapshot {
                depth: 2,
                capacity: 8,
            },
            work_queue: QueueHealthSnapshot {
                depth: 3,
                capacity: 8,
            },
            telemetry_queue: QueueHealthSnapshot {
                depth: 4,
                capacity: 32,
            },
            service_stack: executor_stack(ExecutorStackDomainSnapshot::ServiceCore),
            realtime_stack: Some(executor_stack(ExecutorStackDomainSnapshot::RealtimeCore)),
            realtime_stack_fresh: true,
        }
    }

    fn device_identity(capability: CapabilityIdentity) -> DeviceIdentitySnapshot {
        DeviceIdentitySnapshot {
            board_id: "mks-tinybee-v1".to_owned(),
            device_id: *b"ALUM-SIM:TINYBEE",
            credential_source: CredentialSourceSnapshot::DevelopmentFallback,
            capability: CapabilityIdentitySnapshot::from_identity(capability),
        }
    }

    fn request() -> DeviceConnectionRequest {
        DeviceConnectionRequest {
            connection_id: 7,
            label: "TinyBee bench".to_owned(),
            origin: "http://192.168.4.1".to_owned(),
            secret: b"private test secret".to_vec(),
            sampling: ClockSamplingPolicy::CONSERVATIVE_WIFI,
        }
    }

    #[test]
    fn command_round_trip_is_versioned_and_secret_debug_is_redacted() {
        let command =
            WorkerCommandEnvelope::current(WorkerCommand::Configure { request: request() });
        let json = serde_json::to_vec(&command).unwrap();
        let decoded: WorkerCommandEnvelope = serde_json::from_slice(&json).unwrap();
        assert!(decoded == command);
        assert_eq!(decoded.validate_version(), Ok(()));
        let debug = format!("{decoded:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("private test secret"));
    }

    #[test]
    fn malformed_connection_and_clock_policies_fail_before_io() {
        let mut candidate = request();
        assert_eq!(candidate.validate(), Ok(()));
        candidate.connection_id = 0;
        assert_eq!(
            candidate.validate(),
            Err(WorkerContractError::ConnectionIdentity)
        );
        candidate = request();
        candidate.secret.clear();
        assert_eq!(candidate.validate(), Err(WorkerContractError::Secret));
        candidate = request();
        candidate.sampling.heartbeat_interval_ms = 99;
        assert_eq!(
            candidate.validate(),
            Err(WorkerContractError::HeartbeatInterval)
        );
        candidate = request();
        candidate.sampling.runtime_health_interval_ms = 999;
        assert_eq!(
            candidate.validate(),
            Err(WorkerContractError::RuntimeHealthInterval)
        );
        candidate = request();
        candidate.sampling.maximum_timer_error_ns = 0;
        assert_eq!(candidate.validate(), Err(WorkerContractError::ClockPolicy));
    }

    #[test]
    fn waveform_command_requires_sorted_bounded_canonical_resources() {
        let mut request = WorkerWaveformRequest {
            connection_id: 7,
            channels: [22_u8, 32, 33, 35]
                .map(|gpio| encode_resource_id(ResourceId::Gpio(gpio)))
                .to_vec(),
            duration_cycles: 2_000,
        };
        assert_eq!(request.validate(), Ok(()));
        request.channels.swap(0, 1);
        assert_eq!(
            request.validate(),
            Err(WorkerContractError::WaveformRequest)
        );
        request.channels.sort_unstable();
        request.channels.push(request.channels[3]);
        assert_eq!(
            request.validate(),
            Err(WorkerContractError::WaveformRequest)
        );
        request.channels.truncate(4);
        request.duration_cycles = 0;
        assert_eq!(
            request.validate(),
            Err(WorkerContractError::WaveformRequest)
        );
    }

    #[test]
    fn snapshots_round_trip_without_a_credential_field() {
        let event = WorkerEventEnvelope::current(WorkerEvent::Snapshot {
            snapshot: Box::new(DeviceSessionSnapshot {
                connection_id: 7,
                label: "TinyBee bench".to_owned(),
                origin: "http://192.168.4.1".to_owned(),
                generation: 2,
                phase: DeviceSessionPhase::ClockQualified,
                boot_id: Some([3; 16]),
                device_identity: None,
                accepted_samples: 4,
                rejected_samples: 1,
                consecutive_failures: 0,
                estimate: Some(ClockEstimateSnapshot {
                    ui_ns: 11,
                    earliest_cycle: 12,
                    midpoint_cycle: 13,
                    latest_cycle: 14,
                    uncertainty_cycles: 1,
                }),
                history: vec![ClockHistoryRecord {
                    probe_id: 1,
                    ui_send_ns: 2,
                    ui_receive_ns: 5,
                    causal_span_ns: 3,
                    receive_cycle: 7,
                    transmit_cycle: 8,
                    processing_cycles: 1,
                    frequency_hz: 1_000_000,
                    flags: 7,
                    missed_deadlines: 0,
                    command_queue_free: 3,
                    work_queue_depth: 1,
                }],
                last_error: None,
                runtime_health_availability: RuntimeHealthAvailabilitySnapshot::Available,
                runtime_health: Some(runtime_health()),
                runtime_health_consecutive_failures: 0,
                runtime_health_last_error: None,
                capability_phase: CapabilityDownloadPhaseSnapshot::Discovering,
                capability_received_bytes: 0,
                capability_identity: None,
                capability_consecutive_failures: 0,
                capability_last_error: None,
                waveform_phase: None,
                waveform_capture_id: None,
                waveform_received_bytes: 0,
                waveform_total_bytes: 0,
                waveform_consecutive_failures: 0,
                waveform_last_error: None,
            }),
        });
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("secret"));
        let decoded: WorkerEventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, event);
        assert_eq!(decoded.validate(), Ok(()));
        assert_eq!(WORKER_SCHEMA_VERSION, 4);
    }

    #[test]
    fn health_json_is_revalidated_instead_of_trusted() {
        let valid = runtime_health();
        assert_eq!(valid.validate(), Ok(()));
        assert_eq!(valid.command_queue.free(), 6);
        assert_eq!(valid.service_stack.monitored_bytes(), 32 * 1_024 - 256);
        assert_eq!(
            valid.service_stack.observed_maximum_used_bytes(),
            32 * 1_024 - 256 - 20 * 1_024
        );
        assert_eq!(valid.service_stack.unpainted_bytes(), 3_840);
        assert_eq!(valid.service_stack.sample_age_cycles(200), 10);
        assert!(valid.service_stack.is_partial_boot_epoch());
        assert!(!valid.service_stack.has_completed_sweep());

        let mut overflow = valid;
        overflow.command_queue.depth = overflow.command_queue.capacity + 1;
        assert_eq!(
            overflow.validate(),
            Err(WorkerContractError::RuntimeHealthSnapshot)
        );

        let mut substituted = valid;
        substituted.service_stack.domain = ExecutorStackDomainSnapshot::RealtimeCore;
        assert_eq!(
            substituted.validate(),
            Err(WorkerContractError::RuntimeHealthSnapshot)
        );

        let mut freshness_without_report = valid;
        freshness_without_report.realtime_stack = None;
        assert_eq!(
            freshness_without_report.validate(),
            Err(WorkerContractError::RuntimeHealthSnapshot)
        );
    }

    #[test]
    fn validated_native_health_projects_without_losing_exact_facts() {
        let expected = runtime_health();
        let native = WireRuntimeHealthSnapshot {
            flags: RuntimeHealthFlags(
                RuntimeHealthFlags::REALTIME_STACK_PRESENT
                    | RuntimeHealthFlags::REALTIME_STACK_FRESH,
            ),
            snapshot_cycle: DeviceCycle(expected.snapshot_cycle),
            command_queue_depth: expected.command_queue.depth,
            command_queue_capacity: expected.command_queue.capacity,
            work_queue_depth: expected.work_queue.depth,
            work_queue_capacity: expected.work_queue.capacity,
            telemetry_queue_depth: expected.telemetry_queue.depth,
            telemetry_queue_capacity: expected.telemetry_queue.capacity,
            service_stack: expected.service_stack.wire(),
            realtime_stack: expected.realtime_stack.unwrap().wire(),
        };
        let mut model = crate::health::RuntimeHealthModel::new();
        model
            .accept_response(&crate::Response {
                status: alumina_protocol::StatusCode::Ok,
                body: native.encode().unwrap().to_vec(),
            })
            .unwrap();
        let projected = RuntimeHealthWorkerSnapshot::from_view(model.latest().unwrap());
        assert_eq!(projected, expected);
        assert_eq!(projected.validate(), Ok(()));
    }

    #[test]
    fn snapshot_availability_and_error_relationships_fail_closed() {
        let mut snapshot = DeviceSessionSnapshot {
            connection_id: 7,
            label: "TinyBee bench".to_owned(),
            origin: "http://192.168.4.1".to_owned(),
            generation: 2,
            phase: DeviceSessionPhase::Sampling,
            boot_id: None,
            device_identity: None,
            accepted_samples: 0,
            rejected_samples: 0,
            consecutive_failures: 0,
            estimate: None,
            history: Vec::new(),
            last_error: None,
            runtime_health_availability: RuntimeHealthAvailabilitySnapshot::Unobserved,
            runtime_health: None,
            runtime_health_consecutive_failures: 0,
            runtime_health_last_error: None,
            capability_phase: CapabilityDownloadPhaseSnapshot::Discovering,
            capability_received_bytes: 0,
            capability_identity: None,
            capability_consecutive_failures: 0,
            capability_last_error: None,
            waveform_phase: None,
            waveform_capture_id: None,
            waveform_received_bytes: 0,
            waveform_total_bytes: 0,
            waveform_consecutive_failures: 0,
            waveform_last_error: None,
        };
        assert_eq!(snapshot.validate(), Ok(()));
        snapshot.runtime_health = Some(runtime_health());
        assert_eq!(
            snapshot.validate(),
            Err(WorkerContractError::RuntimeHealthSnapshot)
        );
        snapshot.runtime_health_availability = RuntimeHealthAvailabilitySnapshot::Available;
        assert_eq!(snapshot.validate(), Ok(()));
        snapshot.runtime_health_consecutive_failures = 1;
        assert_eq!(
            snapshot.validate(),
            Err(WorkerContractError::DeviceSnapshot)
        );
        snapshot.runtime_health_last_error = Some("health fetch failed".to_owned());
        assert_eq!(snapshot.validate(), Ok(()));

        let (identity, _) = tinybee_document();
        snapshot.capability_phase = CapabilityDownloadPhaseSnapshot::Downloading;
        snapshot.capability_identity = Some(CapabilityIdentitySnapshot::from_identity(identity));
        snapshot.device_identity = Some(device_identity(identity));
        snapshot.capability_received_bytes = identity.byte_len;
        assert_eq!(
            snapshot.validate(),
            Err(WorkerContractError::CapabilityProgress)
        );
        snapshot.capability_phase = CapabilityDownloadPhaseSnapshot::Complete;
        assert_eq!(snapshot.validate(), Ok(()));
        snapshot.capability_consecutive_failures = 1;
        snapshot.capability_last_error = Some("late failure".to_owned());
        assert_eq!(
            snapshot.validate(),
            Err(WorkerContractError::CapabilityProgress)
        );
        snapshot.capability_consecutive_failures = 0;
        snapshot.capability_last_error = None;
        snapshot.boot_id = Some([3; 16]);
        snapshot.waveform_phase = Some(WaveformCapturePhaseSnapshot::Configured);
        snapshot.waveform_capture_id = Some(*b"WORKER-CAPTURE01");
        assert_eq!(snapshot.validate(), Ok(()));

        snapshot.waveform_phase = Some(WaveformCapturePhaseSnapshot::Downloading);
        snapshot.waveform_received_bytes = 544;
        snapshot.waveform_total_bytes = 544;
        assert_eq!(
            snapshot.validate(),
            Err(WorkerContractError::WaveformProgress)
        );
        snapshot.waveform_received_bytes = 168;
        assert_eq!(snapshot.validate(), Ok(()));
        snapshot.waveform_phase = Some(WaveformCapturePhaseSnapshot::Complete);
        snapshot.waveform_received_bytes = 544;
        assert_eq!(snapshot.validate(), Ok(()));

        snapshot.device_identity.as_mut().unwrap().capability.digest[0] ^= 1;
        assert_eq!(
            snapshot.validate(),
            Err(WorkerContractError::DeviceIdentity)
        );
    }

    #[test]
    fn capability_document_event_is_revalidated_after_json_transfer() {
        let (identity, document) = tinybee_document();
        let transfer = WorkerCapabilityDocument::try_new(7, 3, identity, document).unwrap();
        let event = WorkerEventEnvelope::current(WorkerEvent::CapabilityDocument {
            capability: Box::new(transfer),
        });
        let json = serde_json::to_vec(&event).unwrap();
        let mut decoded: WorkerEventEnvelope = serde_json::from_slice(&json).unwrap();
        assert_eq!(decoded.validate(), Ok(()));

        if let WorkerEvent::CapabilityDocument { capability } = &mut decoded.event {
            capability.document[32] ^= 1;
        } else {
            panic!("round trip changed capability event kind");
        }
        assert_eq!(
            decoded.validate(),
            Err(WorkerContractError::CapabilityDocument)
        );
    }

    #[test]
    fn waveform_document_event_is_revalidated_after_json_transfer() {
        let (identity, _) = tinybee_document();
        let context = alumina_diagnostics::DiagnosticContext {
            device_id: DeviceId(*b"ALUM-SIM:TINYBEE"),
            boot_id: alumina_clock::BootId::new([0x31; 16]).unwrap(),
            capability: identity,
            config_digest: Digest::ZERO,
            clock_frequency_hz: 1_000_000,
        };
        let evidence = tinybee_diagnostic_fixture_for_context(context).unwrap();
        let transfer =
            WorkerWaveformDocument::try_new(7, 3, evidence.digital_capture_bytes().to_vec())
                .unwrap();
        let event = WorkerEventEnvelope::current(WorkerEvent::WaveformDocument {
            waveform: Box::new(transfer),
        });
        let json = serde_json::to_vec(&event).unwrap();
        let mut decoded: WorkerEventEnvelope = serde_json::from_slice(&json).unwrap();
        assert_eq!(decoded.validate(), Ok(()));
        if let WorkerEvent::WaveformDocument { waveform } = &mut decoded.event {
            waveform.record[0] ^= 1;
        } else {
            panic!("round trip changed waveform event kind");
        }
        assert_eq!(
            decoded.validate(),
            Err(WorkerContractError::WaveformDocument)
        );
    }

    #[test]
    fn wrong_schema_is_rejected_after_strict_decode() {
        let envelope = WorkerCommandEnvelope {
            schema_version: WORKER_SCHEMA_VERSION + 1,
            command: WorkerCommand::Disconnect { connection_id: 9 },
        };
        assert_eq!(
            envelope.validate_version(),
            Err(WorkerContractError::SchemaVersion)
        );
    }
}
