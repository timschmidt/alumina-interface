//! Retry-safe telemetry and waveform control over canonical native operations.

use core::fmt;

use alumina_diagnostics::DiagnosticLimits;
use alumina_diagnostics::transport::{
    DiagnosticTransportError, DiagnosticTransportLimits, TelemetryEventView, TelemetryPhase,
    TelemetrySessionRequest, TelemetrySubscribeView, TelemetrySubscriptionStatus,
    WaveformChunkView, WaveformConfigureView, WaveformPhase, WaveformReadRequest,
    WaveformSessionRequest, WaveformStatus, decode_telemetry_event, decode_telemetry_subscribe,
    decode_waveform_chunk, decode_waveform_configure, validate_retained_capture,
};
use alumina_protocol::{Digest, Operation, StatusCode};

use crate::Response;

/// One diagnostic-family operation ready for native authenticated framing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticOperation {
    /// Exact telemetry or waveform operation.
    pub operation: Operation,
    /// Canonical operation-specific body.
    pub body: Vec<u8>,
}

/// Transport-independent diagnostic client or reconciliation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticClientError {
    /// Canonical transport body was rejected.
    Transport(DiagnosticTransportError),
    /// Device returned a non-success application status with an empty body.
    DeviceStatus(StatusCode),
    /// Failure response carried an ambiguous body, or success omitted its body.
    ResponseBody,
    /// Response identity or admitted fields differ from the exact request.
    Identity,
    /// A response/event contradicted the current client lifecycle.
    State,
    /// An event sequence or cumulative loss count regressed or forked.
    EventOrder,
    /// A request is already pending and must be accepted or abandoned first.
    RequestPending,
    /// No request was pending when a response arrived.
    NoPendingRequest,
    /// Requested lifecycle transition is unavailable in the current phase.
    InvalidTransition,
    /// Browser/native allocation for a bounded canonical body failed.
    Allocation,
}

impl fmt::Display for DiagnosticClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "diagnostic client rejected state: {self:?}")
    }
}

impl std::error::Error for DiagnosticClientError {}

impl From<DiagnosticTransportError> for DiagnosticClientError {
    fn from(error: DiagnosticTransportError) -> Self {
        Self::Transport(error)
    }
}

/// User-facing telemetry subscription lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryClientPhase {
    /// The canonical subscribe request is the unique next mutation.
    Subscribing,
    /// Polling exact status after an ambiguous subscribe result.
    ReconcilingSubscribe,
    /// Subscription is active and events may be accepted.
    Active,
    /// The exact unsubscribe request is the unique next mutation.
    Unsubscribing,
    /// Polling exact status after an ambiguous unsubscribe result.
    ReconcilingUnsubscribe,
    /// Device confirmed removal, or status proved no session exists.
    Unsubscribed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TelemetryAction {
    Subscribe,
    PollSubscribe,
    Unsubscribe,
    PollUnsubscribe,
}

/// Monotonic facts retained from the newest accepted telemetry event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelemetryEventProgress {
    /// Newest accepted event sequence.
    pub event_sequence: u64,
    /// Cumulative device-side latest-only replacement count.
    pub dropped_events: u64,
    /// Digest of the complete canonical overview for duplicate detection.
    pub overview_digest: Digest,
}

/// Result of applying a validated device-originated telemetry event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelemetryEventAcceptance<'a> {
    advanced: bool,
    event: TelemetryEventView<'a>,
}

impl<'a> TelemetryEventAcceptance<'a> {
    /// Whether this event strictly advanced retained client progress.
    pub const fn advanced(self) -> bool {
        self.advanced
    }

    /// Independently validated event, including on an exact duplicate replay.
    pub const fn event(self) -> TelemetryEventView<'a> {
        self.event
    }
}

/// Retry-safe subscription coordinator bound to one complete canonical request.
#[derive(Debug)]
pub struct TelemetrySubscriptionMachine {
    request: Vec<u8>,
    limits: DiagnosticTransportLimits,
    record_limits: DiagnosticLimits,
    reference: TelemetrySessionRequest,
    phase: TelemetryClientPhase,
    pending: Option<TelemetryAction>,
    status: Option<TelemetrySubscriptionStatus>,
    event_progress: Option<TelemetryEventProgress>,
}

impl TelemetrySubscriptionMachine {
    /// Validates and owns one canonical subscribe request.
    pub fn new(
        request: Vec<u8>,
        limits: DiagnosticTransportLimits,
        record_limits: DiagnosticLimits,
    ) -> Result<Self, DiagnosticClientError> {
        let subscription = decode_telemetry_subscribe(&request, limits)?;
        let reference = TelemetrySessionRequest {
            subscription_id: subscription.subscription_id(),
            subscription_digest: subscription.digest(),
        };
        Ok(Self {
            request,
            limits,
            record_limits,
            reference,
            phase: TelemetryClientPhase::Subscribing,
            pending: None,
            status: None,
            event_progress: None,
        })
    }

    /// Exact subscription reference used for every status/removal operation.
    pub const fn reference(&self) -> TelemetrySessionRequest {
        self.reference
    }

    /// Current retry/reconciliation phase.
    pub const fn phase(&self) -> TelemetryClientPhase {
        self.phase
    }

    /// Latest independently validated device status.
    pub const fn status(&self) -> Option<TelemetrySubscriptionStatus> {
        self.status
    }

    /// Newest accepted event sequence, loss count, and overview digest.
    pub const fn event_progress(&self) -> Option<TelemetryEventProgress> {
        self.event_progress
    }

