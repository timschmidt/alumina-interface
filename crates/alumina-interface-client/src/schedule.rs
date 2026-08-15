//! Exact per-participant prepare/install/confirm/abort reconciliation.

use std::fmt;

use alumina_clock::BootId;
use alumina_job::{
    JobCancelRequest, JobCancelWireError, JobCommitRequest, JobDescriptor, JobDescriptorWireError,
    JobScheduleReference, JobScheduleReferenceAction, JobScheduleReport, JobScheduleState,
    JobScheduleWireError, JobStatusReport, JobStatusReportWireError, PreparedJobToken,
    RealtimeJobState, ServiceJobState,
};
use alumina_protocol::{Operation, StatusCode};

use crate::Response;

/// One canonical job operation emitted for authenticated transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleOperation {
    /// Exact native operation.
    pub operation: Operation,
    /// Canonical operation-specific body.
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduleAction {
    Prepare,
    Status,
    Install,
    Confirm,
    Abort,
    Cancel,
}

impl ScheduleAction {
    const fn mutates(self) -> bool {
        !matches!(self, Self::Status)
    }
}

/// Exact device-observed phase for one prepared participant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParticipantSchedulePhase {
    /// No correlated job actors have been observed.
    Empty,
    /// Core actors or their boot-bound schedule receipt are not all ready.
    Preparing,
    /// Cache actors and the boot-bound prepared receipt are ready for install.
    Ready,
    /// A commit is bound locally but the device has not reported it installed.
    Installing,
    /// The exact commit is installed but has no start authority.
    Installed,
    /// Start authority is confirmed and remote abort remains open until its guard.
    Confirmed,
    /// The abort guard closed and local hardware priming is in progress.
    Priming,
    /// The local hardware owner reported its complete future horizon primed.
    Primed,
    /// The scheduled local start was emitted.
    Running,
    /// Remote reconciliation safely aborted the installed schedule.
    Aborted,
    /// Both cached and real-time preparation actors reported cancellation.
    Cancelled,
    /// An unconfirmed schedule expired locally.
    Expired,
    /// Exact local execution completed.
    Complete,
    /// The local schedule or execution owner faulted.
    Faulted,
}

/// Retry-safe controller for one device's already cached job partition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticipantScheduleMachine<const AXES: usize> {
    descriptor: JobDescriptor,
    boot_id: BootId,
    prepared_token: PreparedJobToken,
    commit: Option<JobCommitRequest>,
    report: Option<JobStatusReport>,
    pending: Option<ScheduleAction>,
    reconciliation_required: bool,
}

impl<const AXES: usize> ParticipantScheduleMachine<AXES> {
    /// Binds the exact descriptor and boot before any request can be emitted.
    pub fn new(descriptor: JobDescriptor, boot_id: BootId) -> Result<Self, ScheduleControlError> {
        descriptor
            .validate::<AXES>()
            .map_err(JobDescriptorWireError::Descriptor)?;
        let prepared_token = PreparedJobToken::derive::<AXES>(boot_id, descriptor)?;
        Ok(Self {
            descriptor,
            boot_id,
            prepared_token,
            commit: None,
            report: None,
            pending: None,
            reconciliation_required: false,
        })
    }

    /// Exact immutable local job descriptor.
    pub const fn descriptor(&self) -> JobDescriptor {
        self.descriptor
    }

    /// Boot identity bound into the expected prepare receipt and commit.
    pub const fn boot_id(&self) -> BootId {
        self.boot_id
    }

    /// Locally derived receipt that the device must report before install.
    pub const fn prepared_token(&self) -> PreparedJobToken {
        self.prepared_token
    }

    /// Bound exact commit, once selected by the global coordinator.
    pub const fn commit(&self) -> Option<JobCommitRequest> {
        self.commit
    }

    /// Latest fully validated combined device report.
    pub const fn report(&self) -> Option<JobStatusReport> {
        self.report
    }

