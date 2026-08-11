//! Retry-safe heartbeat acquisition around Alumina's exact causal clock estimator.

use std::fmt;

use alumina_clock::{
    BootId, CLOCK_HEARTBEAT_REQUEST_BYTES, ClockEstimateError, ClockEstimationPolicy,
    ClockEstimator, ClockHeartbeatRequest, ClockHeartbeatResponse, ClockInstantEstimate,
    ClockObservation, ClockPrediction, ClockWireError,
};
use alumina_protocol::{Operation, StatusCode};

use crate::Response;

const NANOSECONDS_PER_MILLISECOND: f64 = 1_000_000.0;
const MAXIMUM_EXACT_FLOAT_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Conservative integer interval around one browser monotonic timer reading.
///
/// The caller-supplied error bound covers browser timer coarsening, jitter, and
/// privacy rounding. Conversion from the returned JavaScript number is widened
/// outward independently, so neither endpoint claims more precision than the
/// browser supplied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonotonicTimeBounds {
    earliest_ns: u64,
    latest_ns: u64,
}

impl MonotonicTimeBounds {
    /// Converts one nonnegative `DOMHighResTimeStamp` in milliseconds.
    ///
    /// # Errors
    ///
    /// Rejects a zero error policy, non-finite/negative sample, a sample beyond
    /// JavaScript's exact integer range after conversion, or endpoint overflow.
    pub fn from_milliseconds(
        reported_milliseconds: f64,
        maximum_timer_error_ns: u64,
    ) -> Result<Self, BrowserTimeError> {
        if maximum_timer_error_ns == 0 {
            return Err(BrowserTimeError::ZeroErrorBound);
        }
        if !reported_milliseconds.is_finite() || reported_milliseconds.is_sign_negative() {
            return Err(BrowserTimeError::InvalidSample);
        }
        let reported_ns = reported_milliseconds * NANOSECONDS_PER_MILLISECOND;
        if !reported_ns.is_finite() || reported_ns > MAXIMUM_EXACT_FLOAT_INTEGER {
            return Err(BrowserTimeError::SampleRange);
        }
        let numerical_lower = reported_ns.floor() as u64;
        let numerical_upper = reported_ns.ceil() as u64;
        let earliest_ns = numerical_lower.saturating_sub(maximum_timer_error_ns);
        let latest_ns = numerical_upper
            .checked_add(maximum_timer_error_ns)
            .ok_or(BrowserTimeError::EndpointOverflow)?;
        Ok(Self {
            earliest_ns,
            latest_ns,
        })
    }

    /// Conservative lower bound suitable for committing a request send time.
    pub const fn earliest_ns(self) -> u64 {
        self.earliest_ns
    }

    /// Conservative upper bound suitable for accepting a response receive time.
    pub const fn latest_ns(self) -> u64 {
        self.latest_ns
    }
}

/// Browser monotonic timer policy or conversion failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserTimeError {
    /// A real browser timer must declare nonzero uncertainty.
    ZeroErrorBound,
    /// The reported millisecond timestamp was negative or non-finite.
    InvalidSample,
    /// Nanosecond conversion exceeded JavaScript's exact integer range.
    SampleRange,
    /// Widening the converted value overflowed the integer time domain.
    EndpointOverflow,
}

impl fmt::Display for BrowserTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroErrorBound => formatter.write_str("browser timer error bound is zero"),
            Self::InvalidSample => formatter.write_str("browser timer sample is invalid"),
            Self::SampleRange => formatter.write_str("browser timer sample exceeds exact range"),
            Self::EndpointOverflow => formatter.write_str("browser timer bounds overflowed"),
        }
    }
}

impl std::error::Error for BrowserTimeError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingProbe {
    probe_id: u64,
    ui_send_ns: u64,
}

/// One canonical heartbeat operation whose identity was spent before I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockProbeRequest {
    probe_id: u64,
    ui_send_ns: u64,
    body: [u8; CLOCK_HEARTBEAT_REQUEST_BYTES],
}

impl ClockProbeRequest {
    /// Exact native protocol operation.
    pub const fn operation(self) -> Operation {
        Operation::ClockHeartbeat
    }

    /// Nonzero model-local probe identity.
    pub const fn probe_id(self) -> u64 {
        self.probe_id
    }

    /// Conservative browser-worker send timestamp committed into the request.
    pub const fn ui_send_ns(self) -> u64 {
        self.ui_send_ns
    }

    /// Canonical fixed heartbeat body.
    pub const fn body(&self) -> &[u8; CLOCK_HEARTBEAT_REQUEST_BYTES] {
        &self.body
    }
}

/// One boot-scoped device clock model independent of browser or native I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceClockModel {
    estimator: ClockEstimator,
    boot_id: Option<BootId>,
    latest_response: Option<ClockHeartbeatResponse>,
    next_probe_id: u64,
    pending: Option<PendingProbe>,
}