    /// Emits the unique next operation, if this phase requires network work.
    pub fn next_request(&mut self) -> Result<Option<DiagnosticOperation>, DiagnosticClientError> {
        if self.pending.is_some() {
            return Err(DiagnosticClientError::RequestPending);
        }
        let (action, operation, body) = match self.phase {
            TelemetryClientPhase::Subscribing => (
                TelemetryAction::Subscribe,
                Operation::TelemetrySubscribe,
                self.request.clone(),
            ),
            TelemetryClientPhase::ReconcilingSubscribe => (
                TelemetryAction::PollSubscribe,
                Operation::TelemetryStatus,
                self.reference.encode()?.to_vec(),
            ),
            TelemetryClientPhase::Unsubscribing => (
                TelemetryAction::Unsubscribe,
                Operation::TelemetryUnsubscribe,
                self.reference.encode()?.to_vec(),
            ),
            TelemetryClientPhase::ReconcilingUnsubscribe => (
                TelemetryAction::PollUnsubscribe,
                Operation::TelemetryStatus,
                self.reference.encode()?.to_vec(),
            ),
            TelemetryClientPhase::Active | TelemetryClientPhase::Unsubscribed => return Ok(None),
        };
        self.pending = Some(action);
        Ok(Some(DiagnosticOperation { operation, body }))
    }

    /// Requests removal after the active subscription is known exactly.
    pub fn request_unsubscribe(&mut self) -> Result<(), DiagnosticClientError> {
        if self.pending.is_some() {
            return Err(DiagnosticClientError::RequestPending);
        }
        match self.phase {
            TelemetryClientPhase::Active => {
                self.phase = TelemetryClientPhase::Unsubscribing;
                Ok(())
            }
            TelemetryClientPhase::Unsubscribed => Ok(()),
            _ => Err(DiagnosticClientError::InvalidTransition),
        }
    }

    /// Applies one already authenticated and correlated response.
    pub fn accept_response(&mut self, response: &Response) -> Result<(), DiagnosticClientError> {
        let action = self
            .pending
            .take()
            .ok_or(DiagnosticClientError::NoPendingRequest)?;
        match action {
            TelemetryAction::PollSubscribe if response.status == StatusCode::NotFound => {
                require_empty(response)?;
                self.phase = TelemetryClientPhase::Subscribing;
                return Ok(());
            }
            TelemetryAction::PollUnsubscribe if response.status == StatusCode::NotFound => {
                require_empty(response)?;
                self.phase = TelemetryClientPhase::Unsubscribed;
                self.status = None;
                return Ok(());
            }
            _ => {}
        }
        require_ok(response)?;
        let status = TelemetrySubscriptionStatus::decode(&response.body)?;
        let subscription = self.subscription()?;
        validate_telemetry_status(status, subscription)?;
        if self.event_progress.is_some_and(|progress| {
            status.next_event_sequence <= progress.event_sequence
                || status.dropped_events < progress.dropped_events
        }) {
            return Err(DiagnosticClientError::EventOrder);
        }
        let expected_phase = match action {
            TelemetryAction::Subscribe | TelemetryAction::PollSubscribe => TelemetryPhase::Active,
            TelemetryAction::Unsubscribe | TelemetryAction::PollUnsubscribe => {
                TelemetryPhase::Unsubscribed
            }
        };
        if status.phase != expected_phase {
            return Err(DiagnosticClientError::State);
        }
        self.status = Some(status);
        self.phase = match expected_phase {
            TelemetryPhase::Active => TelemetryClientPhase::Active,
            TelemetryPhase::Unsubscribed => TelemetryClientPhase::Unsubscribed,
        };
        Ok(())
    }

    /// Converts an ambiguous I/O outcome into an exact status reconciliation.
    pub fn abandon_pending(&mut self) -> bool {
        let Some(action) = self.pending.take() else {
            return false;
        };
        self.phase = match action {
            TelemetryAction::Subscribe | TelemetryAction::PollSubscribe => {
                TelemetryClientPhase::ReconcilingSubscribe
            }
            TelemetryAction::Unsubscribe | TelemetryAction::PollUnsubscribe => {
                TelemetryClientPhase::ReconcilingUnsubscribe
            }
        };
        true
    }

    /// Validates and applies one complete device-originated event envelope.
    pub fn accept_event<'a>(
        &mut self,
        encoded: &'a [u8],
    ) -> Result<TelemetryEventAcceptance<'a>, DiagnosticClientError> {
        if self.phase != TelemetryClientPhase::Active {
            return Err(DiagnosticClientError::State);
        }
        let event = decode_telemetry_event(encoded, self.subscription()?, self.record_limits)?;
        let progress = TelemetryEventProgress {
            event_sequence: event.event_sequence(),
            dropped_events: event.dropped_events(),
            overview_digest: event.overview_digest(),
        };
        if let Some(previous) = self.event_progress {
            if progress == previous {
                return Ok(TelemetryEventAcceptance {
                    advanced: false,
                    event,
                });
            }
            if progress.event_sequence <= previous.event_sequence
                || progress.dropped_events < previous.dropped_events
            {
                return Err(DiagnosticClientError::EventOrder);
            }
        }
        self.event_progress = Some(progress);
        Ok(TelemetryEventAcceptance {
            advanced: true,
            event,
        })
    }

    fn subscription(&self) -> Result<TelemetrySubscribeView<'_>, DiagnosticClientError> {
        decode_telemetry_subscribe(&self.request, self.limits).map_err(Into::into)
    }
}

fn validate_telemetry_status(
    status: TelemetrySubscriptionStatus,
    subscription: TelemetrySubscribeView<'_>,
) -> Result<(), DiagnosticClientError> {
    if status.subscription_id != subscription.subscription_id()
        || status.subscription_digest != subscription.digest()
        || status.minimum_period_cycles != subscription.minimum_period_cycles()
        || status.maximum_event_bytes != subscription.maximum_event_bytes()
        || usize::from(status.resource_count) != subscription.resource_count()
    {
        return Err(DiagnosticClientError::Identity);
    }
    Ok(())
}

/// User-facing waveform acquisition and exact recovery lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaveformClientPhase {
    /// Canonical configuration is the unique next mutation.
    Configuring,
    /// Polling exact status after an ambiguous configuration result.
    ReconcilingConfigure,
    /// Configuration is admitted but deliberately not armed.
    Configured,
    /// Exact arm is the unique next mutation.
    Arming,
    /// Polling exact status after an ambiguous arm result.
    ReconcilingArm,
    /// Acquisition is armed; exact status polling is available.
    Armed,
    /// A complete retained record is being fetched in canonical ranges.
    Downloading {
        /// Contiguous validated prefix bytes already retained by the client.
        received_bytes: u32,
        /// Exact complete retained-record length from status.
        total_bytes: u32,
    },
    /// Complete bytes passed record digest and configuration validation.
    Complete,
    /// Exact stop/release is the unique next mutation.
    Stopping,
    /// Polling exact status after an ambiguous stop result.
    ReconcilingStop,
    /// Device confirmed release, or status proved no session exists.
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaveformAction {
    Configure,
    PollConfigure,
    Arm,
    PollArm,
    PollComplete,
    Read,
    Stop,
    PollStop,
}