    /// Whether one request is awaiting acceptance or explicit abandonment.
    pub const fn has_pending_request(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether an ambiguous mutation must be inspected before another mutation.
    pub const fn reconciliation_required(&self) -> bool {
        self.reconciliation_required
    }

    /// Current device-observed phase, with locally bound install intent retained.
    pub fn phase(&self) -> ParticipantSchedulePhase {
        let Some(report) = self.report else {
            return ParticipantSchedulePhase::Empty;
        };
        if report == JobStatusReport::default() {
            return ParticipantSchedulePhase::Empty;
        }
        if report
            .service
            .is_some_and(|service| service.state == ServiceJobState::Faulted)
            || report
                .realtime
                .is_some_and(|realtime| realtime.state == RealtimeJobState::Faulted)
        {
            return ParticipantSchedulePhase::Faulted;
        }
        if report
            .service
            .is_some_and(|service| service.state == ServiceJobState::Cancelled)
            && report
                .realtime
                .is_some_and(|realtime| realtime.state == RealtimeJobState::Cancelled)
        {
            return ParticipantSchedulePhase::Cancelled;
        }
        let Some(schedule) = report.schedule else {
            return ParticipantSchedulePhase::Preparing;
        };
        match schedule.state {
            JobScheduleState::Prepared if !self.ready_report(report) => {
                ParticipantSchedulePhase::Preparing
            }
            JobScheduleState::Prepared if self.commit.is_some() => {
                ParticipantSchedulePhase::Installing
            }
            JobScheduleState::Prepared => ParticipantSchedulePhase::Ready,
            JobScheduleState::Installed => ParticipantSchedulePhase::Installed,
            JobScheduleState::Confirmed => ParticipantSchedulePhase::Confirmed,
            JobScheduleState::Priming => ParticipantSchedulePhase::Priming,
            JobScheduleState::Primed => ParticipantSchedulePhase::Primed,
            JobScheduleState::Running => ParticipantSchedulePhase::Running,
            JobScheduleState::Aborted => ParticipantSchedulePhase::Aborted,
            JobScheduleState::Expired => ParticipantSchedulePhase::Expired,
            JobScheduleState::Complete => ParticipantSchedulePhase::Complete,
            JobScheduleState::Faulted => ParticipantSchedulePhase::Faulted,
        }
    }

    /// Emits an idempotent prepare for the exact descriptor.
    pub fn begin_prepare(&mut self) -> Result<ScheduleOperation, ScheduleControlError> {
        self.ensure_mutation_allowed()?;
        if !matches!(
            self.phase(),
            ParticipantSchedulePhase::Empty | ParticipantSchedulePhase::Preparing
        ) || self.commit.is_some()
        {
            return Err(ScheduleControlError::State);
        }
        let body = self.descriptor.encode::<AXES>()?.to_vec();
        self.pending = Some(ScheduleAction::Prepare);
        Ok(ScheduleOperation {
            operation: Operation::JobPrepare,
            body,
        })
    }

    /// Emits a read-only exact status reconciliation request.
    pub fn begin_status(&mut self) -> Result<ScheduleOperation, ScheduleControlError> {
        self.ensure_no_pending()?;
        self.pending = Some(ScheduleAction::Status);
        Ok(ScheduleOperation {
            operation: Operation::JobStatus,
            body: Vec::new(),
        })
    }

    /// Binds and emits one exact future schedule without granting start authority.
    pub fn begin_install(
        &mut self,
        commit: JobCommitRequest,
    ) -> Result<ScheduleOperation, ScheduleControlError> {
        self.ensure_mutation_allowed()?;
        if commit.prepare_id != self.descriptor.prepare_id
            || commit.boot_id != self.boot_id
            || commit.prepared_token != self.prepared_token
            || commit.partition_digest != self.descriptor.partition.object.content.digest
        {
            return Err(ScheduleControlError::Identity);
        }
        let body = commit.encode()?.to_vec();
        match self.commit {
            Some(bound) if bound != commit => return Err(ScheduleControlError::Conflict),
            _ => {}
        }
        if !matches!(
            self.phase(),
            ParticipantSchedulePhase::Ready | ParticipantSchedulePhase::Installing
        ) {
            return Err(ScheduleControlError::State);
        }
        self.commit = Some(commit);
        self.pending = Some(ScheduleAction::Install);
        Ok(ScheduleOperation {
            operation: Operation::JobCommit,
            body,
        })
    }

    /// Emits the exact commit-digest reference that grants local start authority.
    pub fn begin_confirm(&mut self) -> Result<ScheduleOperation, ScheduleControlError> {
        self.ensure_mutation_allowed()?;
        if self.phase() != ParticipantSchedulePhase::Installed {
            return Err(ScheduleControlError::State);
        }
        let commit = self.commit.ok_or(ScheduleControlError::State)?;
        let body = JobScheduleReference::for_commit(JobScheduleReferenceAction::Confirm, commit)?
            .encode()?
            .to_vec();
        self.pending = Some(ScheduleAction::Confirm);
        Ok(ScheduleOperation {
            operation: Operation::JobConfirm,
            body,
        })
    }

    /// Emits the exact idempotent pre-guard abort reference.
    pub fn begin_abort(&mut self) -> Result<ScheduleOperation, ScheduleControlError> {
        self.ensure_mutation_allowed()?;
        if !matches!(
            self.phase(),
            ParticipantSchedulePhase::Installing
                | ParticipantSchedulePhase::Installed
                | ParticipantSchedulePhase::Confirmed
        ) {
            return Err(ScheduleControlError::State);
        }
        let commit = self.commit.ok_or(ScheduleControlError::State)?;
        let body = JobScheduleReference::for_commit(JobScheduleReferenceAction::Abort, commit)?
            .encode()?
            .to_vec();
        self.pending = Some(ScheduleAction::Abort);
        Ok(ScheduleOperation {
            operation: Operation::JobAbort,
            body,
        })
    }

    /// Emits idempotent cleanup for one preparation that has no bound commit.
    pub fn begin_cancel(&mut self) -> Result<ScheduleOperation, ScheduleControlError> {
        self.ensure_mutation_allowed()?;
        if self.commit.is_some()
            || !matches!(
                self.phase(),
                ParticipantSchedulePhase::Preparing
                    | ParticipantSchedulePhase::Ready
                    | ParticipantSchedulePhase::Faulted
            )
        {
            return Err(ScheduleControlError::State);
        }
        let body = JobCancelRequest {
            prepare_id: self.descriptor.prepare_id,
        }
        .encode()?
        .to_vec();
        self.pending = Some(ScheduleAction::Cancel);
        Ok(ScheduleOperation {
            operation: Operation::JobCancel,
            body,
        })
    }

    /// Accepts one authenticated, natively correlated response and validates any report.
    ///
    /// A mutation remains reconciliation-required until its response includes a
    /// valid complete status report. Device application status is returned only
    /// after those report bytes have been checked and retained.
    pub fn accept_response(
        &mut self,
        response: &Response,
    ) -> Result<ParticipantSchedulePhase, ScheduleControlError> {
        let action = self
            .pending
            .take()
            .ok_or(ScheduleControlError::NoPendingRequest)?;
        if action.mutates() {
            self.reconciliation_required = true;
        }
        if response.body.is_empty() {
            if response.status == StatusCode::Ok || action == ScheduleAction::Status {
                return Err(ScheduleControlError::ResponseBody);
            }
            return Err(ScheduleControlError::DeviceStatus(response.status));
        }
        let report = JobStatusReport::decode(&response.body)?;
        self.validate_report(report)?;
        if let Some(previous) = self.report
            && !job_report_advances(previous, report)
        {
            return Err(ScheduleControlError::Regression);
        }
        self.report = Some(report);
        self.reconciliation_required = false;
        if response.status != StatusCode::Ok {
            return Err(ScheduleControlError::DeviceStatus(response.status));
        }
        Ok(self.phase())
    }

    /// Marks a lost/ambiguous request; mutations must next use `JobStatus`.
    pub fn abandon_pending(&mut self) -> bool {
        let Some(action) = self.pending.take() else {
            return false;
        };
        if action.mutates() {
            self.reconciliation_required = true;
        }
        true
    }

    fn ensure_no_pending(&self) -> Result<(), ScheduleControlError> {
        if self.pending.is_some() {
            Err(ScheduleControlError::RequestPending)
        } else {
            Ok(())
        }
    }

    fn ensure_mutation_allowed(&self) -> Result<(), ScheduleControlError> {
        self.ensure_no_pending()?;
        if self.reconciliation_required {
            Err(ScheduleControlError::ReconciliationRequired)
        } else {
            Ok(())
        }
    }

    fn ready_report(&self, report: JobStatusReport) -> bool {
        report.service.is_some_and(|service| {
            matches!(
                service.state,
                ServiceJobState::Prefetching | ServiceJobState::Complete
            )
        }) && report.realtime.is_some_and(|realtime| {
            realtime.state == RealtimeJobState::Admitted && realtime.outstanding
        })
    }

    fn validate_report(&self, report: JobStatusReport) -> Result<(), ScheduleControlError> {
        if let Some(service) = report.service
            && (service.prepare_id != self.descriptor.prepare_id
                || usize::from(service.axis_count) != AXES
                || service.total_blocks != self.descriptor.block_count)
        {
            return Err(ScheduleControlError::Identity);
        }
        if let Some(realtime) = report.realtime
            && (realtime.prepare_id != self.descriptor.prepare_id
                || realtime.total_blocks != self.descriptor.block_count)
        {
            return Err(ScheduleControlError::Identity);
        }
        let Some(schedule) = report.schedule else {
            return Ok(());
        };
        if schedule.descriptor_token != self.prepared_token {
            return Err(ScheduleControlError::Identity);
        }
        if schedule.state == JobScheduleState::Prepared {
            if schedule.prepared_token != Some(self.prepared_token) {
                return Err(ScheduleControlError::Identity);
            }
            return Ok(());
        }
        if self
            .commit
            .is_some_and(|commit| !schedule_matches_commit(schedule, commit))
        {
            return Err(ScheduleControlError::Identity);
        }
        Ok(())
    }
}

fn schedule_matches_commit(schedule: JobScheduleReport, commit: JobCommitRequest) -> bool {
    schedule.policy == Some(commit.policy)
        && schedule.prepared_token.is_none()
        && schedule.local_start_cycle == commit.local_start_cycle
        && schedule.confirm_deadline_cycle == commit.confirm_deadline_cycle
        && schedule.abort_guard_cycle == commit.abort_guard_cycle
        && schedule.lease_expiry_cycle == commit.lease_expiry_cycle
        && schedule.commit_id == commit.commit_id.as_bytes()
}

fn job_report_advances(previous: JobStatusReport, next: JobStatusReport) -> bool {
    match (previous.schedule, next.schedule) {
        (None, _) => true,
        (Some(_), None) => {
            next.service
                .is_some_and(|service| service.state == ServiceJobState::Cancelled)
                && next
                    .realtime
                    .is_some_and(|realtime| realtime.state == RealtimeJobState::Cancelled)
        }
        (Some(previous), Some(next)) => schedule_report_advances(previous, next),
    }
}

fn schedule_report_advances(previous: JobScheduleReport, next: JobScheduleReport) -> bool {
    if previous == next {
        return true;
    }
    if previous.state == JobScheduleState::Running
        && next.state == JobScheduleState::Running
        && previous.start_observation.is_none()
        && next.start_observation.is_some()
    {
        let mut without_observation = next;
        without_observation.start_observation = None;
        return without_observation == previous;
    }
    match previous.state {
        JobScheduleState::Prepared => next.state != JobScheduleState::Prepared,
        JobScheduleState::Installed => matches!(
            next.state,
            JobScheduleState::Confirmed
                | JobScheduleState::Priming
                | JobScheduleState::Primed
                | JobScheduleState::Running
                | JobScheduleState::Aborted
                | JobScheduleState::Expired
                | JobScheduleState::Complete
                | JobScheduleState::Faulted
        ),
        JobScheduleState::Confirmed => matches!(
            next.state,
            JobScheduleState::Priming
                | JobScheduleState::Primed
                | JobScheduleState::Running
                | JobScheduleState::Aborted
                | JobScheduleState::Complete
                | JobScheduleState::Faulted
        ),
        JobScheduleState::Priming => matches!(
            next.state,
            JobScheduleState::Primed
                | JobScheduleState::Running
                | JobScheduleState::Complete
                | JobScheduleState::Faulted
        ),
        JobScheduleState::Primed => matches!(
            next.state,
            JobScheduleState::Running | JobScheduleState::Complete | JobScheduleState::Faulted
        ),
        JobScheduleState::Running => matches!(
            next.state,
            JobScheduleState::Complete | JobScheduleState::Faulted
        ),
        JobScheduleState::Aborted
        | JobScheduleState::Expired
        | JobScheduleState::Complete
        | JobScheduleState::Faulted => false,
    }
}

/// Per-participant schedule construction, state, response, or identity failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleControlError {
    /// Another operation is awaiting acceptance or abandonment.
    RequestPending,
    /// No operation exists for the supplied response.
    NoPendingRequest,
    /// A lost or malformed mutation must be followed by exact status inspection.
    ReconciliationRequired,
    /// Requested mutation is not valid in the observed device phase.
    State,
    /// Caller attempted to replace a previously bound commit.
    Conflict,
    /// Descriptor, boot, prepare receipt, partition, or report identity differed.
    Identity,
    /// Device reported a commit before this controller bound one.
    ForeignCommit,
    /// An otherwise successful operation omitted its fixed status report.
    ResponseBody,
    /// Device returned a typed application rejection after report validation.
    DeviceStatus(StatusCode),
    /// A later authenticated status attempted to erase or regress schedule evidence.
    Regression,
    /// Descriptor construction or encoding failed.
    Descriptor(JobDescriptorWireError),
    /// Commit/reference construction or schedule report decoding failed.
    Schedule(JobScheduleWireError),
    /// Preparation cancellation could not be represented canonically.
    Cancel(JobCancelWireError),
    /// Combined service/realtime/schedule report decoding failed.
    Status(JobStatusReportWireError),
}