impl DeviceClockModel {
    /// Creates an empty model using an explicit causal-estimation policy.
    pub fn new(policy: ClockEstimationPolicy) -> Result<Self, ClockProbeError> {
        Ok(Self {
            estimator: ClockEstimator::new(policy)?,
            boot_id: None,
            latest_response: None,
            next_probe_id: 1,
            pending: None,
        })
    }

    /// Whether one spent probe is awaiting a response or abandonment.
    pub const fn has_pending_probe(&self) -> bool {
        self.pending.is_some()
    }

    /// Boot identity retained by accepted observations, if any.
    pub const fn boot_id(&self) -> Option<BootId> {
        self.boot_id
    }

    /// Latest fully validated heartbeat facts, if any.
    pub const fn latest_response(&self) -> Option<ClockHeartbeatResponse> {
        self.latest_response
    }

    /// Number of observations retained by the exact interval intersection.
    pub const fn accepted_samples(&self) -> u32 {
        self.estimator.accepted_samples()
    }

    /// Number of observations rejected by estimator policy or consistency.
    pub const fn rejected_samples(&self) -> u32 {
        self.estimator.rejected_samples()
    }

    /// Spends the next probe identity and builds its canonical request before I/O.
    ///
    /// `ui_send_ns` must be a conservative lower bound for the actual browser
    /// send instant. A browser adapter with a quantized timer must not pass a
    /// rounded-up value here.
    pub fn begin_probe(&mut self, ui_send_ns: u64) -> Result<ClockProbeRequest, ClockProbeError> {
        if self.pending.is_some() {
            return Err(ClockProbeError::RequestPending);
        }
        let probe_id = self.next_probe_id;
        let next_probe_id = probe_id
            .checked_add(1)
            .ok_or(ClockProbeError::ProbeExhausted)?;
        let request = ClockHeartbeatRequest {
            probe_id,
            ui_send_ns,
        };
        let body = request.encode()?;
        self.pending = Some(PendingProbe {
            probe_id,
            ui_send_ns,
        });
        self.next_probe_id = next_probe_id;
        Ok(ClockProbeRequest {
            probe_id,
            ui_send_ns,
            body,
        })
    }

    /// Accepts one already authenticated and natively correlated response.
    ///
    /// The pending probe is consumed even when the authenticated response is
    /// semantically invalid. `ui_receive_ns` must be a conservative upper bound
    /// for response receipt, so browser timer quantization widens rather than
    /// narrows the causal interval.
    pub fn accept_response(
        &mut self,
        response: &Response,
        ui_receive_ns: u64,
    ) -> Result<ClockObservation, ClockProbeError> {
        let pending = self
            .pending
            .take()
            .ok_or(ClockProbeError::NoPendingRequest)?;
        if response.status != StatusCode::Ok {
            if !response.body.is_empty() {
                return Err(ClockProbeError::ResponseBody);
            }
            return Err(ClockProbeError::DeviceStatus(response.status));
        }
        let heartbeat = ClockHeartbeatResponse::decode(&response.body)?;
        if heartbeat.probe_id != pending.probe_id || heartbeat.ui_send_ns != pending.ui_send_ns {
            return Err(ClockProbeError::Correlation);
        }
        if let Some(expected) = self.boot_id
            && expected != heartbeat.boot_id
        {
            return Err(ClockProbeError::BootChanged {
                expected,
                received: heartbeat.boot_id,
            });
        }
        let observation = ClockObservation {
            response: heartbeat,
            ui_receive_ns,
        };
        self.estimator.observe(observation)?;
        self.boot_id = Some(heartbeat.boot_id);
        self.latest_response = Some(heartbeat);
        Ok(observation)
    }

    /// Abandons an ambiguous transport result without reusing its probe identity.
    pub fn abandon_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }

    /// Explicitly discards every boot-scoped fact before observing another boot.
    pub fn reset(&mut self) -> Result<(), ClockProbeError> {
        if self.pending.is_some() {
            return Err(ClockProbeError::RequestPending);
        }
        self.estimator.reset();
        self.boot_id = None;
        self.latest_response = None;
        self.next_probe_id = 1;
        Ok(())
    }

    /// Maps a future browser-worker time to a conservative local cycle interval.
    pub fn predict(
        &self,
        now_ui_ns: u64,
        target_ui_ns: u64,
        maximum_uncertainty_cycles: u64,
    ) -> Result<ClockPrediction, ClockProbeError> {
        self.estimator
            .predict(now_ui_ns, target_ui_ns, maximum_uncertainty_cycles)
            .map_err(Into::into)
    }

    /// Maps a current browser monotonic instant for deadline reconciliation.
    pub fn estimate_at(
        &self,
        ui_ns: u64,
        maximum_uncertainty_cycles: u64,
    ) -> Result<ClockInstantEstimate, ClockProbeError> {
        self.estimator
            .estimate_at(ui_ns, maximum_uncertainty_cycles)
            .map_err(Into::into)
    }
}