/// Retry-safe waveform coordinator and exact retained-record assembler.
#[derive(Debug)]
pub struct WaveformCaptureMachine {
    configure: Vec<u8>,
    limits: DiagnosticTransportLimits,
    record_limits: DiagnosticLimits,
    reference: WaveformSessionRequest,
    maximum_chunk_bytes: u32,
    phase: WaveformClientPhase,
    pending: Option<WaveformAction>,
    status: Option<WaveformStatus>,
    record: Vec<u8>,
}

impl WaveformCaptureMachine {
    /// Validates and owns one canonical waveform configuration.
    pub fn new(
        configure: Vec<u8>,
        limits: DiagnosticTransportLimits,
        record_limits: DiagnosticLimits,
    ) -> Result<Self, DiagnosticClientError> {
        let configuration = decode_waveform_configure(&configure, limits)?;
        let reference = WaveformSessionRequest {
            capture_id: configuration.capture_id(),
            configure_digest: configuration.digest(),
        };
        let maximum_chunk_bytes = configuration.maximum_chunk_bytes();
        Ok(Self {
            configure,
            limits,
            record_limits,
            reference,
            maximum_chunk_bytes,
            phase: WaveformClientPhase::Configuring,
            pending: None,
            status: None,
            record: Vec::new(),
        })
    }

    /// Exact capture/configuration reference used by every later operation.
    pub const fn reference(&self) -> WaveformSessionRequest {
        self.reference
    }

    /// Current retry/reconciliation/download phase.
    pub const fn phase(&self) -> WaveformClientPhase {
        self.phase
    }

    /// Latest independently validated lifecycle status.
    pub const fn status(&self) -> Option<WaveformStatus> {
        self.status
    }

    /// Complete validated canonical capture bytes, only in `Complete` phase.
    pub fn record(&self) -> Option<&[u8]> {
        (self.phase == WaveformClientPhase::Complete).then_some(self.record.as_slice())
    }

    /// Emits the unique next operation, including exact status polls and ranges.
    pub fn next_request(&mut self) -> Result<Option<DiagnosticOperation>, DiagnosticClientError> {
        if self.pending.is_some() {
            return Err(DiagnosticClientError::RequestPending);
        }
        let (action, operation, body) = match self.phase {
            WaveformClientPhase::Configuring => (
                WaveformAction::Configure,
                Operation::WaveformConfigure,
                self.configure.clone(),
            ),
            WaveformClientPhase::ReconcilingConfigure => (
                WaveformAction::PollConfigure,
                Operation::WaveformStatus,
                self.reference.encode()?.to_vec(),
            ),
            WaveformClientPhase::Arming => (
                WaveformAction::Arm,
                Operation::WaveformArm,
                self.reference.encode()?.to_vec(),
            ),
            WaveformClientPhase::ReconcilingArm => (
                WaveformAction::PollArm,
                Operation::WaveformStatus,
                self.reference.encode()?.to_vec(),
            ),
            WaveformClientPhase::Armed => (
                WaveformAction::PollComplete,
                Operation::WaveformStatus,
                self.reference.encode()?.to_vec(),
            ),
            WaveformClientPhase::Downloading { received_bytes, .. } => {
                let status = self.status.ok_or(DiagnosticClientError::State)?;
                let read = WaveformReadRequest {
                    capture_id: self.reference.capture_id,
                    configure_digest: self.reference.configure_digest,
                    record_digest: status.record_digest,
                    offset: received_bytes,
                    maximum_bytes: self.maximum_chunk_bytes,
                };
                (
                    WaveformAction::Read,
                    Operation::WaveformRead,
                    read.encode(self.limits)?.to_vec(),
                )
            }
            WaveformClientPhase::Stopping => (
                WaveformAction::Stop,
                Operation::WaveformStop,
                self.reference.encode()?.to_vec(),
            ),
            WaveformClientPhase::ReconcilingStop => (
                WaveformAction::PollStop,
                Operation::WaveformStatus,
                self.reference.encode()?.to_vec(),
            ),
            WaveformClientPhase::Configured
            | WaveformClientPhase::Complete
            | WaveformClientPhase::Stopped => return Ok(None),
        };
        self.pending = Some(action);
        Ok(Some(DiagnosticOperation { operation, body }))
    }

    /// Requests arming only after configuration admission is known exactly.
    pub fn request_arm(&mut self) -> Result<(), DiagnosticClientError> {
        if self.pending.is_some() {
            return Err(DiagnosticClientError::RequestPending);
        }
        if self.phase != WaveformClientPhase::Configured {
            return Err(DiagnosticClientError::InvalidTransition);
        }
        self.phase = WaveformClientPhase::Arming;
        Ok(())
    }

    /// Requests capture release from any known configured lifecycle.
    pub fn request_stop(&mut self) -> Result<(), DiagnosticClientError> {
        if self.pending.is_some() {
            return Err(DiagnosticClientError::RequestPending);
        }
        match self.phase {
            WaveformClientPhase::Configured
            | WaveformClientPhase::Armed
            | WaveformClientPhase::Downloading { .. }
            | WaveformClientPhase::Complete => {
                self.phase = WaveformClientPhase::Stopping;
                Ok(())
            }
            WaveformClientPhase::Stopped => Ok(()),
            _ => Err(DiagnosticClientError::InvalidTransition),
        }
    }