impl From<JobDescriptorWireError> for ScheduleControlError {
    fn from(value: JobDescriptorWireError) -> Self {
        Self::Descriptor(value)
    }
}

impl From<JobScheduleWireError> for ScheduleControlError {
    fn from(value: JobScheduleWireError) -> Self {
        Self::Schedule(value)
    }
}

impl From<JobCancelWireError> for ScheduleControlError {
    fn from(value: JobCancelWireError) -> Self {
        Self::Cancel(value)
    }
}

impl From<JobStatusReportWireError> for ScheduleControlError {
    fn from(value: JobStatusReportWireError) -> Self {
        Self::Status(value)
    }
}

impl fmt::Display for ScheduleControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestPending => formatter.write_str("a participant request is pending"),
            Self::NoPendingRequest => formatter.write_str("no participant request is pending"),
            Self::ReconciliationRequired => {
                formatter.write_str("participant mutation requires status reconciliation")
            }
            Self::State => formatter.write_str("participant schedule state rejects operation"),
            Self::Conflict => formatter.write_str("participant commit conflicts with bound commit"),
            Self::Identity => formatter.write_str("participant schedule identity differs"),
            Self::ForeignCommit => formatter.write_str("device reported an unbound commit"),
            Self::ResponseBody => formatter.write_str("participant status body is missing"),
            Self::DeviceStatus(status) => write!(formatter, "participant returned {status:?}"),
            Self::Regression => {
                formatter.write_str("participant schedule report regressed prior evidence")
            }
            Self::Descriptor(error) => write!(formatter, "job descriptor rejected: {error:?}"),
            Self::Schedule(error) => write!(formatter, "job schedule rejected: {error:?}"),
            Self::Cancel(error) => write!(formatter, "job cancellation rejected: {error:?}"),
            Self::Status(error) => write!(formatter, "job status rejected: {error:?}"),
        }
    }
}

