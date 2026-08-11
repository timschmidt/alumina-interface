//! Versioned UI/worker messages and redacted live clock diagnostics.

use std::fmt;

use alumina_clock::ClockEstimationPolicy;
use serde::{Deserialize, Serialize};

/// Exact JSON message schema shared by the browser UI and its control worker.
pub const WORKER_SCHEMA_VERSION: u16 = 1;
/// Maximum clock-history records retained and copied into one UI snapshot.
pub const MAXIMUM_CLOCK_HISTORY: usize = 64;

const MAXIMUM_LABEL_BYTES: usize = 64;
const MAXIMUM_ORIGIN_BYTES: usize = 256;
const MAXIMUM_SECRET_BYTES: usize = 256;

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
    };

    /// Converts the wire policy into the exact estimator policy after validation.
    ///
    /// # Errors
    ///
    /// Rejects invalid estimator bounds, a zero uncertainty budget, or an
    /// automatic probe interval outside 100 ms through 60 seconds.
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
        snapshot: DeviceSessionSnapshot,
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
        }
    }
}

impl std::error::Error for WorkerContractError {}

#[cfg(test)]
mod tests {
    use super::*;

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
        candidate.sampling.maximum_timer_error_ns = 0;
        assert_eq!(candidate.validate(), Err(WorkerContractError::ClockPolicy));
    }

    #[test]
    fn snapshots_round_trip_without_a_credential_field() {
        let event = WorkerEventEnvelope::current(WorkerEvent::Snapshot {
            snapshot: DeviceSessionSnapshot {
                connection_id: 7,
                label: "TinyBee bench".to_owned(),
                origin: "http://192.168.4.1".to_owned(),
                generation: 2,
                phase: DeviceSessionPhase::ClockQualified,
                boot_id: Some([3; 16]),
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
            },
        });
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("secret"));
        let decoded: WorkerEventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, event);
        assert_eq!(decoded.validate_version(), Ok(()));
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