    /// Applies one already authenticated and correlated response.
    pub fn accept_response(&mut self, response: &Response) -> Result<(), DiagnosticClientError> {
        let action = self
            .pending
            .take()
            .ok_or(DiagnosticClientError::NoPendingRequest)?;
        match action {
            WaveformAction::PollConfigure if response.status == StatusCode::NotFound => {
                require_empty(response)?;
                self.phase = WaveformClientPhase::Configuring;
                return Ok(());
            }
            WaveformAction::PollStop if response.status == StatusCode::NotFound => {
                require_empty(response)?;
                self.phase = WaveformClientPhase::Stopped;
                self.status = None;
                self.record.clear();
                return Ok(());
            }
            _ => {}
        }
        require_ok(response)?;
        if action == WaveformAction::Read {
            return self.accept_range(&response.body);
        }
        let status = WaveformStatus::decode(&response.body)?;
        self.validate_status(status)?;
        match action {
            WaveformAction::Configure | WaveformAction::PollConfigure => {
                if status.phase != WaveformPhase::Configured {
                    return Err(DiagnosticClientError::State);
                }
                self.phase = WaveformClientPhase::Configured;
            }
            WaveformAction::Arm | WaveformAction::PollArm | WaveformAction::PollComplete => {
                self.accept_acquisition_status(status)?;
            }
            WaveformAction::Stop | WaveformAction::PollStop => {
                if status.phase != WaveformPhase::Stopped {
                    return Err(DiagnosticClientError::State);
                }
                self.phase = WaveformClientPhase::Stopped;
                self.record.clear();
            }
            WaveformAction::Read => unreachable!("range handled before status decoding"),
        };
        self.status = Some(status);
        Ok(())
    }

    /// Converts an ambiguous I/O result into exact status/range reconciliation.
    pub fn abandon_pending(&mut self) -> bool {
        let Some(action) = self.pending.take() else {
            return false;
        };
        self.phase = match action {
            WaveformAction::Configure | WaveformAction::PollConfigure => {
                WaveformClientPhase::ReconcilingConfigure
            }
            WaveformAction::Arm | WaveformAction::PollArm => WaveformClientPhase::ReconcilingArm,
            WaveformAction::PollComplete => WaveformClientPhase::Armed,
            WaveformAction::Read => self.phase,
            WaveformAction::Stop | WaveformAction::PollStop => WaveformClientPhase::ReconcilingStop,
        };
        true
    }

    fn accept_acquisition_status(
        &mut self,
        status: WaveformStatus,
    ) -> Result<(), DiagnosticClientError> {
        match status.phase {
            WaveformPhase::Armed => {
                self.phase = WaveformClientPhase::Armed;
                Ok(())
            }
            WaveformPhase::Complete => self.begin_download(status),
            WaveformPhase::Configured if self.phase == WaveformClientPhase::ReconcilingArm => {
                self.phase = WaveformClientPhase::Arming;
                Ok(())
            }
            WaveformPhase::Faulted | WaveformPhase::Stopped | WaveformPhase::Configured => {
                Err(DiagnosticClientError::State)
            }
        }
    }

    fn begin_download(&mut self, status: WaveformStatus) -> Result<(), DiagnosticClientError> {
        if status.record_bytes == 0
            || status.record_bytes > self.limits.maximum_waveform_record_bytes
            || status.record_digest.is_zero()
        {
            return Err(DiagnosticClientError::State);
        }
        let total =
            usize::try_from(status.record_bytes).map_err(|_| DiagnosticClientError::Allocation)?;
        self.record.clear();
        self.record
            .try_reserve_exact(total)
            .map_err(|_| DiagnosticClientError::Allocation)?;
        self.phase = WaveformClientPhase::Downloading {
            received_bytes: 0,
            total_bytes: status.record_bytes,
        };
        Ok(())
    }

    fn accept_range(&mut self, encoded: &[u8]) -> Result<(), DiagnosticClientError> {
        let status = self.status.ok_or(DiagnosticClientError::State)?;
        let WaveformClientPhase::Downloading {
            received_bytes,
            total_bytes,
        } = self.phase
        else {
            return Err(DiagnosticClientError::State);
        };
        let chunk =
            decode_waveform_chunk(encoded, self.reference, status.record_digest, self.limits)?;
        if chunk.offset() != received_bytes || chunk.record_bytes() != total_bytes {
            return Err(DiagnosticClientError::Identity);
        }
        self.record
            .try_reserve(chunk.bytes().len())
            .map_err(|_| DiagnosticClientError::Allocation)?;
        self.record.extend_from_slice(chunk.bytes());
        let received_bytes = chunk.end_offset();
        if received_bytes == total_bytes {
            let configuration = self.configuration()?;
            let validation = validate_retained_capture(
                &self.record,
                configuration,
                self.record_limits,
                self.limits,
            );
            let digest = match validation {
                Ok(digest) => digest,
                Err(error) => {
                    self.record.clear();
                    self.phase = WaveformClientPhase::Downloading {
                        received_bytes: 0,
                        total_bytes,
                    };
                    return Err(error.into());
                }
            };
            if digest != status.record_digest {
                self.record.clear();
                self.phase = WaveformClientPhase::Downloading {
                    received_bytes: 0,
                    total_bytes,
                };
                return Err(DiagnosticClientError::Identity);
            }
            self.phase = WaveformClientPhase::Complete;
        } else {
            self.phase = WaveformClientPhase::Downloading {
                received_bytes,
                total_bytes,
            };
        }
        Ok(())
    }

    fn configuration(&self) -> Result<WaveformConfigureView<'_>, DiagnosticClientError> {
        decode_waveform_configure(&self.configure, self.limits).map_err(Into::into)
    }

    fn validate_status(&self, status: WaveformStatus) -> Result<(), DiagnosticClientError> {
        if status.capture_id != self.reference.capture_id
            || status.configure_digest != self.reference.configure_digest
        {
            return Err(DiagnosticClientError::Identity);
        }
        Ok(())
    }
}

fn require_ok(response: &Response) -> Result<(), DiagnosticClientError> {
    if response.status == StatusCode::Ok {
        if response.body.is_empty() {
            Err(DiagnosticClientError::ResponseBody)
        } else {
            Ok(())
        }
    } else if response.body.is_empty() {
        Err(DiagnosticClientError::DeviceStatus(response.status))
    } else {
        Err(DiagnosticClientError::ResponseBody)
    }
}

fn require_empty(response: &Response) -> Result<(), DiagnosticClientError> {
    if response.body.is_empty() {
        Ok(())
    } else {
        Err(DiagnosticClientError::ResponseBody)
    }
}

