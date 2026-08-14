//! Passive runtime-health polling with monotonic device-report validation.

use core::fmt;

use alumina_protocol::{DeviceCycle, Digest, Operation, StatusCode};
use alumina_runtime::health::{RuntimeHealthError, RuntimeHealthFlags, RuntimeHealthSnapshot};
use alumina_runtime::stack::{StackDomain, StackWatermarkFlags, StackWatermarkSnapshot};

use crate::Response;

/// Bodyless operation emitted for one authenticated runtime-health poll.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeHealthRequest;

impl RuntimeHealthRequest {
    /// Exact native protocol operation.
    pub const fn operation(self) -> Operation {
        Operation::HealthSnapshot
    }

    /// Canonical empty request body.
    pub const fn body(self) -> &'static [u8] {
        &[]
    }

    /// Health is deliberately independent of active machine configuration.
    pub const fn config_digest(self) -> Digest {
        Digest::ZERO
    }
}

/// Exact occupancy facts for one bounded firmware queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueHealth {
    depth: u16,
    capacity: u16,
}

impl QueueHealth {
    const fn new(depth: u16, capacity: u16) -> Self {
        Self { depth, capacity }
    }

    /// Entries currently owned by the queue.
    pub const fn depth(self) -> u16 {
        self.depth
    }

    /// Compile-time queue capacity reported by the firmware image.
    pub const fn capacity(self) -> u16 {
        self.capacity
    }

    /// Remaining admission credits.
    pub const fn free(self) -> u16 {
        self.capacity - self.depth
    }

    /// Exact utilization ratio without introducing a floating-point policy.
    pub const fn utilization_ratio(self) -> (u16, u16) {
        (self.depth, self.capacity)
    }

    /// Whether no further entry can be admitted.
    pub const fn is_full(self) -> bool {
        self.depth == self.capacity
    }
}

/// Validated, UI-facing facts for one executor stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutorStackHealth {
    snapshot: StackWatermarkSnapshot,
    response_cycle: DeviceCycle,
}

impl ExecutorStackHealth {
    const fn new(snapshot: StackWatermarkSnapshot, response_cycle: DeviceCycle) -> Self {
        Self {
            snapshot,
            response_cycle,
        }
    }

    /// Executor domain that owns the measured stack.
    pub const fn domain(self) -> StackDomain {
        self.snapshot.domain
    }

    /// Complete linker or HAL allocation, including the low exclusion.
    pub const fn allocated_bytes(self) -> u32 {
        self.snapshot.allocated_bytes
    }

    /// Low guard and policy bytes omitted from canary access.
    pub const fn excluded_low_bytes(self) -> u32 {
        self.snapshot.excluded_low_bytes
    }

    /// Bytes covered by the measurement policy after the low exclusion.
    pub const fn monitored_bytes(self) -> u32 {
        self.snapshot
            .allocated_bytes
            .saturating_sub(self.snapshot.excluded_low_bytes)
    }

    /// Initialization-time canary extent.
    pub const fn painted_bytes(self) -> u32 {
        self.snapshot.painted_bytes
    }

    /// Upper bytes deliberately never painted and therefore charged as used.
    pub const fn unpainted_bytes(self) -> u32 {
        self.monitored_bytes()
            .saturating_sub(self.snapshot.painted_bytes)
    }

    /// Smallest headroom observed by current-SP bounds or completed scan windows.
    pub const fn minimum_headroom_bytes(self) -> u32 {
        self.snapshot.minimum_headroom_bytes
    }

    /// Observed maximum use, including the unpainted reserve but excluding the low guard.
    pub const fn observed_maximum_used_bytes(self) -> u32 {
        self.snapshot.maximum_used_bytes()
    }

    /// Number of bounded scan passes attempted in this partial epoch.
    pub const fn samples(self) -> u32 {
        self.snapshot.samples
    }

    /// Number of sweeps that reached the then-current observed headroom.
    pub const fn completed_sweeps(self) -> u32 {
        self.snapshot.completed_sweeps
    }

    /// Whether at least one bounded sweep has completed.
    pub const fn has_completed_sweep(self) -> bool {
        self.snapshot.flags.0 & StackWatermarkFlags::COMPLETE_SWEEP != 0
    }

    /// Whether startup before this measurement epoch remains explicitly unknown.
    pub const fn is_partial_boot_epoch(self) -> bool {
        self.snapshot.flags.0 & StackWatermarkFlags::PARTIAL_BOOT_EPOCH != 0
    }

    /// Local cycle at which the partial measurement epoch began.
    pub const fn epoch_cycle(self) -> DeviceCycle {
        self.snapshot.epoch_cycle
    }