/// Heartbeat request, response, boot, or exact-estimation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockProbeError {
    /// Another spent probe must be accepted or abandoned first.
    RequestPending,
    /// No probe exists for the supplied response.
    NoPendingRequest,
    /// The model-local probe counter cannot advance without reuse.
    ProbeExhausted,
    /// Fixed heartbeat bytes were invalid.
    Wire(ClockWireError),
    /// Device rejected the heartbeat without a typed response body.
    DeviceStatus(StatusCode),
    /// An error response unexpectedly carried unaudited operation bytes.
    ResponseBody,
    /// Response echoes did not name the unique pending probe.
    Correlation,
    /// A response belongs to a different boot and cannot update this model.
    BootChanged {
        /// Boot retained by accepted observations.
        expected: BootId,
        /// Boot reported by the authenticated response.
        received: BootId,
    },
    /// Causal estimator policy, consistency, freshness, or arithmetic rejection.
    Estimate(ClockEstimateError),
}

impl From<ClockWireError> for ClockProbeError {
    fn from(value: ClockWireError) -> Self {
        Self::Wire(value)
    }
}

impl From<ClockEstimateError> for ClockProbeError {
    fn from(value: ClockEstimateError) -> Self {
        Self::Estimate(value)
    }
}

impl fmt::Display for ClockProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestPending => formatter.write_str("a clock probe is already pending"),
            Self::NoPendingRequest => formatter.write_str("no clock probe is pending"),
            Self::ProbeExhausted => formatter.write_str("clock probe identity is exhausted"),
            Self::Wire(error) => write!(formatter, "clock wire response rejected: {error:?}"),
            Self::DeviceStatus(status) => write!(formatter, "clock probe returned {status:?}"),
            Self::ResponseBody => formatter.write_str("failed clock probe carried a body"),
            Self::Correlation => formatter.write_str("clock response does not match pending probe"),
            Self::BootChanged { .. } => {
                formatter.write_str("clock response belongs to a different device boot")
            }
            Self::Estimate(error) => write!(formatter, "clock estimate rejected: {error:?}"),
        }
    }
}

impl std::error::Error for ClockProbeError {}

#[cfg(test)]
mod tests {
    use alumina_clock::{BOOT_ID_BYTES, ClockFlags, ClockSource};
    use alumina_protocol::DeviceCycle;

    use super::*;

    fn boot(byte: u8) -> BootId {
        BootId::new([byte; BOOT_ID_BYTES]).unwrap()
    }

    fn policy() -> ClockEstimationPolicy {
        ClockEstimationPolicy {
            maximum_round_trip_ns: 3_000_000,
            maximum_device_processing_cycles: 1_000,
            maximum_drift_ppm: 100,
            maximum_sample_age_ns: 100_000_000,
            maximum_schedule_horizon_ns: 10_000_000_000,
            minimum_schedule_lead_ns: 100_000_000,
            minimum_samples: 3,
        }
    }

    fn heartbeat(
        request: ClockProbeRequest,
        boot_id: BootId,
        request_delay_ns: u64,
        processing_ns: u64,
        response_delay_ns: u64,
    ) -> (Response, u64) {
        let offset = 75_000_u64;
        let cycle_at = |ui_ns: u64| DeviceCycle(offset + ui_ns / 1_000);
        let receive_ns = request.ui_send_ns() + request_delay_ns;
        let transmit_ns = receive_ns + processing_ns;
        let receive_ui_ns = transmit_ns + response_delay_ns;
        let heartbeat = ClockHeartbeatResponse {
            flags: ClockFlags(
                ClockFlags::MONOTONIC
                    | ClockFlags::SHARED_BETWEEN_CORES
                    | ClockFlags::DEADLINE_HEALTHY,
            ),
            counter_bits: 64,
            source: ClockSource::EmbassyMonotonic,
            probe_id: request.probe_id(),
            ui_send_ns: request.ui_send_ns(),
            boot_id,
            receive_cycle: cycle_at(receive_ns),
            transmit_cycle: cycle_at(transmit_ns),
            frequency_hz: 1_000_000,
            minimum_lead_cycles: 100_000,
            maximum_schedule_horizon_cycles: 10_000_000,
            queue_horizon_cycles: 100_000,
            maximum_lateness_cycles: 0,
            missed_deadlines: 0,
            command_queue_free: 4,
            work_queue_depth: 1,
        };
        (
            Response {
                status: StatusCode::Ok,
                body: heartbeat.encode().unwrap().to_vec(),
            },
            receive_ui_ns,
        )
    }