/// Copies an already validated waveform chunk into owned UI/client storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedWaveformChunk {
    /// Complete retained-record digest.
    pub record_digest: Digest,
    /// Complete retained-record byte length.
    pub record_bytes: u32,
    /// First represented byte.
    pub offset: u32,
    /// Independently hashed range bytes.
    pub bytes: Vec<u8>,
}

impl OwnedWaveformChunk {
    /// Fallibly owns a validated borrowed chunk.
    pub fn try_from_view(view: WaveformChunkView<'_>) -> Result<Self, DiagnosticClientError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(view.bytes().len())
            .map_err(|_| DiagnosticClientError::Allocation)?;
        bytes.extend_from_slice(view.bytes());
        Ok(Self {
            record_digest: view.record_digest(),
            record_bytes: view.record_bytes(),
            offset: view.offset(),
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use alumina_board::ResourceId;
    use alumina_diagnostics::transport::{
        SubscriptionId, TelemetrySubscribeFlags, TelemetrySubscribeRequest, WaveformConfigureFlags,
        WaveformConfigureRequest, encode_telemetry_subscribe, encode_waveform_configure,
    };
    use alumina_diagnostics::{CaptureId, DigitalTriggerCondition};
    use alumina_net::{AuthenticatedMedia, BootNonce, CORS_ORIGIN_HEADER, CorsOrigin, HttpMethod};
    use alumina_protocol::DeviceCycle;
    use alumina_service::diagnostics::{DiagnosticProviderPolicy, DiagnosticServiceState};
    use alumina_service::{ServiceRequest, ServiceResponse};
    use alumina_sim::diagnostics::{
        tinybee_diagnostic_fixture, tinybee_diagnostic_fixture_for_context,
    };
    use alumina_sim::http_fixture::{ClockFixturePolicy, ClockHttpFixture, FixtureHttpRequest};

    use crate::http::{AuthenticatedHttpSession, AuthenticatedProtocolResponse};
    use crate::{ClientError, ProtocolClient, Transport};

    use super::*;

    const RESOURCES: [ResourceId; 4] = [
        ResourceId::Gpio(22),
        ResourceId::Gpio(32),
        ResourceId::Gpio(33),
        ResourceId::Gpio(35),
    ];

    type NativeState = DiagnosticServiceState<176, 432, 208, 2_048>;

    #[derive(Debug)]
    struct NativeDiagnosticTransport {
        state: NativeState,
        now: DeviceCycle,
    }

    impl NativeDiagnosticTransport {
        fn new(context: alumina_diagnostics::DiagnosticContext) -> Self {
            Self {
                state: NativeState::new(
                    context,
                    DiagnosticProviderPolicy::SIMULATED,
                    DiagnosticTransportLimits::native_control(),
                    DiagnosticLimits::interactive(),
                ),
                now: DeviceCycle(2_000_100),
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum NativeTransportError {
        Admission,
    }

    impl fmt::Display for NativeTransportError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "{self:?}")
        }
    }

    impl Error for NativeTransportError {}

    impl Transport for NativeDiagnosticTransport {
        type Error = NativeTransportError;

        fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>, Self::Error> {
            let request =
                ServiceRequest::native(request).map_err(|_| NativeTransportError::Admission)?;
            let response: ServiceResponse = self.state.dispatch(&request, self.now);
            self.now.0 = self.now.0.saturating_add(10);
            Ok(response.bytes().to_vec())
        }
    }

    fn subscription_bytes(context: alumina_diagnostics::DiagnosticContext) -> Vec<u8> {
        let mut encoded = [0_u8; 176];
        encode_telemetry_subscribe(
            &TelemetrySubscribeRequest {
                subscription_id: SubscriptionId::new(7).unwrap(),
                context,
                flags: TelemetrySubscribeFlags(TelemetrySubscribeFlags::LATEST_ONLY),
                minimum_period_cycles: 10_000,
                maximum_event_bytes: 432,
                resources: &RESOURCES,
            },
            &mut encoded,
            DiagnosticTransportLimits::native_control(),
        )
        .unwrap();
        encoded.to_vec()
    }

    fn configuration_bytes(context: alumina_diagnostics::DiagnosticContext) -> Vec<u8> {
        let mut encoded = [0_u8; 208];
        encode_waveform_configure(
            &WaveformConfigureRequest {
                capture_id: CaptureId::new(*b"TINYBEE-SIM-0001").unwrap(),
                context,
                flags: WaveformConfigureFlags(WaveformConfigureFlags::EDGE_TIMESTAMPS),
                requested_pretrigger_cycles: 500,
                requested_posttrigger_cycles: 1_500,
                earliest_trigger_cycle: DeviceCycle(2_000_400),
                latest_trigger_cycle: DeviceCycle(2_000_600),
                transition_capacity: 64,
                maximum_chunk_bytes: 168,
                trigger_channel_index: 2,
                trigger_condition: DigitalTriggerCondition::Rising,
                channels: &RESOURCES,
            },
            &mut encoded,
            DiagnosticTransportLimits::native_control(),
        )
        .unwrap();
        encoded.to_vec()
    }

    fn exchange(
        client: &mut ProtocolClient<NativeDiagnosticTransport>,
        operation: DiagnosticOperation,
    ) -> Result<Response, ClientError<NativeTransportError>> {
        client.request(operation.operation, &operation.body)
    }

    fn authenticated_exchange(
        fixture: &mut ClockHttpFixture,
        session: &mut AuthenticatedHttpSession,
        operation: DiagnosticOperation,
        counter: u64,
    ) -> Response {
        const SECRET: &[u8] = b"alumina-development";
        const ORIGIN: &str = "http://127.0.0.1:8097";

        let outbound = session
            .begin_request(operation.operation, &operation.body, SECRET)
            .unwrap();
        let counter_header = outbound.counter_header_value();
        let authorization = outbound.authorization_header_value().as_bytes().to_vec();
        let body = outbound.body().to_vec();
        let request = FixtureHttpRequest {
            method: HttpMethod::Post,
            path: outbound.path().to_owned(),
            headers: vec![
                (
                    b"Content-Type".to_vec(),
                    outbound.content_type().as_bytes().to_vec(),
                ),
                (
                    b"Content-Length".to_vec(),
                    body.len().to_string().into_bytes(),
                ),
                (
                    outbound.counter_header_name().as_bytes().to_vec(),
                    counter_header.as_bytes().to_vec(),
                ),
                (
                    outbound.authorization_header_name().as_bytes().to_vec(),
                    authorization,
                ),
                (
                    CORS_ORIGIN_HEADER.as_bytes().to_vec(),
                    ORIGIN.as_bytes().to_vec(),
                ),
            ],
            body,
        };
        let response = fixture.handle(
            &request,
            counter * 10,
            DeviceCycle(2_000_090),
            DeviceCycle(2_000_100),
        );
        assert_eq!(response.status, 200);
        let response_counter = response
            .headers
            .iter()
            .find(|(name, _)| name == outbound.counter_header_name())
            .unwrap()
            .1
            .clone();
        let response_authorization = response
            .headers
            .iter()
            .find(|(name, _)| name == AuthenticatedProtocolResponse::authorization_header_name())
            .unwrap()
            .1
            .clone();
        session
            .accept_response(
                AuthenticatedProtocolResponse {
                    http_status: response.status,
                    media: AuthenticatedMedia::NativeFrame,
                    counter_header: &response_counter,
                    authorization_header: &response_authorization,
                    body: &response.body,
                },
                SECRET,
            )
            .unwrap()
    }

    #[test]
    fn subscription_reconciles_lost_mutation_response_and_accepts_exact_events() {
        let fixture = tinybee_diagnostic_fixture().unwrap();
        let context = fixture.overview().context();
        let request = subscription_bytes(context);
        let mut machine = TelemetrySubscriptionMachine::new(
            request,
            DiagnosticTransportLimits::native_control(),
            DiagnosticLimits::interactive(),
        )
        .unwrap();
        let mut client = ProtocolClient::new(
            NativeDiagnosticTransport::new(context),
            context.config_digest,
        );

        let subscribe = machine.next_request().unwrap().unwrap();
        let lost_response = exchange(&mut client, subscribe).unwrap();
        assert_eq!(lost_response.status, StatusCode::Ok);
        assert!(machine.abandon_pending());
        assert_eq!(machine.phase(), TelemetryClientPhase::ReconcilingSubscribe);
        let poll = machine.next_request().unwrap().unwrap();
        let response = exchange(&mut client, poll).unwrap();
        machine.accept_response(&response).unwrap();
        assert_eq!(machine.phase(), TelemetryClientPhase::Active);

        client
            .transport_mut()
            .state
            .publish_overview(fixture.overview_bytes())
            .unwrap();
        let event = client
            .transport()
            .state
            .pending_telemetry_event()
            .unwrap()
            .to_vec();
        let accepted = machine.accept_event(&event).unwrap();
        assert!(accepted.advanced());
        assert!(!machine.accept_event(&event).unwrap().advanced());
        client
            .transport_mut()
            .state
            .acknowledge_telemetry_event(machine.reference(), 1)
            .unwrap();

        let mut second = fixture.overview_bytes().to_vec();
        second[128..136].copy_from_slice(&1_260_000_u64.to_le_bytes());
        second[136..144].copy_from_slice(&2_u64.to_le_bytes());
        client
            .transport_mut()
            .state
            .publish_overview(&second)
            .unwrap();
        let event = client
            .transport()
            .state
            .pending_telemetry_event()
            .unwrap()
            .to_vec();
        let accepted = machine.accept_event(&event).unwrap();
        assert!(accepted.advanced());
        let event = accepted.event();
        assert_eq!(event.event_sequence(), 2);

        machine.request_unsubscribe().unwrap();
        let unsubscribe = machine.next_request().unwrap().unwrap();
        let lost_response = exchange(&mut client, unsubscribe).unwrap();
        assert_eq!(lost_response.status, StatusCode::Ok);
        assert!(machine.abandon_pending());
        let poll = machine.next_request().unwrap().unwrap();
        let response = exchange(&mut client, poll).unwrap();
        machine.accept_response(&response).unwrap();
        assert_eq!(machine.phase(), TelemetryClientPhase::Unsubscribed);
    }

    #[test]
    fn waveform_reconciles_configuration_and_range_retry_to_exact_record() {
        let fixture = tinybee_diagnostic_fixture().unwrap();
        let context = fixture.digital_capture().context();
        let configure = configuration_bytes(context);
        let mut machine = WaveformCaptureMachine::new(
            configure,
            DiagnosticTransportLimits::native_control(),
            DiagnosticLimits::interactive(),
        )
        .unwrap();
        let mut client = ProtocolClient::new(
            NativeDiagnosticTransport::new(context),
            context.config_digest,
        );

        let configure = machine.next_request().unwrap().unwrap();
        let lost_response = exchange(&mut client, configure).unwrap();
        assert_eq!(lost_response.status, StatusCode::Ok);
        assert!(machine.abandon_pending());
        let poll = machine.next_request().unwrap().unwrap();
        machine
            .accept_response(&exchange(&mut client, poll).unwrap())
            .unwrap();
        assert_eq!(machine.phase(), WaveformClientPhase::Configured);

        machine.request_arm().unwrap();
        let arm = machine.next_request().unwrap().unwrap();
        machine
            .accept_response(&exchange(&mut client, arm).unwrap())
            .unwrap();
        assert_eq!(machine.phase(), WaveformClientPhase::Armed);
        client
            .transport_mut()
            .state
            .retain_waveform_capture(fixture.digital_capture_bytes())
            .unwrap();

        let status = machine.next_request().unwrap().unwrap();
        machine
            .accept_response(&exchange(&mut client, status).unwrap())
            .unwrap();
        assert_eq!(
            machine.phase(),
            WaveformClientPhase::Downloading {
                received_bytes: 0,
                total_bytes: 512
            }
        );

        let first_read = machine.next_request().unwrap().unwrap();
        let first_body = first_read.body.clone();
        let lost_range = exchange(&mut client, first_read).unwrap();
        assert_eq!(lost_range.status, StatusCode::Ok);
        assert!(machine.abandon_pending());
        let retried = machine.next_request().unwrap().unwrap();
        assert_eq!(retried.body, first_body);
        machine
            .accept_response(&exchange(&mut client, retried).unwrap())
            .unwrap();

        while machine.phase() != WaveformClientPhase::Complete {
            let read = machine.next_request().unwrap().unwrap();
            let response = exchange(&mut client, read).unwrap();
            machine.accept_response(&response).unwrap();
        }
        assert_eq!(machine.record(), Some(fixture.digital_capture_bytes()));

        machine.request_stop().unwrap();
        let stop = machine.next_request().unwrap().unwrap();
        let lost_response = exchange(&mut client, stop).unwrap();
        assert_eq!(lost_response.status, StatusCode::Ok);
        assert!(machine.abandon_pending());
        let poll = machine.next_request().unwrap().unwrap();
        machine
            .accept_response(&exchange(&mut client, poll).unwrap())
            .unwrap();
        assert_eq!(machine.phase(), WaveformClientPhase::Stopped);
        assert!(machine.record().is_none());
    }

    #[test]
    fn status_identity_substitution_cannot_advance_client_state() {
        let fixture = tinybee_diagnostic_fixture().unwrap();
        let context = fixture.overview().context();
        let mut machine = TelemetrySubscriptionMachine::new(
            subscription_bytes(context),
            DiagnosticTransportLimits::native_control(),
            DiagnosticLimits::interactive(),
        )
        .unwrap();
        let _request = machine.next_request().unwrap().unwrap();
        let subscription = machine.subscription().unwrap();
        let substituted = TelemetrySubscriptionStatus {
            phase: TelemetryPhase::Active,
            pending: false,
            subscription_id: subscription.subscription_id(),
            subscription_digest: Digest([0x99; 32]),
            minimum_period_cycles: subscription.minimum_period_cycles(),
            maximum_event_bytes: subscription.maximum_event_bytes(),
            resource_count: u16::try_from(subscription.resource_count()).unwrap(),
            next_event_sequence: 1,
            published_events: 0,
            dropped_events: 0,
            last_event_cycle: DeviceCycle(0),
            pending_event_sequence: 0,
            pending_event_bytes: 0,
        };
        let response = Response {
            status: StatusCode::Ok,
            body: substituted.encode().unwrap().to_vec(),
        };
        assert_eq!(
            machine.accept_response(&response),
            Err(DiagnosticClientError::Identity)
        );
    }

    #[test]
    fn authenticated_http_fixture_carries_diagnostic_lifecycle_and_exact_ranges() {
        const SECRET: &[u8] = b"alumina-development";
        let mut fixture = ClockHttpFixture::new(
            SECRET.to_vec(),
            [0x31; 16],
            ClockFixturePolicy::HEALTHY_1MHZ,
        )
        .unwrap();
        let context = fixture.diagnostic_context();
        let evidence = tinybee_diagnostic_fixture_for_context(context).unwrap();
        let mut session = AuthenticatedHttpSession::new(
            BootNonce::new([0x31; 16]).unwrap(),
            context.config_digest,
            CorsOrigin::parse("http://127.0.0.1:8097").unwrap(),
        );
        let mut counter = 1;

        let mut telemetry = TelemetrySubscriptionMachine::new(
            subscription_bytes(context),
            DiagnosticTransportLimits::native_control(),
            DiagnosticLimits::interactive(),
        )
        .unwrap();
        let operation = telemetry.next_request().unwrap().unwrap();
        let response = authenticated_exchange(&mut fixture, &mut session, operation, counter);
        counter += 1;
        telemetry.accept_response(&response).unwrap();
        assert_eq!(telemetry.phase(), TelemetryClientPhase::Active);
        fixture
            .diagnostics_mut()
            .publish_overview(evidence.overview_bytes())
            .unwrap();
        let event = fixture
            .diagnostics()
            .pending_telemetry_event()
            .unwrap()
            .to_vec();
        assert!(telemetry.accept_event(&event).unwrap().advanced());

        let mut waveform = WaveformCaptureMachine::new(
            configuration_bytes(context),
            DiagnosticTransportLimits::native_control(),
            DiagnosticLimits::interactive(),
        )
        .unwrap();
        let operation = waveform.next_request().unwrap().unwrap();
        let response = authenticated_exchange(&mut fixture, &mut session, operation, counter);
        counter += 1;
        waveform.accept_response(&response).unwrap();
        waveform.request_arm().unwrap();
        let operation = waveform.next_request().unwrap().unwrap();
        let response = authenticated_exchange(&mut fixture, &mut session, operation, counter);
        counter += 1;
        waveform.accept_response(&response).unwrap();
        fixture
            .diagnostics_mut()
            .retain_waveform_capture(evidence.digital_capture_bytes())
            .unwrap();

        while waveform.phase() != WaveformClientPhase::Complete {
            let operation = waveform.next_request().unwrap().unwrap();
            let response = authenticated_exchange(&mut fixture, &mut session, operation, counter);
            counter += 1;
            waveform.accept_response(&response).unwrap();
        }
        assert_eq!(waveform.record(), Some(evidence.digital_capture_bytes()));
        assert_eq!(counter, 9);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn localhost_http_socket_replays_complete_waveform_download() {
        use std::io::{Read as _, Write as _};
        use std::net::{SocketAddr, TcpListener, TcpStream};
        use std::thread;

        use alumina_service::NativeRequest;

        const SECRET: &[u8] = b"alumina-development";

        fn read_request(stream: &mut TcpStream) -> FixtureHttpRequest {
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 512];
            let header_end = loop {
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0, "client closed before complete HTTP headers");
                bytes.extend_from_slice(&buffer[..read]);
                if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    break index + 4;
                }
                assert!(bytes.len() < 16 * 1_024);
            };
            let header_text = std::str::from_utf8(&bytes[..header_end]).unwrap();
            let mut lines = header_text[..header_text.len() - 4].split("\r\n");
            let mut request_line = lines.next().unwrap().split(' ');
            assert_eq!(request_line.next(), Some("POST"));
            let path = request_line.next().unwrap().to_owned();
            assert_eq!(request_line.next(), Some("HTTP/1.1"));
            let mut headers = Vec::new();
            let mut content_length = None;
            for line in lines {
                let (name, value) = line.split_once(": ").unwrap();
                if name.eq_ignore_ascii_case("Content-Length") {
                    content_length = Some(value.parse::<usize>().unwrap());
                }
                headers.push((name.as_bytes().to_vec(), value.as_bytes().to_vec()));
            }
            let content_length = content_length.unwrap();
            while bytes.len() < header_end + content_length {
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0, "client closed before complete HTTP body");
                bytes.extend_from_slice(&buffer[..read]);
            }
            assert_eq!(bytes.len(), header_end + content_length);
            FixtureHttpRequest {
                method: HttpMethod::Post,
                path,
                headers,
                body: bytes[header_end..].to_vec(),
            }
        }

        fn write_response(
            stream: &mut TcpStream,
            response: &alumina_sim::http_fixture::FixtureHttpResponse,
        ) {
            write!(
                stream,
                "HTTP/1.1 {} {}\r\n",
                response.status, response.reason
            )
            .unwrap();
            for (name, value) in &response.headers {
                write!(stream, "{name}: {value}\r\n").unwrap();
            }
            stream.write_all(b"Connection: close\r\n\r\n").unwrap();
            stream.write_all(&response.body).unwrap();
            stream.flush().unwrap();
        }

        fn start_server() -> (
            SocketAddr,
            alumina_diagnostics::DiagnosticContext,
            thread::JoinHandle<()>,
        ) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let mut fixture = ClockHttpFixture::new(
                SECRET.to_vec(),
                [0x31; 16],
                ClockFixturePolicy::HEALTHY_1MHZ,
            )
            .unwrap();
            let context = fixture.diagnostic_context();
            let evidence = tinybee_diagnostic_fixture_for_context(context).unwrap();
            let handle = thread::spawn(move || {
                for counter in 1..=8 {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&mut stream);
                    let operation = NativeRequest::decode(&request.body)
                        .unwrap()
                        .message
                        .operation;
                    let response = fixture.handle(
                        &request,
                        counter * 10,
                        DeviceCycle(2_000_090),
                        DeviceCycle(2_000_100),
                    );
                    if operation == Operation::WaveformArm {
                        fixture
                            .diagnostics_mut()
                            .retain_waveform_capture(evidence.digital_capture_bytes())
                            .unwrap();
                    }
                    write_response(&mut stream, &response);
                }
            });
            (address, context, handle)
        }

        fn exchange_http(
            address: SocketAddr,
            origin: &str,
            session: &mut AuthenticatedHttpSession,
            operation: DiagnosticOperation,
        ) -> Response {
            let outbound = session
                .begin_request(operation.operation, &operation.body, SECRET)
                .unwrap();
            let counter = outbound.counter_header_value();
            let authorization = outbound.authorization_header_value().to_owned();
            let mut stream = TcpStream::connect(address).unwrap();
            write!(
                stream,
                "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}: {}\r\n{}: {}\r\nOrigin: {}\r\nConnection: close\r\n\r\n",
                outbound.path(),
                address,
                outbound.content_type(),
                outbound.body().len(),
                outbound.counter_header_name(),
                counter,
                outbound.authorization_header_name(),
                authorization,
                origin,
            )
            .unwrap();
            stream.write_all(outbound.body()).unwrap();
            stream.flush().unwrap();
            let mut encoded = Vec::new();
            stream.read_to_end(&mut encoded).unwrap();
            let header_end = encoded
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = std::str::from_utf8(&encoded[..header_end]).unwrap();
            let mut lines = headers[..headers.len() - 4].split("\r\n");
            let status = lines
                .next()
                .unwrap()
                .split(' ')
                .nth(1)
                .unwrap()
                .parse::<u16>()
                .unwrap();
            let fields = lines
                .map(|line| line.split_once(": ").unwrap())
                .collect::<Vec<_>>();
            let response_counter = fields
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(outbound.counter_header_name()))
                .unwrap()
                .1;
            let response_authorization = fields
                .iter()
                .find(|(name, _)| {
                    name.eq_ignore_ascii_case(
                        AuthenticatedProtocolResponse::authorization_header_name(),
                    )
                })
                .unwrap()
                .1;
            session
                .accept_response(
                    AuthenticatedProtocolResponse {
                        http_status: status,
                        media: AuthenticatedMedia::NativeFrame,
                        counter_header: response_counter,
                        authorization_header: response_authorization,
                        body: &encoded[header_end..],
                    },
                    SECRET,
                )
                .unwrap()
        }

        let (address, context, server) = start_server();
        let origin_text = format!("http://{address}");
        let mut session = AuthenticatedHttpSession::new(
            BootNonce::new([0x31; 16]).unwrap(),
            context.config_digest,
            CorsOrigin::parse(&origin_text).unwrap(),
        );
        let mut waveform = WaveformCaptureMachine::new(
            configuration_bytes(context),
            DiagnosticTransportLimits::native_control(),
            DiagnosticLimits::interactive(),
        )
        .unwrap();

        let configure = waveform.next_request().unwrap().unwrap();
        let response = exchange_http(address, &origin_text, &mut session, configure);
        waveform.accept_response(&response).unwrap();
        waveform.request_arm().unwrap();
        let arm = waveform.next_request().unwrap().unwrap();
        let response = exchange_http(address, &origin_text, &mut session, arm);
        waveform.accept_response(&response).unwrap();
        while waveform.phase() != WaveformClientPhase::Complete {
            let operation = waveform.next_request().unwrap().unwrap();
            let response = exchange_http(address, &origin_text, &mut session, operation);
            waveform.accept_response(&response).unwrap();
        }
        let expected = tinybee_diagnostic_fixture_for_context(context).unwrap();
        assert_eq!(waveform.record(), Some(expected.digital_capture_bytes()));
        waveform.request_stop().unwrap();
        let stop = waveform.next_request().unwrap().unwrap();
        let response = exchange_http(address, &origin_text, &mut session, stop);
        waveform.accept_response(&response).unwrap();
        assert_eq!(waveform.phase(), WaveformClientPhase::Stopped);
        server.join().unwrap();
    }
}