impl std::error::Error for ScheduleControlError {}

#[cfg(test)]
mod tests {
    use alumina_job::{
        JOB_COMMIT_ID_BYTES, JobCommitId, JobNetworkPolicy, JobScheduleAdmission,
        JobScheduleReferenceAction, JobScheduleState, JobStartObservation,
        JobStartObservationSource, PreparedJobSchedule, RealtimeJobReport, ServiceJobReport,
    };
    use alumina_machine_ir::{
        BlockValidationLimits, EXECUTION_BLOCK_BYTES, StreamId, StreamTick, ValidationLimits,
    };
    use alumina_protocol::{DeviceCycle, Digest};
    use alumina_storage::{ContentId, DigestAlgorithm, ObjectKind, PublishedObject, StoredObject};

    use super::*;

    fn boot() -> BootId {
        BootId::new([0x31; 16]).unwrap()
    }

    fn descriptor() -> JobDescriptor {
        JobDescriptor {
            prepare_id: 41,
            partition: PublishedObject {
                object: StoredObject {
                    kind: ObjectKind::MachineJobPartition,
                    content: ContentId {
                        algorithm: DigestAlgorithm::Sha256,
                        digest: Digest([0x51; 32]),
                    },
                    byte_len: EXECUTION_BLOCK_BYTES as u64,
                },
                manifest: ContentId {
                    algorithm: DigestAlgorithm::Sha256,
                    digest: Digest([0x52; 32]),
                },
            },
            stream_id: StreamId::new([0x53; 16]).unwrap(),
            capability_digest: Digest([0x54; 32]),
            config_digest: Digest([0x55; 32]),
            axis_count: 2,
            execution_kind: alumina_machine_ir::ExecutionKind::Motion,
            maximum_dense_updates: 0,
            dense_update_period_ticks: 0,
            block_count: 1,
            first_tick: StreamTick(0),
            initial_position: [0; alumina_machine_ir::MAX_EXECUTION_AXES],
            limits: BlockValidationLimits {
                maximum_block_ticks: 1_000_000,
                segment: ValidationLimits {
                    maximum_segment_ticks: 100_000,
                    maximum_steps_per_segment: 100_000,
                },
            },
        }
    }