    /// Local cycle of the newest bounded sample represented here.
    pub const fn sampled_at(self) -> DeviceCycle {
        self.snapshot.sampled_at
    }

    /// Age of the represented sample at the service response cycle.
    pub const fn sample_age_cycles(self) -> u64 {
        self.response_cycle
            .0
            .saturating_sub(self.snapshot.sampled_at.0)
    }

    /// Complete validated wire snapshot for detailed diagnostics.
    pub const fn snapshot(self) -> StackWatermarkSnapshot {
        self.snapshot
    }
}

/// Independently validated UI view of one complete health response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeHealthView {
    snapshot: RuntimeHealthSnapshot,
}

impl RuntimeHealthView {
    fn new(snapshot: RuntimeHealthSnapshot) -> Result<Self, RuntimeHealthError> {
        snapshot.validate()?;
        Ok(Self { snapshot })
    }

    /// Device cycle at which core 0 formed the response.
    pub const fn snapshot_cycle(self) -> DeviceCycle {
        self.snapshot.snapshot_cycle
    }

    /// Ordered service-to-real-time command occupancy.
    pub const fn command_queue(self) -> QueueHealth {
        QueueHealth::new(
            self.snapshot.command_queue_depth,
            self.snapshot.command_queue_capacity,
        )
    }

    /// Deterministic work-block occupancy.
    pub const fn work_queue(self) -> QueueHealth {
        QueueHealth::new(
            self.snapshot.work_queue_depth,
            self.snapshot.work_queue_capacity,
        )
    }

    /// Lossy real-time-to-service telemetry occupancy.
    pub const fn telemetry_queue(self) -> QueueHealth {
        QueueHealth::new(
            self.snapshot.telemetry_queue_depth,
            self.snapshot.telemetry_queue_capacity,
        )
    }

    /// Same-response service-core stack observation.
    pub const fn service_stack(self) -> ExecutorStackHealth {
        ExecutorStackHealth::new(self.snapshot.service_stack, self.snapshot.snapshot_cycle)
    }

    /// Latest valid real-time-core observation, if firmware has one this boot.
    pub const fn realtime_stack(self) -> Option<ExecutorStackHealth> {
        if self.snapshot.flags.0 & RuntimeHealthFlags::REALTIME_STACK_PRESENT != 0 {
            Some(ExecutorStackHealth::new(
                self.snapshot.realtime_stack,
                self.snapshot.snapshot_cycle,
            ))
        } else {
            None
        }
    }

    /// Whether the present real-time report is inside firmware's freshness window.
    pub const fn realtime_stack_fresh(self) -> bool {
        self.snapshot.flags.0 & RuntimeHealthFlags::REALTIME_STACK_FRESH != 0
    }

    /// Complete validated wire snapshot for capture/export.
    pub const fn snapshot(self) -> RuntimeHealthSnapshot {
        self.snapshot
    }
}

/// Device support state retained by the passive polling model.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeHealthAvailability {
    /// No authenticated health response has been accepted in this session.
    #[default]
    Unobserved,
    /// Firmware explicitly reported that its service probe is unavailable.
    Unsupported,
    /// A complete validated snapshot is retained.
    Available,
}

/// Result of accepting one authenticated and correlated health response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeHealthUpdate {
    /// Firmware explicitly has no usable service-core probe.
    Unsupported,
    /// A complete snapshot was accepted.
    Snapshot {
        /// Whether the accepted facts differ from the previously retained snapshot.
        advanced: bool,
        /// Independently validated UI-facing view.
        view: RuntimeHealthView,
    },
}

/// Session-scoped passive runtime-health model.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeHealthModel {
    availability: RuntimeHealthAvailability,
    latest: Option<RuntimeHealthSnapshot>,
    last_realtime: Option<StackWatermarkSnapshot>,
}

impl RuntimeHealthModel {
    /// Creates an empty model for one authenticated boot-scoped session.
    pub const fn new() -> Self {
        Self {
            availability: RuntimeHealthAvailability::Unobserved,
            latest: None,
            last_realtime: None,
        }
    }

    /// Canonical request for the next passive poll.
    pub const fn request(&self) -> RuntimeHealthRequest {
        RuntimeHealthRequest
    }

    /// Current support/observation state.
    pub const fn availability(&self) -> RuntimeHealthAvailability {
        self.availability
    }

    /// Latest validated snapshot, if one remains available.
    pub fn latest(&self) -> Option<RuntimeHealthView> {
        self.latest.map(|snapshot| RuntimeHealthView { snapshot })
    }