    #[test]
    fn ambiguous_transport_spends_probe_and_authenticated_echo_must_match() {
        let mut model = DeviceClockModel::new(policy()).unwrap();
        let first = model.begin_probe(1_000_000_000).unwrap();
        assert_eq!(first.probe_id(), 1);
        assert_eq!(first.operation(), Operation::ClockHeartbeat);
        assert_eq!(
            ClockHeartbeatRequest::decode(first.body())
                .unwrap()
                .probe_id,
            1
        );
        assert_eq!(
            model.begin_probe(1_000_000_001),
            Err(ClockProbeError::RequestPending)
        );
        assert!(model.abandon_pending());

        let second = model.begin_probe(2_000_000_000).unwrap();
        assert_eq!(second.probe_id(), 2);
        let (mut response, received) = heartbeat(second, boot(0x11), 100_000, 50_000, 200_000);
        response.body[16] ^= 1;
        assert_eq!(
            model.accept_response(&response, received),
            Err(ClockProbeError::Correlation)
        );
        assert!(!model.has_pending_probe());
    }

    #[test]
    fn browser_timer_conversion_only_widens_reported_precision() {
        let bounds = MonotonicTimeBounds::from_milliseconds(1_234.567_890_4, 250).unwrap();
        let represented_ns = 1_234_567_890_u64;
        assert!(bounds.earliest_ns() <= represented_ns.saturating_sub(250));
        assert!(bounds.latest_ns() >= represented_ns + 250);
        assert!(bounds.latest_ns() - bounds.earliest_ns() <= 501);

        let near_origin = MonotonicTimeBounds::from_milliseconds(0.000_001, 10).unwrap();
        assert_eq!(near_origin.earliest_ns(), 0);
        assert!(near_origin.latest_ns() >= 11);
    }

    #[test]
    fn browser_timer_rejects_unbounded_or_unrepresentable_samples() {
        assert_eq!(
            MonotonicTimeBounds::from_milliseconds(1.0, 0),
            Err(BrowserTimeError::ZeroErrorBound)
        );
        assert_eq!(
            MonotonicTimeBounds::from_milliseconds(f64::NAN, 1),
            Err(BrowserTimeError::InvalidSample)
        );
        assert_eq!(
            MonotonicTimeBounds::from_milliseconds(-1.0, 1),
            Err(BrowserTimeError::InvalidSample)
        );
        assert_eq!(
            MonotonicTimeBounds::from_milliseconds(f64::MAX, 1),
            Err(BrowserTimeError::SampleRange)
        );
    }

    #[test]
    fn causal_samples_predict_a_future_cycle_interval() {
        let mut model = DeviceClockModel::new(policy()).unwrap();
        let delays = [
            (120_000, 50_000, 700_000),
            (650_000, 60_000, 120_000),
            (300_000, 40_000, 600_000),
        ];
        for (index, (up, processing, down)) in delays.into_iter().enumerate() {
            let send = u64::try_from(index + 1).unwrap() * 1_000_000_000;
            let request = model.begin_probe(send).unwrap();
            let (response, received) = heartbeat(request, boot(0x21), up, processing, down);
            model.accept_response(&response, received).unwrap();
        }
        let prediction = model.predict(3_001_000_000, 5_000_000_000, 2_000).unwrap();
        let exact = 75_000 + 5_000_000_000 / 1_000;
        assert!(prediction.earliest_cycle.0 <= exact);
        assert!(exact <= prediction.latest_cycle.0);
        assert_eq!(prediction.accepted_samples, 3);
        assert_eq!(model.boot_id(), Some(boot(0x21)));
    }

    #[test]
    fn changed_boot_cannot_mutate_an_existing_model() {
        let mut model = DeviceClockModel::new(ClockEstimationPolicy {
            minimum_samples: 1,
            ..policy()
        })
        .unwrap();
        let request = model.begin_probe(1_000_000_000).unwrap();
        let (response, received) = heartbeat(request, boot(0x31), 100_000, 50_000, 100_000);
        model.accept_response(&response, received).unwrap();

        let request = model.begin_probe(2_000_000_000).unwrap();
        let (response, received) = heartbeat(request, boot(0x32), 100_000, 50_000, 100_000);
        assert_eq!(
            model.accept_response(&response, received),
            Err(ClockProbeError::BootChanged {
                expected: boot(0x31),
                received: boot(0x32),
            })
        );
        assert_eq!(model.boot_id(), Some(boot(0x31)));
        assert_eq!(model.accepted_samples(), 1);
        model.reset().unwrap();
        assert_eq!(model.boot_id(), None);
        assert_eq!(model.accepted_samples(), 0);
    }
}