    fn commit(machine: &ParticipantScheduleMachine<2>) -> JobCommitRequest {
        JobCommitRequest {
            policy: JobNetworkPolicy::NetworkAttended,
            prepare_id: machine.descriptor().prepare_id,
            boot_id: machine.boot_id(),
            global_job_digest: Digest([0x61; 32]),
            participant_set_digest: Digest([0x62; 32]),
            prepared_token: machine.prepared_token(),
            partition_digest: machine.descriptor().partition.object.content.digest,
            local_start_cycle: DeviceCycle(5_000_000),
            confirm_deadline_cycle: DeviceCycle(4_600_000),
            abort_guard_cycle: DeviceCycle(4_800_000),
            lease_expiry_cycle: DeviceCycle(8_000_000),
            clock_probe_id: 7,
            clock_uncertainty_cycles: 300,
            required_sync_tolerance_cycles: 2_000,
            commit_id: JobCommitId::new([0x63; JOB_COMMIT_ID_BYTES]).unwrap(),
        }
    }

    fn status(schedule: JobScheduleReport) -> JobStatusReport {
        JobStatusReport {
            service: Some(ServiceJobReport {
                prepare_id: 41,
                state: ServiceJobState::Prefetching,
                axis_count: 2,
                validated_blocks: 1,
                sent_blocks: 1,
                total_blocks: 1,
                storage_chunks_read: 1,
                queue_free: 1,
                queue_depth: 0,
                final_progress: None,
            }),
            realtime: Some(RealtimeJobReport {
                prepare_id: 41,
                state: RealtimeJobState::Admitted,
                admitted_blocks: 1,
                completed_blocks: 0,
                total_blocks: 1,
                queue_depth: 0,
                admitted_progress: Some((StreamTick(100), Digest([0x71; 32]))),
                completed_progress: None,
                outstanding: true,
            }),
            schedule: Some(schedule),
        }
    }