    /// Applies one already authenticated and natively correlated response.
    ///
    /// Rejected responses do not mutate retained valid evidence. An explicit
    /// `Unsupported` response clears boot-scoped observations.
    pub fn accept_response(
        &mut self,
        response: &Response,
    ) -> Result<RuntimeHealthUpdate, RuntimeHealthClientError> {
        if response.status == StatusCode::Unsupported {
            if !response.body.is_empty() {
                return Err(RuntimeHealthClientError::ResponseBody);
            }
            self.availability = RuntimeHealthAvailability::Unsupported;
            self.latest = None;
            self.last_realtime = None;
            return Ok(RuntimeHealthUpdate::Unsupported);
        }
        if response.status != StatusCode::Ok {
            if !response.body.is_empty() {
                return Err(RuntimeHealthClientError::ResponseBody);
            }
            return Err(RuntimeHealthClientError::DeviceStatus(response.status));
        }

        let snapshot = RuntimeHealthSnapshot::decode(&response.body)
            .map_err(RuntimeHealthClientError::Wire)?;
        let view = RuntimeHealthView::new(snapshot).map_err(RuntimeHealthClientError::Wire)?;
        self.validate_continuity(snapshot)?;
        let advanced = self.latest != Some(snapshot)
            || self.availability != RuntimeHealthAvailability::Available;
        self.availability = RuntimeHealthAvailability::Available;
        self.latest = Some(snapshot);
        if snapshot.flags.0 & RuntimeHealthFlags::REALTIME_STACK_PRESENT != 0 {
            self.last_realtime = Some(snapshot.realtime_stack);
        }
        Ok(RuntimeHealthUpdate::Snapshot { advanced, view })
    }

    /// Erases every boot-scoped observation when authentication discovers a new boot.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    fn validate_continuity(
        &self,
        candidate: RuntimeHealthSnapshot,
    ) -> Result<(), RuntimeHealthClientError> {
        if let Some(previous) = self.latest {
            if candidate.snapshot_cycle < previous.snapshot_cycle {
                return Err(RuntimeHealthClientError::SnapshotOrder);
            }
            validate_stack_continuity(previous.service_stack, candidate.service_stack)
                .map_err(|()| RuntimeHealthClientError::ServiceStackOrder)?;
        }
        if candidate.flags.0 & RuntimeHealthFlags::REALTIME_STACK_PRESENT != 0
            && let Some(previous) = self.last_realtime
        {
            validate_stack_continuity(previous, candidate.realtime_stack)
                .map_err(|()| RuntimeHealthClientError::RealtimeStackOrder)?;
        }
        Ok(())
    }
}

fn validate_stack_continuity(
    previous: StackWatermarkSnapshot,
    candidate: StackWatermarkSnapshot,
) -> Result<(), ()> {
    if candidate.domain != previous.domain
        || candidate.epoch_cycle != previous.epoch_cycle
        || candidate.allocated_bytes != previous.allocated_bytes
        || candidate.excluded_low_bytes != previous.excluded_low_bytes
        || candidate.painted_bytes != previous.painted_bytes
        || candidate.minimum_headroom_bytes > previous.minimum_headroom_bytes
        || candidate.samples < previous.samples
        || candidate.completed_sweeps < previous.completed_sweeps
        || candidate.sampled_at < previous.sampled_at
    {
        Err(())
    } else {
        Ok(())
    }
}

/// Passive health response rejected before it can replace retained evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeHealthClientError {
    /// Device returned a non-success application status with an empty body.
    DeviceStatus(StatusCode),
    /// A failure response carried an ambiguous body.
    ResponseBody,
    /// Fixed runtime-health bytes failed independent decoding or validation.
    Wire(RuntimeHealthError),
    /// Service response cycles regressed within one authenticated boot session.
    SnapshotOrder,
    /// Service-core epoch, layout, counters, time, or headroom regressed.
    ServiceStackOrder,
    /// Real-time-core epoch, layout, counters, time, or headroom regressed.
    RealtimeStackOrder,
}

impl fmt::Display for RuntimeHealthClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "runtime health rejected state: {self:?}")
    }
}