    fn response(status: StatusCode, report: JobStatusReport) -> Response {
        Response {
            status,
            body: report.encode().unwrap().to_vec(),
        }
    }

    fn admission() -> JobScheduleAdmission {
        JobScheduleAdmission {
            now: DeviceCycle(3_000_000),
            active_config: descriptor().config_digest,
            minimum_lead_cycles: 100_000,
            maximum_start_horizon_cycles: 10_000_000,
            maximum_lease_cycles: 10_000_000,
            maximum_sync_tolerance_cycles: 2_000,
            minimum_prime_lead_cycles: 100_000,
            cache_ready: true,
            safety_ready: true,
            autonomous_allowed: false,
        }
    }

    #[test]
    fn prepare_install_and_confirm_require_exact_observed_transitions() {
        let descriptor = descriptor();
        let mut authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();

        let prepare = machine.begin_prepare().unwrap();
        assert_eq!(prepare.operation, Operation::JobPrepare);
        assert_eq!(JobDescriptor::decode::<2>(&prepare.body), Ok(descriptor));
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Ready
        );

        let commit = commit(&machine);
        let install = machine.begin_install(commit).unwrap();
        assert_eq!(install.operation, Operation::JobCommit);
        assert_eq!(JobCommitRequest::decode(&install.body), Ok(commit));
        authority.install(commit, admission()).unwrap();
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Installed
        );

        let confirm = machine.begin_confirm().unwrap();
        assert_eq!(confirm.operation, Operation::JobConfirm);
        let reference = JobScheduleReference::decode(&confirm.body).unwrap();
        authority
            .confirm(reference, DeviceCycle(4_000_000))
            .unwrap();
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Confirmed
        );
    }

    #[test]
    fn lost_install_response_forces_read_only_reconciliation() {
        let descriptor = descriptor();
        let mut authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        machine.begin_prepare().unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        let commit = commit(&machine);
        machine.begin_install(commit).unwrap();
        authority.install(commit, admission()).unwrap();
        assert!(machine.abandon_pending());
        assert_eq!(
            machine.begin_install(commit),
            Err(ScheduleControlError::ReconciliationRequired)
        );
        assert_eq!(
            machine.begin_status().unwrap().operation,
            Operation::JobStatus
        );
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Installed
        );
        assert!(!machine.reconciliation_required());
    }

    #[test]
    fn lost_abort_response_reconciles_before_any_second_mutation() {
        let descriptor = descriptor();
        let mut authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        machine.begin_prepare().unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        let commit = commit(&machine);
        machine.begin_install(commit).unwrap();
        authority.install(commit, admission()).unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();

        let abort = machine.begin_abort().unwrap();
        let reference = JobScheduleReference::decode(&abort.body).unwrap();
        authority.abort(reference, DeviceCycle(4_100_000)).unwrap();
        assert!(machine.abandon_pending());
        assert_eq!(
            machine.begin_abort(),
            Err(ScheduleControlError::ReconciliationRequired)
        );
        assert_eq!(
            machine.begin_status().unwrap().operation,
            Operation::JobStatus
        );
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Aborted
        );
        assert!(!machine.reconciliation_required());
    }

    #[test]
    fn lost_confirmed_abort_response_reconciles_revoked_start_authority() {
        let descriptor = descriptor();
        let mut authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        machine.begin_prepare().unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        let commit = commit(&machine);
        machine.begin_install(commit).unwrap();
        authority.install(commit, admission()).unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        let confirm = machine.begin_confirm().unwrap();
        authority
            .confirm(
                JobScheduleReference::decode(&confirm.body).unwrap(),
                DeviceCycle(4_000_000),
            )
            .unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();

        let abort = machine.begin_abort().unwrap();
        authority
            .abort(
                JobScheduleReference::decode(&abort.body).unwrap(),
                DeviceCycle(4_100_000),
            )
            .unwrap();
        assert!(machine.abandon_pending());
        assert_eq!(
            machine.begin_abort(),
            Err(ScheduleControlError::ReconciliationRequired)
        );
        machine.begin_status().unwrap();
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Aborted
        );
    }

    #[test]
    fn lost_confirmed_abort_request_reconciles_then_retries_the_mutation() {
        let descriptor = descriptor();
        let mut authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        machine.begin_prepare().unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        let commit = commit(&machine);
        machine.begin_install(commit).unwrap();
        authority.install(commit, admission()).unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        let confirm = machine.begin_confirm().unwrap();
        authority
            .confirm(
                JobScheduleReference::decode(&confirm.body).unwrap(),
                DeviceCycle(4_000_000),
            )
            .unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();

        assert_eq!(
            machine.begin_abort().unwrap().operation,
            Operation::JobAbort
        );
        assert!(machine.abandon_pending());
        assert_eq!(
            machine.begin_abort(),
            Err(ScheduleControlError::ReconciliationRequired)
        );
        assert_eq!(
            machine.begin_status().unwrap().operation,
            Operation::JobStatus
        );
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Confirmed
        );
        assert!(!machine.reconciliation_required());

        let retry = machine.begin_abort().unwrap();
        assert_eq!(retry.operation, Operation::JobAbort);
        authority
            .abort(
                JobScheduleReference::decode(&retry.body).unwrap(),
                DeviceCycle(4_100_000),
            )
            .unwrap();
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Aborted
        );
    }

    #[test]
    fn retained_complete_report_requires_the_exact_descriptor_token() {
        let descriptor = descriptor();
        let machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        let commit = commit(&machine);
        let mut authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut complete = authority.install(commit, admission()).unwrap();
        complete.state = JobScheduleState::Complete;
        complete.start_emitted = true;
        complete.start_observation = Some(JobStartObservation {
            source: JobStartObservationSource::SimulatedLatch,
            output_token: 7,
            scheduled_cycle: commit.local_start_cycle,
            earliest_cycle: commit.local_start_cycle,
            latest_cycle: commit.local_start_cycle,
        });

        let mut reattached = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        reattached.begin_status().unwrap();
        assert_eq!(
            reattached
                .accept_response(&response(StatusCode::Ok, status(complete)))
                .unwrap(),
            ParticipantSchedulePhase::Complete
        );
        assert_eq!(reattached.commit(), None);

        let mut foreign = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        let mut substituted = complete;
        substituted.descriptor_token = PreparedJobToken(Digest([0x99; 32]));
        foreign.begin_status().unwrap();
        assert_eq!(
            foreign.accept_response(&response(StatusCode::Ok, status(substituted))),
            Err(ScheduleControlError::Identity)
        );
        assert_eq!(foreign.report(), None);
    }

    #[test]
    fn malformed_or_foreign_status_never_becomes_authoritative() {
        let descriptor = descriptor();
        let authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        machine.begin_prepare().unwrap();
        let mut foreign = status(authority.report());
        foreign.service.as_mut().unwrap().prepare_id += 1;
        foreign.realtime.as_mut().unwrap().prepare_id += 1;
        assert_eq!(
            machine.accept_response(&response(StatusCode::Ok, foreign)),
            Err(ScheduleControlError::Identity)
        );
        assert_eq!(machine.report(), None);
        assert!(machine.reconciliation_required());
    }

    #[test]
    fn missing_prepare_after_ambiguous_request_reconciles_to_empty() {
        let descriptor = descriptor();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        machine.begin_prepare().unwrap();
        assert!(machine.abandon_pending());

        assert_eq!(
            machine.begin_status().unwrap().operation,
            Operation::JobStatus
        );
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, JobStatusReport::default()))
                .unwrap(),
            ParticipantSchedulePhase::Empty
        );
        assert!(!machine.reconciliation_required());
        assert_eq!(
            machine.begin_prepare().unwrap().operation,
            Operation::JobPrepare
        );
    }

    #[test]
    fn prepared_job_cleanup_requires_both_actors_to_report_cancelled() {
        let descriptor = descriptor();
        let authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();
        machine.begin_prepare().unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();

        let cancel = machine.begin_cancel().unwrap();
        assert_eq!(cancel.operation, Operation::JobCancel);
        assert_eq!(
            JobCancelRequest::decode(&cancel.body).unwrap().prepare_id,
            descriptor.prepare_id
        );
        let mut cancelled = status(authority.report());
        cancelled.service.as_mut().unwrap().state = ServiceJobState::Cancelled;
        let realtime = cancelled.realtime.as_mut().unwrap();
        realtime.state = RealtimeJobState::Cancelled;
        realtime.admitted_progress = None;
        realtime.outstanding = false;
        cancelled.schedule = None;
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, cancelled))
                .unwrap(),
            ParticipantSchedulePhase::Cancelled
        );
    }

    #[test]
    fn accepted_start_observation_cannot_be_erased_by_later_status() {
        let descriptor = descriptor();
        let mut authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();

        machine.begin_prepare().unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        let commit = commit(&machine);
        machine.begin_install(commit).unwrap();
        authority.install(commit, admission()).unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        machine.begin_confirm().unwrap();
        let confirm =
            JobScheduleReference::for_commit(JobScheduleReferenceAction::Confirm, commit).unwrap();
        authority.confirm(confirm, DeviceCycle(4_000_000)).unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();

        assert!(matches!(
            authority.advance(commit.abort_guard_cycle),
            alumina_job::JobScheduleAction::PrimeHardware { .. }
        ));
        authority
            .mark_primed(DeviceCycle(commit.abort_guard_cycle.0 + 1))
            .unwrap();
        assert!(matches!(
            authority.advance(commit.local_start_cycle),
            alumina_job::JobScheduleAction::Start { .. }
        ));
        machine.begin_status().unwrap();
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Running
        );

        let observation = JobStartObservation {
            source: JobStartObservationSource::SimulatedLatch,
            output_token: 17,
            scheduled_cycle: commit.local_start_cycle,
            earliest_cycle: DeviceCycle(commit.local_start_cycle.0 + 3),
            latest_cycle: DeviceCycle(commit.local_start_cycle.0 + 3),
        };
        authority.record_start_observation(observation).unwrap();
        machine.begin_status().unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        assert_eq!(
            machine
                .report()
                .unwrap()
                .schedule
                .unwrap()
                .start_observation,
            Some(observation)
        );

        let mut regressed = status(authority.report());
        regressed.schedule.as_mut().unwrap().start_observation = None;
        assert_eq!(regressed.schedule.unwrap().state, JobScheduleState::Running);
        machine.begin_status().unwrap();
        assert_eq!(
            machine.accept_response(&response(StatusCode::Ok, regressed)),
            Err(ScheduleControlError::Regression)
        );
        assert_eq!(
            machine
                .report()
                .unwrap()
                .schedule
                .unwrap()
                .start_observation,
            Some(observation)
        );
    }

    #[test]
    fn polling_may_observe_complete_after_primed_without_an_intermediate_running_report() {
        let descriptor = descriptor();
        let mut authority = PreparedJobSchedule::prepare::<2>(boot(), descriptor).unwrap();
        let mut machine = ParticipantScheduleMachine::<2>::new(descriptor, boot()).unwrap();

        machine.begin_prepare().unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        let commit = commit(&machine);
        machine.begin_install(commit).unwrap();
        authority.install(commit, admission()).unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();
        machine.begin_confirm().unwrap();
        let confirm =
            JobScheduleReference::for_commit(JobScheduleReferenceAction::Confirm, commit).unwrap();
        authority.confirm(confirm, DeviceCycle(4_000_000)).unwrap();
        machine
            .accept_response(&response(StatusCode::Ok, status(authority.report())))
            .unwrap();

        assert!(matches!(
            authority.advance(commit.abort_guard_cycle),
            alumina_job::JobScheduleAction::PrimeHardware { .. }
        ));
        authority
            .mark_primed(DeviceCycle(commit.abort_guard_cycle.0 + 1))
            .unwrap();
        machine.begin_status().unwrap();
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Primed
        );

        assert!(matches!(
            authority.advance(commit.local_start_cycle),
            alumina_job::JobScheduleAction::Start { .. }
        ));
        let observation = JobStartObservation {
            source: JobStartObservationSource::SimulatedLatch,
            output_token: 17,
            scheduled_cycle: commit.local_start_cycle,
            earliest_cycle: commit.local_start_cycle,
            latest_cycle: commit.local_start_cycle,
        };
        authority.record_start_observation(observation).unwrap();
        authority
            .complete(DeviceCycle(commit.local_start_cycle.0 + 1))
            .unwrap();
        machine.begin_status().unwrap();
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status(authority.report())))
                .unwrap(),
            ParticipantSchedulePhase::Complete
        );
    }
}