impl std::error::Error for RuntimeHealthClientError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alumina_protocol::Digest;
    use alumina_runtime::stack::StackWatermarkFlags;

    use crate::{ProtocolClient, SimulatedResponse, SimulatorTransport};

    fn stack(
        domain: StackDomain,
        minimum_headroom_bytes: u32,
        samples: u32,
        sampled_at: u64,
    ) -> StackWatermarkSnapshot {
        StackWatermarkSnapshot {
            domain,
            flags: StackWatermarkFlags(
                StackWatermarkFlags::INITIALIZED
                    | StackWatermarkFlags::PARTIAL_BOOT_EPOCH
                    | StackWatermarkFlags::CURRENT_POINTER_BOUND,
            ),
            allocated_bytes: 32 * 1_024,
            excluded_low_bytes: 256,
            painted_bytes: 28 * 1_024,
            minimum_headroom_bytes,
            samples,
            completed_sweeps: 0,
            epoch_cycle: DeviceCycle(10),
            sampled_at: DeviceCycle(sampled_at),
        }
    }

    fn snapshot(cycle: u64) -> RuntimeHealthSnapshot {
        RuntimeHealthSnapshot {
            flags: RuntimeHealthFlags(
                RuntimeHealthFlags::REALTIME_STACK_PRESENT
                    | RuntimeHealthFlags::REALTIME_STACK_FRESH,
            ),
            snapshot_cycle: DeviceCycle(cycle),
            command_queue_depth: 2,
            command_queue_capacity: 8,
            work_queue_depth: 3,
            work_queue_capacity: 8,
            telemetry_queue_depth: 4,
            telemetry_queue_capacity: 32,
            service_stack: stack(StackDomain::ServiceCore, 20 * 1_024, 4, cycle - 2),
            realtime_stack: stack(StackDomain::RealtimeCore, 24 * 1_024, 5, cycle - 3),
        }
    }

    fn response(status: StatusCode, body: Vec<u8>) -> Response {
        Response { status, body }
    }

    #[test]
    fn bodyless_request_and_native_framing_round_trip_exact_health() {
        let expected = snapshot(200);
        let encoded = expected.encode().unwrap();
        let simulator = SimulatorTransport::new(move |operation, body: &[u8]| {
            assert_eq!(operation, Operation::HealthSnapshot);
            assert!(body.is_empty());
            SimulatedResponse {
                status: StatusCode::Ok,
                body: encoded.to_vec(),
            }
        });
        let mut model = RuntimeHealthModel::new();
        let request = model.request();
        assert_eq!(request.config_digest(), Digest::ZERO);
        let mut client = ProtocolClient::new(simulator, request.config_digest());

        let native = client.request(request.operation(), request.body()).unwrap();
        let RuntimeHealthUpdate::Snapshot { advanced, view } =
            model.accept_response(&native).unwrap()
        else {
            panic!("expected a complete snapshot");
        };

        assert!(advanced);
        assert_eq!(view.snapshot(), expected);
        assert_eq!(view.command_queue().utilization_ratio(), (2, 8));
        assert_eq!(view.command_queue().free(), 6);
        assert_eq!(view.work_queue().free(), 5);
        assert_eq!(view.telemetry_queue().free(), 28);
        assert_eq!(view.service_stack().unpainted_bytes(), 3_840);
        assert_eq!(
            view.service_stack().observed_maximum_used_bytes(),
            32 * 1_024 - 256 - 20 * 1_024
        );
        assert_eq!(view.service_stack().sample_age_cycles(), 2);
        assert!(view.service_stack().is_partial_boot_epoch());
        assert_eq!(view.realtime_stack().unwrap().sample_age_cycles(), 3);
        assert!(view.realtime_stack_fresh());
        assert_eq!(client.transport().request_count(), 1);
    }

    #[test]
    fn exact_duplicate_is_not_reported_as_progress() {
        let encoded = snapshot(200).encode().unwrap().to_vec();
        let mut model = RuntimeHealthModel::new();
        model
            .accept_response(&response(StatusCode::Ok, encoded.clone()))
            .unwrap();
        assert_eq!(
            model.accept_response(&response(StatusCode::Ok, encoded)),
            Ok(RuntimeHealthUpdate::Snapshot {
                advanced: false,
                view: model.latest().unwrap(),
            })
        );
    }

    #[test]
    fn response_cycle_regression_cannot_replace_retained_facts() {
        let mut model = RuntimeHealthModel::new();
        let original = snapshot(200);
        model
            .accept_response(&response(
                StatusCode::Ok,
                original.encode().unwrap().to_vec(),
            ))
            .unwrap();
        let mut regressed = original;
        regressed.snapshot_cycle = DeviceCycle(199);
        assert_eq!(
            model.accept_response(&response(
                StatusCode::Ok,
                regressed.encode().unwrap().to_vec(),
            )),
            Err(RuntimeHealthClientError::SnapshotOrder)
        );
        assert_eq!(model.latest().unwrap().snapshot(), original);
    }

    #[test]
    fn unsupported_is_canonical_and_clears_boot_scoped_evidence() {
        let mut model = RuntimeHealthModel::new();
        model
            .accept_response(&response(
                StatusCode::Ok,
                snapshot(200).encode().unwrap().to_vec(),
            ))
            .unwrap();

        assert_eq!(
            model.accept_response(&response(StatusCode::Unsupported, Vec::new())),
            Ok(RuntimeHealthUpdate::Unsupported)
        );
        assert_eq!(model.availability(), RuntimeHealthAvailability::Unsupported);
        assert!(model.latest().is_none());
        assert_eq!(
            model.accept_response(&response(StatusCode::Unsupported, vec![1])),
            Err(RuntimeHealthClientError::ResponseBody)
        );
    }

    #[test]
    fn malformed_or_failed_response_preserves_last_valid_snapshot() {
        let mut model = RuntimeHealthModel::new();
        let original = snapshot(200);
        model
            .accept_response(&response(
                StatusCode::Ok,
                original.encode().unwrap().to_vec(),
            ))
            .unwrap();

        let mut malformed = original.encode().unwrap();
        malformed[6] = 1;
        assert!(matches!(
            model.accept_response(&response(StatusCode::Ok, malformed.to_vec())),
            Err(RuntimeHealthClientError::Wire(_))
        ));
        assert_eq!(
            model.accept_response(&response(StatusCode::Internal, Vec::new())),
            Err(RuntimeHealthClientError::DeviceStatus(StatusCode::Internal))
        );
        assert_eq!(model.latest().unwrap().snapshot(), original);
    }

    #[test]
    fn service_and_realtime_continuity_fail_closed_across_absence() {
        let mut model = RuntimeHealthModel::new();
        let first = snapshot(200);
        model
            .accept_response(&response(StatusCode::Ok, first.encode().unwrap().to_vec()))
            .unwrap();

        let mut service_regression = first;
        service_regression.snapshot_cycle = DeviceCycle(210);
        service_regression.service_stack.minimum_headroom_bytes += 4;
        assert_eq!(
            model.accept_response(&response(
                StatusCode::Ok,
                service_regression.encode().unwrap().to_vec(),
            )),
            Err(RuntimeHealthClientError::ServiceStackOrder)
        );

        let mut absent = first;
        absent.snapshot_cycle = DeviceCycle(220);
        absent.service_stack.samples += 1;
        absent.service_stack.sampled_at = DeviceCycle(218);
        absent.flags = RuntimeHealthFlags(0);
        absent.realtime_stack = StackWatermarkSnapshot::unavailable(StackDomain::RealtimeCore);
        model
            .accept_response(&response(StatusCode::Ok, absent.encode().unwrap().to_vec()))
            .unwrap();
        assert!(model.latest().unwrap().realtime_stack().is_none());

        let mut realtime_regression = first;
        realtime_regression.snapshot_cycle = DeviceCycle(230);
        realtime_regression.service_stack.samples += 2;
        realtime_regression.service_stack.sampled_at = DeviceCycle(228);
        realtime_regression.realtime_stack.minimum_headroom_bytes += 4;
        realtime_regression.realtime_stack.samples += 1;
        realtime_regression.realtime_stack.sampled_at = DeviceCycle(227);
        assert_eq!(
            model.accept_response(&response(
                StatusCode::Ok,
                realtime_regression.encode().unwrap().to_vec(),
            )),
            Err(RuntimeHealthClientError::RealtimeStackOrder)
        );
    }

    #[test]
    fn stale_present_report_remains_visible_but_not_fresh() {
        let mut stale = snapshot(300);
        stale.flags = RuntimeHealthFlags(RuntimeHealthFlags::REALTIME_STACK_PRESENT);
        let mut model = RuntimeHealthModel::new();
        let update = model
            .accept_response(&response(StatusCode::Ok, stale.encode().unwrap().to_vec()))
            .unwrap();
        let RuntimeHealthUpdate::Snapshot { view, .. } = update else {
            panic!("expected snapshot");
        };
        assert!(view.realtime_stack().is_some());
        assert!(!view.realtime_stack_fresh());
    }

    #[test]
    fn reset_allows_a_new_boot_epoch() {
        let mut model = RuntimeHealthModel::new();
        model
            .accept_response(&response(
                StatusCode::Ok,
                snapshot(200).encode().unwrap().to_vec(),
            ))
            .unwrap();
        model.reset();
        let mut next_boot = snapshot(20);
        next_boot.service_stack.epoch_cycle = DeviceCycle(1);
        next_boot.realtime_stack.epoch_cycle = DeviceCycle(1);
        assert!(
            model
                .accept_response(&response(
                    StatusCode::Ok,
                    next_boot.encode().unwrap().to_vec(),
                ))
                .is_ok()
        );
    }
}
