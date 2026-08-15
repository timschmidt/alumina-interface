//! Worker-owned immutable cache delivery and deterministic multi-MCU job control.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use alumina_clock::BootId;
use alumina_interface_client::Response;
use alumina_interface_client::upload::{
    CacheUploadError, CacheUploadMachine, CacheUploadPhase, OwnedUploadSource,
    OwnedUploadSourceError,
};
use alumina_interface_client::worker::{
    WORKER_CACHED_JOB_LIMITS, WorkerCacheArtifactSnapshot, WorkerCachePhaseSnapshot,
    WorkerCachedJobParticipant, WorkerCachedJobParticipantSnapshot, WorkerCachedJobPhaseSnapshot,
    WorkerCachedJobRequest, WorkerCachedJobSnapshot, WorkerContractError, WorkerJobExecutionMode,
    WorkerParticipantSchedulePhaseSnapshot,
};
use alumina_job::{DecodedMachineJobManifest, JobDescriptor};
use alumina_protocol::{DeviceId, Digest, Operation};
use alumina_storage::UploadPlan;

use crate::cache_delivery::{ParticipantCacheDeliveryError, ParticipantCacheReady};
use crate::distributed_schedule::{
    CachedParticipantSchedule, DistributedJobIdentity, DistributedScheduleCoordinator,
    DistributedScheduleError, DistributedSchedulePhase, DistributedScheduleRequest,
    ParticipantStartInput,
};

const JOB_AXES: usize = 2;

/// Exact live session identity required by one compiled participant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveJobParticipantBinding {
    /// UI-local connection identity.
    pub connection_id: u64,
    /// Worker generation observed during compilation.
    pub generation: u64,
    /// Stable MCU identity in the canonical global manifest.
    pub device_id: DeviceId,
    /// Boot identity bound into the prepared receipt.
    pub boot_id: BootId,
    /// Active configuration used by authoritative CAM.
    pub config_digest: Digest,
    /// Exact immutable board capability used by authoritative CAM.
    pub capability_digest: Digest,
}

/// One device-targeted native operation emitted by the live job machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveJobOperation {
    /// Exact live session that must perform the operation.
    pub binding: LiveJobParticipantBinding,
    /// Native configuration identity required in the frame header.
    pub frame_config_digest: Digest,
    /// Exact protocol operation.
    pub operation: Operation,
    /// Canonical operation-specific body.
    pub body: Vec<u8>,
}

struct LiveParticipant {
    binding: LiveJobParticipantBinding,
    descriptor: JobDescriptor,
    partition_source: OwnedUploadSource,
    manifest_source: OwnedUploadSource,
    partition_upload: CacheUploadMachine,
    manifest_upload: CacheUploadMachine,
}

impl LiveParticipant {
    fn cache_complete(&self) -> bool {
        self.partition_upload.phase() == CacheUploadPhase::Complete
            && self.manifest_upload.phase() == CacheUploadPhase::Complete
    }

    fn cache_snapshot(
        &self,
    ) -> (
        WorkerCacheArtifactSnapshot,
        WorkerCachePhaseSnapshot,
        u64,
        u64,
        u32,
    ) {
        if self.partition_upload.phase() != CacheUploadPhase::Complete {
            return cache_progress(
                WorkerCacheArtifactSnapshot::Partition,
                self.partition_upload.phase(),
                self.partition_upload.upload_plan().object.byte_len,
            );
        }
        if self.manifest_upload.phase() != CacheUploadPhase::Complete {
            return cache_progress(
                WorkerCacheArtifactSnapshot::GlobalManifest,
                self.manifest_upload.phase(),
                self.manifest_upload.upload_plan().object.byte_len,
            );
        }
        let total = self
            .partition_upload
            .upload_plan()
            .object
            .byte_len
            .saturating_add(self.manifest_upload.upload_plan().object.byte_len);
        (
            WorkerCacheArtifactSnapshot::Complete,
            WorkerCachePhaseSnapshot::Complete,
            total,
            total,
            0,
        )
    }
}

fn cache_progress(
    artifact: WorkerCacheArtifactSnapshot,
    phase: CacheUploadPhase,
    declared_total: u64,
) -> (
    WorkerCacheArtifactSnapshot,
    WorkerCachePhaseSnapshot,
    u64,
    u64,
    u32,
) {
    match phase {
        CacheUploadPhase::Uploading {
            next_chunk,
            accepted_bytes,
            total_bytes,
        } => (
            artifact,
            WorkerCachePhaseSnapshot::Uploading,
            accepted_bytes,
            total_bytes,
            next_chunk,
        ),
        CacheUploadPhase::Finalizing => (
            artifact,
            WorkerCachePhaseSnapshot::Finalizing,
            declared_total,
            declared_total,
            0,
        ),
        CacheUploadPhase::Complete => (
            artifact,
            WorkerCachePhaseSnapshot::Complete,
            declared_total,
            declared_total,
            0,
        ),
        other => (artifact, other.into(), 0, declared_total, 0),
    }
}

/// One bounded worker-owned job from immutable cache handoff through completion.
pub struct LiveCachedJob {
    job_id: u64,
    execution_mode: WorkerJobExecutionMode,
    manifest_byte_len: u32,
    identity: DistributedJobIdentity,
    participants: Vec<LiveParticipant>,
    coordinator: Option<DistributedScheduleCoordinator>,
    status_queue: VecDeque<DistributedScheduleRequest>,
    stop_requested: bool,
    terminal_override: Option<WorkerCachedJobPhaseSnapshot>,
    consecutive_failures: u32,
    last_error: Option<String>,
}

impl LiveCachedJob {
    /// Consumes and independently reconstructs the complete browser CAM handoff.
    ///
    /// # Errors
    ///
    /// Rejects any malformed, substituted, oversized, or internally inconsistent artifact.
    pub fn try_new(request: WorkerCachedJobRequest) -> Result<Self, LiveCachedJobError> {
        request.validate()?;
        let WorkerCachedJobRequest {
            job_id,
            execution_mode,
            manifest_bytes,
            participants: requested_participants,
        } = request;
        let manifest_byte_len = u32::try_from(manifest_bytes.len())
            .map_err(|_| LiveCachedJobError::Contract(WorkerContractError::CachedJobRequest))?;
        let manifest: Arc<[u8]> = Arc::from(manifest_bytes);
        let decoded = DecodedMachineJobManifest::decode(&manifest)
            .map_err(|_| LiveCachedJobError::Contract(WorkerContractError::CachedJobRequest))?;
        let global = decoded.global();
        let global_job_digest = decoded.global_job_digest();
        let participant_set_digest = decoded.participant_set_digest();

        let mut participants = Vec::new();
        participants
            .try_reserve_exact(requested_participants.len())
            .map_err(|_| LiveCachedJobError::AllocationOverflow)?;
        let mut global_manifest = None;
        for participant in requested_participants {
            let built = build_participant(participant, Arc::clone(&manifest))?;
            let publication = built.manifest_upload.publication();
            if global_manifest.is_some_and(|expected| expected != publication) {
                return Err(LiveCachedJobError::Identity);
            }
            global_manifest = Some(publication);
            participants.push(built);
        }
        let identity = DistributedJobIdentity {
            global_job_digest,
            participant_set_digest,
            global_timebase_hz: global.global_timebase_hz,
            duration_ticks: global.duration_ticks,
            network_policy: global.network_policy,
            global_manifest: global_manifest.ok_or(LiveCachedJobError::Identity)?,
        };
        Ok(Self {
            job_id,
            execution_mode,
            manifest_byte_len,
            identity,
            participants,
            coordinator: None,
            status_queue: VecDeque::new(),
            stop_requested: false,
            terminal_override: None,
            consecutive_failures: 0,
            last_error: None,
        })
    }

    /// UI-local identity of this retained job.
    #[must_use]
    pub const fn job_id(&self) -> u64 {
        self.job_id
    }

    /// Explicit simulator versus physical execution authority class.
    #[must_use]
    pub const fn execution_mode(&self) -> WorkerJobExecutionMode {
        self.execution_mode
    }

    /// Shared manifest-wide immutable job identity.
    #[must_use]
    pub const fn identity(&self) -> DistributedJobIdentity {
        self.identity
    }

    /// Shared future browser epoch after a deterministic start was bound.
    #[must_use]
    pub fn target_ui_ns(&self) -> Option<u64> {
        self.coordinator
            .as_ref()
            .and_then(DistributedScheduleCoordinator::target_ui_ns)
    }

    /// Looks up the exact compiled live-session binding for one connection.
    #[must_use]
    pub fn binding(&self, connection_id: u64) -> Option<LiveJobParticipantBinding> {
        self.participants
            .iter()
            .find(|participant| participant.binding.connection_id == connection_id)
            .map(|participant| participant.binding)
    }

    /// Canonically ordered participant bindings.
    pub fn bindings(&self) -> impl Iterator<Item = LiveJobParticipantBinding> + '_ {
        self.participants
            .iter()
            .map(|participant| participant.binding)
    }

    /// Derives the current global lifecycle from exact cache and schedule state.
    #[must_use]
    pub fn phase(&self) -> WorkerCachedJobPhaseSnapshot {
        if let Some(phase) = self.terminal_override {
            return phase;
        }
        self.coordinator
            .as_ref()
            .map_or(WorkerCachedJobPhaseSnapshot::Caching, |coordinator| {
                project_distributed_phase(
                    coordinator.phase(),
                    coordinator.target_ui_ns().is_some(),
                    self.stop_requested,
                )
            })
    }

    /// Whether no further mutation can be usefully requested for this retained job.
    #[must_use]
    pub fn terminal(&self) -> bool {
        matches!(
            self.phase(),
            WorkerCachedJobPhaseSnapshot::Aborted
                | WorkerCachedJobPhaseSnapshot::Cancelled
                | WorkerCachedJobPhaseSnapshot::Complete
                | WorkerCachedJobPhaseSnapshot::CompletedAfterStopRequest
                | WorkerCachedJobPhaseSnapshot::SplitAfterStopRequest
                | WorkerCachedJobPhaseSnapshot::RetainedComplete
                | WorkerCachedJobPhaseSnapshot::Faulted
        )
    }

    /// Emits the unique next cache, preparation, install, confirm, abort, cancel, or status request.
    ///
    /// # Errors
    ///
    /// Returns an error if retained upload or scheduling state is inconsistent,
    /// or if the next native operation cannot be formed canonically.
    pub fn next_operation(&mut self) -> Result<Option<LiveJobOperation>, LiveCachedJobError> {
        if self.terminal_override.is_some() {
            return Ok(None);
        }
        if let Some(request) = self.status_queue.pop_front() {
            return self.schedule_operation(request).map(Some);
        }
        if self.coordinator.is_none() {
            if let Some(operation) = self.next_cache_operation()? {
                return Ok(Some(operation));
            }
            self.initialize_schedule()?;
        }
        let request = self
            .coordinator
            .as_mut()
            .ok_or(LiveCachedJobError::State)?
            .next_request()?;
        request
            .map(|request| self.schedule_operation(request))
            .transpose()
    }

    /// Applies one already authenticated and correlated response to its exact owner.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown participant, an absent or ambiguous
    /// pending request, a malformed response, or an invalid state transition.
    pub fn accept_response(
        &mut self,
        connection_id: u64,
        response: &Response,
    ) -> Result<WorkerCachedJobPhaseSnapshot, LiveCachedJobError> {
        let device_id = self
            .binding(connection_id)
            .ok_or(LiveCachedJobError::Participant)?
            .device_id;
        if let Some(coordinator) = self.coordinator.as_mut() {
            coordinator.accept_response(device_id, response)?;
        } else {
            let participant = self
                .participants
                .iter_mut()
                .find(|participant| participant.binding.connection_id == connection_id)
                .ok_or(LiveCachedJobError::Participant)?;
            match (
                participant.partition_upload.has_pending_request(),
                participant.manifest_upload.has_pending_request(),
            ) {
                (true, false) => participant.partition_upload.accept_response(response)?,
                (false, true) => participant.manifest_upload.accept_response(response)?,
                _ => return Err(LiveCachedJobError::Pending),
            }
            if self
                .participants
                .iter()
                .all(LiveParticipant::cache_complete)
            {
                self.initialize_schedule()?;
            }
        }
        self.clear_failure();
        Ok(self.phase())
    }

    /// Marks one ambiguous operation lost so its exact state is reconciled before mutation.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection does not belong to this job or the
    /// scheduling machine rejects reconciliation in its current phase.
    pub fn abandon_pending(&mut self, connection_id: u64) -> Result<bool, LiveCachedJobError> {
        let device_id = self
            .binding(connection_id)
            .ok_or(LiveCachedJobError::Participant)?
            .device_id;
        if let Some(coordinator) = self.coordinator.as_mut() {
            return coordinator.abandon_pending(device_id).map_err(Into::into);
        }
        let participant = self
            .participants
            .iter_mut()
            .find(|participant| participant.binding.connection_id == connection_id)
            .ok_or(LiveCachedJobError::Participant)?;
        Ok(participant.partition_upload.abandon_pending()
            || participant.manifest_upload.abandon_pending())
    }

    /// Atomically binds a common future browser epoch into every local commit.
    ///
    /// # Errors
    ///
    /// Returns an error unless the job is ready for start and every participant
    /// clock/deadline input proves the same valid future epoch.
    pub fn begin_start(
        &mut self,
        now_ui_ns: u64,
        target_ui_ns: u64,
        inputs: &[ParticipantStartInput<'_>],
    ) -> Result<(), LiveCachedJobError> {
        if self.terminal_override.is_some() || !self.status_queue.is_empty() {
            return Err(LiveCachedJobError::State);
        }
        self.coordinator
            .as_mut()
            .ok_or(LiveCachedJobError::State)?
            .bind_start(now_ui_ns, target_ui_ns, inputs)?;
        self.clear_failure();
        Ok(())
    }

    /// Opens confirmation only after every installed commit and deadline is exact.
    ///
    /// # Errors
    ///
    /// Returns an error unless every participant is installed and the retained
    /// clock bounds leave sufficient time before the confirmation deadline.
    pub fn begin_confirmation(
        &mut self,
        now_ui_ns: u64,
        inputs: &[ParticipantStartInput<'_>],
    ) -> Result<(), LiveCachedJobError> {
        if self.terminal_override.is_some() || !self.status_queue.is_empty() {
            return Err(LiveCachedJobError::State);
        }
        self.coordinator
            .as_mut()
            .ok_or(LiveCachedJobError::State)?
            .begin_confirmation(now_ui_ns, inputs)?;
        Ok(())
    }

    /// Begins one complete read-only participant status round.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is terminal, another status round is
    /// pending, or the schedule cannot be observed in its current phase.
    pub fn begin_status_round(&mut self) -> Result<(), LiveCachedJobError> {
        if self.terminal_override.is_some() || !self.status_queue.is_empty() {
            return Err(LiveCachedJobError::State);
        }
        let requests = self
            .coordinator
            .as_mut()
            .ok_or(LiveCachedJobError::State)?
            .begin_status_round()?;
        self.status_queue = requests.into();
        Ok(())
    }

    /// Selects local cache cancellation, precommit actor cancellation, or pre-guard abort.
    ///
    /// # Errors
    ///
    /// Returns an error for a terminal job or when the distributed schedule can
    /// no longer enter the required cancellation/abort transition.
    pub fn request_stop(&mut self) -> Result<(), LiveCachedJobError> {
        if self.terminal() {
            return Err(LiveCachedJobError::State);
        }
        self.status_queue.clear();
        let Some(coordinator) = self.coordinator.as_mut() else {
            self.terminal_override = Some(WorkerCachedJobPhaseSnapshot::Cancelled);
            return Ok(());
        };
        if coordinator.target_ui_ns().is_some() {
            coordinator.begin_abort()?;
            self.stop_requested = true;
        } else {
            coordinator.begin_cancel()?;
        }
        Ok(())
    }

    /// Records one bounded transient transport/reconciliation failure without discarding state.
    pub fn record_failure(&mut self, error: &str) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.last_error = Some(bounded_job_diagnostic(error));
    }

    /// Converts an unrecoverable local invariant failure into a visible terminal state.
    pub fn mark_fault(&mut self, error: &str) {
        self.record_failure(error);
        self.terminal_override = Some(WorkerCachedJobPhaseSnapshot::Faulted);
        self.status_queue.clear();
    }

    /// Produces a credential-free independently validated rendering snapshot.
    #[must_use]
    pub fn snapshot(&self) -> WorkerCachedJobSnapshot {
        let participants = self
            .participants
            .iter()
            .map(|participant| {
                let (cache_artifact, cache_phase, accepted_bytes, total_bytes, next_chunk) =
                    participant.cache_snapshot();
                let (schedule_phase, local_start_cycle) = self.coordinator.as_ref().map_or(
                    (WorkerParticipantSchedulePhaseSnapshot::Empty, None),
                    |coordinator| {
                        (
                            coordinator
                                .participant_phase(participant.binding.device_id)
                                .map_or(WorkerParticipantSchedulePhaseSnapshot::Empty, Into::into),
                            coordinator
                                .participant_local_start_cycle(participant.binding.device_id)
                                .map(|cycle| cycle.0),
                        )
                    },
                );
                WorkerCachedJobParticipantSnapshot {
                    connection_id: participant.binding.connection_id,
                    generation: participant.binding.generation,
                    device_id: participant.binding.device_id.0,
                    boot_id: participant.binding.boot_id.as_bytes(),
                    config_digest: participant.binding.config_digest.0,
                    capability_digest: participant.binding.capability_digest.0,
                    cache_artifact,
                    cache_phase,
                    accepted_bytes,
                    total_bytes,
                    next_chunk,
                    schedule_phase,
                    local_start_cycle,
                }
            })
            .collect();
        WorkerCachedJobSnapshot {
            job_id: self.job_id,
            execution_mode: self.execution_mode,
            phase: self.phase(),
            global_job_digest: self.identity.global_job_digest.0,
            participant_set_digest: self.identity.participant_set_digest.0,
            manifest_byte_len: self.manifest_byte_len,
            target_ui_ns: self.target_ui_ns(),
            consecutive_failures: self.consecutive_failures,
            last_error: self.last_error.clone(),
            participants,
        }
    }

    fn next_cache_operation(&mut self) -> Result<Option<LiveJobOperation>, LiveCachedJobError> {
        for participant in &mut self.participants {
            let operation = if participant.partition_upload.phase() != CacheUploadPhase::Complete {
                participant
                    .partition_upload
                    .next_request(&participant.partition_source)?
            } else if participant.manifest_upload.phase() != CacheUploadPhase::Complete {
                participant
                    .manifest_upload
                    .next_request(&participant.manifest_source)?
            } else {
                None
            };
            if let Some(operation) = operation {
                return Ok(Some(LiveJobOperation {
                    binding: participant.binding,
                    frame_config_digest: Digest::ZERO,
                    operation: operation.operation,
                    body: operation.body,
                }));
            }
        }
        Ok(None)
    }

    fn initialize_schedule(&mut self) -> Result<(), LiveCachedJobError> {
        if self.coordinator.is_some() {
            return Ok(());
        }
        if !self
            .participants
            .iter()
            .all(LiveParticipant::cache_complete)
        {
            return Err(LiveCachedJobError::State);
        }
        let mut ready = Vec::new();
        let mut cached = Vec::new();
        ready
            .try_reserve_exact(self.participants.len())
            .map_err(|_| LiveCachedJobError::AllocationOverflow)?;
        cached
            .try_reserve_exact(self.participants.len())
            .map_err(|_| LiveCachedJobError::AllocationOverflow)?;
        for participant in &self.participants {
            ready.push(ParticipantCacheReady::from_reconciled(
                participant.binding.device_id,
                participant.partition_upload.publication(),
                participant.manifest_upload.publication(),
            )?);
            cached.push(CachedParticipantSchedule {
                device_id: participant.binding.device_id,
                boot_id: participant.binding.boot_id,
                descriptor: participant.descriptor,
            });
        }
        self.coordinator = Some(DistributedScheduleCoordinator::after_cached_artifacts(
            self.identity,
            &ready,
            &cached,
        )?);
        Ok(())
    }

    fn schedule_operation(
        &self,
        request: DistributedScheduleRequest,
    ) -> Result<LiveJobOperation, LiveCachedJobError> {
        let participant = self
            .participants
            .binary_search_by_key(&request.device_id, |participant| {
                participant.binding.device_id
            })
            .ok()
            .and_then(|index| self.participants.get(index))
            .ok_or(LiveCachedJobError::Participant)?;
        Ok(LiveJobOperation {
            binding: participant.binding,
            frame_config_digest: participant.binding.config_digest,
            operation: request.request.operation,
            body: request.request.body,
        })
    }

    fn clear_failure(&mut self) {
        self.consecutive_failures = 0;
        self.last_error = None;
    }
}

fn build_participant(
    participant: WorkerCachedJobParticipant,
    manifest: Arc<[u8]>,
) -> Result<LiveParticipant, LiveCachedJobError> {
    let partition_plan = UploadPlan::decode(&participant.partition_plan, WORKER_CACHED_JOB_LIMITS)
        .map_err(|_| LiveCachedJobError::Identity)?;
    let manifest_plan = UploadPlan::decode(&participant.manifest_plan, WORKER_CACHED_JOB_LIMITS)
        .map_err(|_| LiveCachedJobError::Identity)?;
    let partition_source = OwnedUploadSource::try_new(
        partition_plan,
        participant.partition_bytes,
        WORKER_CACHED_JOB_LIMITS,
    )?;
    let manifest_source =
        OwnedUploadSource::try_new_shared(manifest_plan, manifest, WORKER_CACHED_JOB_LIMITS)?;
    let partition_upload = CacheUploadMachine::new(&partition_source, WORKER_CACHED_JOB_LIMITS)?;
    let manifest_upload = CacheUploadMachine::new(&manifest_source, WORKER_CACHED_JOB_LIMITS)?;
    let descriptor = JobDescriptor::decode::<JOB_AXES>(&participant.descriptor)
        .map_err(|_| LiveCachedJobError::Identity)?;
    let boot_id = BootId::new(participant.boot_id).map_err(|_| LiveCachedJobError::Identity)?;
    Ok(LiveParticipant {
        binding: LiveJobParticipantBinding {
            connection_id: participant.connection_id,
            generation: participant.generation,
            device_id: DeviceId(participant.device_id),
            boot_id,
            config_digest: descriptor.config_digest,
            capability_digest: descriptor.capability_digest,
        },
        descriptor,
        partition_source,
        manifest_source,
        partition_upload,
        manifest_upload,
    })
}

const fn distributed_phase(phase: DistributedSchedulePhase) -> WorkerCachedJobPhaseSnapshot {
    match phase {
        DistributedSchedulePhase::Preparing => WorkerCachedJobPhaseSnapshot::Preparing,
        DistributedSchedulePhase::Ready => WorkerCachedJobPhaseSnapshot::Ready,
        DistributedSchedulePhase::Installing => WorkerCachedJobPhaseSnapshot::Installing,
        DistributedSchedulePhase::Installed => WorkerCachedJobPhaseSnapshot::Installed,
        DistributedSchedulePhase::Confirming => WorkerCachedJobPhaseSnapshot::Confirming,
        DistributedSchedulePhase::Confirmed => WorkerCachedJobPhaseSnapshot::Confirmed,
        DistributedSchedulePhase::Irrevocable => WorkerCachedJobPhaseSnapshot::Irrevocable,
        DistributedSchedulePhase::Aborting => WorkerCachedJobPhaseSnapshot::Aborting,
        DistributedSchedulePhase::Aborted => WorkerCachedJobPhaseSnapshot::Aborted,
        DistributedSchedulePhase::SplitAfterAbort => {
            WorkerCachedJobPhaseSnapshot::SplitAfterStopRequest
        }
        DistributedSchedulePhase::Cancelling => WorkerCachedJobPhaseSnapshot::Cancelling,
        DistributedSchedulePhase::Cancelled => WorkerCachedJobPhaseSnapshot::Cancelled,
        DistributedSchedulePhase::Complete => WorkerCachedJobPhaseSnapshot::Complete,
        DistributedSchedulePhase::Faulted => WorkerCachedJobPhaseSnapshot::Faulted,
    }
}

const fn project_distributed_phase(
    phase: DistributedSchedulePhase,
    target_ui_bound: bool,
    stop_requested: bool,
) -> WorkerCachedJobPhaseSnapshot {
    if matches!(phase, DistributedSchedulePhase::Complete) {
        if !target_ui_bound {
            return WorkerCachedJobPhaseSnapshot::RetainedComplete;
        }
        if stop_requested {
            return WorkerCachedJobPhaseSnapshot::CompletedAfterStopRequest;
        }
    }
    distributed_phase(phase)
}

fn bounded_job_diagnostic(error: &str) -> String {
    let mut end = error
        .len()
        .min(alumina_interface_client::worker::MAXIMUM_WORKER_DIAGNOSTIC_BYTES);
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    let bounded = error[..end].trim();
    if bounded.is_empty() {
        "cached job operation failed".to_owned()
    } else {
        bounded.to_owned()
    }
}

/// Failure before live job state may advance.
#[derive(Debug)]
pub enum LiveCachedJobError {
    /// The browser/worker transfer contract was invalid.
    Contract(WorkerContractError),
    /// Immutable upload reconstruction or response reconciliation failed.
    Upload(CacheUploadError),
    /// Browser-supplied immutable bytes did not reconstruct their declared identities.
    UploadSource(OwnedUploadSourceError),
    /// Multi-participant scheduling rejected identity, state, or timing.
    Schedule(DistributedScheduleError),
    /// Cache readiness evidence could not be formed.
    Cache(ParticipantCacheDeliveryError),
    /// A participant, descriptor, publication, or boot identity disagreed.
    Identity,
    /// The named connection is not a participant in this job.
    Participant,
    /// No unique pending cache operation owned the supplied response.
    Pending,
    /// The requested lifecycle transition is not currently legal.
    State,
    /// A bounded participant/request vector could not be allocated.
    AllocationOverflow,
}

impl From<WorkerContractError> for LiveCachedJobError {
    fn from(value: WorkerContractError) -> Self {
        Self::Contract(value)
    }
}

impl From<CacheUploadError> for LiveCachedJobError {
    fn from(value: CacheUploadError) -> Self {
        Self::Upload(value)
    }
}

impl From<OwnedUploadSourceError> for LiveCachedJobError {
    fn from(value: OwnedUploadSourceError) -> Self {
        Self::UploadSource(value)
    }
}

impl From<DistributedScheduleError> for LiveCachedJobError {
    fn from(value: DistributedScheduleError) -> Self {
        Self::Schedule(value)
    }
}

impl From<ParticipantCacheDeliveryError> for LiveCachedJobError {
    fn from(value: ParticipantCacheDeliveryError) -> Self {
        Self::Cache(value)
    }
}

impl fmt::Display for LiveCachedJobError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(formatter, "cached job transfer rejected: {error}"),
            Self::Upload(error) => write!(formatter, "cached artifact rejected: {error}"),
            Self::UploadSource(error) => {
                write!(formatter, "cached artifact source rejected: {error}")
            }
            Self::Schedule(error) => write!(formatter, "distributed schedule rejected: {error}"),
            Self::Cache(error) => write!(formatter, "cache readiness rejected: {error}"),
            Self::Identity => formatter.write_str("cached job identity is inconsistent"),
            Self::Participant => formatter.write_str("cached job participant is absent"),
            Self::Pending => formatter.write_str("cached job response has no unique pending owner"),
            Self::State => formatter.write_str("cached job lifecycle transition is not legal"),
            Self::AllocationOverflow => formatter.write_str("cached job allocation failed"),
        }
    }
}

impl std::error::Error for LiveCachedJobError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use alumina_interface_core::{
        compile_representative_global_job, compile_representative_program,
    };
    use alumina_job::JobStatusReport;
    use alumina_protocol::StatusCode;
    use alumina_storage::{ChunkUploadHeader, UploadId, UploadPhase, UploadProgress};

    use super::*;

    fn request() -> WorkerCachedJobRequest {
        let program = compile_representative_program().unwrap();
        let job = compile_representative_global_job(&program).unwrap();
        let participants = job
            .participants()
            .iter()
            .enumerate()
            .map(|(index, participant)| {
                let prepare_id = u64::try_from(index).unwrap() + 11;
                let descriptor = participant.partition().job_descriptor(prepare_id).unwrap();
                let manifest_plan = job
                    .upload_plan_for(UploadId(0x9000 + u64::try_from(index).unwrap()))
                    .unwrap();
                WorkerCachedJobParticipant {
                    connection_id: u64::try_from(index).unwrap() + 1,
                    generation: u64::try_from(index).unwrap() + 20,
                    device_id: participant.device_id().0,
                    boot_id: [u8::try_from(index).unwrap() + 0x31; 16],
                    descriptor: descriptor.encode::<JOB_AXES>().unwrap().to_vec(),
                    partition_plan: participant.partition().upload_plan().encode().to_vec(),
                    partition_bytes: participant.partition().bytes().to_vec(),
                    manifest_plan: manifest_plan.encode().to_vec(),
                }
            })
            .collect();
        WorkerCachedJobRequest {
            job_id: 7,
            execution_mode: WorkerJobExecutionMode::SimulationOnly,
            manifest_bytes: job.manifest_bytes().to_vec(),
            participants,
        }
    }

    #[test]
    fn exact_handoff_builds_a_valid_cache_snapshot() {
        let job = LiveCachedJob::try_new(request()).unwrap();
        assert_eq!(job.phase(), WorkerCachedJobPhaseSnapshot::Caching);
        assert_eq!(job.bindings().count(), 2);
        assert_eq!(job.snapshot().validate(), Ok(()));
    }

    #[test]
    fn cache_delivery_is_ordered_before_prepare() {
        let mut job = LiveCachedJob::try_new(request()).unwrap();
        let mut active_plans: BTreeMap<u64, UploadPlan> = BTreeMap::new();
        let mut operations = 0_usize;
        loop {
            let operation = job.next_operation().unwrap().unwrap();
            if operation.operation == Operation::JobPrepare {
                assert_eq!(job.phase(), WorkerCachedJobPhaseSnapshot::Preparing);
                assert!(operations > 0);
                assert_eq!(job.snapshot().validate(), Ok(()));
                break;
            }
            let response = match operation.operation {
                Operation::StorageInspect => Response {
                    status: StatusCode::NotFound,
                    body: Vec::new(),
                },
                Operation::StorageBeginUpload => {
                    let plan =
                        UploadPlan::decode(&operation.body, WORKER_CACHED_JOB_LIMITS).unwrap();
                    active_plans.insert(operation.binding.connection_id, plan);
                    Response {
                        status: StatusCode::Ok,
                        body: UploadProgress {
                            upload_id: plan.upload_id,
                            phase: UploadPhase::Receiving,
                            next_chunk: 0,
                            accepted_bytes: 0,
                            total_bytes: plan.object.byte_len,
                        }
                        .encode()
                        .to_vec(),
                    }
                }
                Operation::StoragePutChunk => {
                    let plan = active_plans[&operation.binding.connection_id];
                    let header =
                        ChunkUploadHeader::decode(&operation.body[..ChunkUploadHeader::WIRE_LEN])
                            .unwrap();
                    let next_chunk = header.index + 1;
                    let accepted_bytes = if next_chunk == plan.chunk_count {
                        plan.object.byte_len
                    } else {
                        u64::from(next_chunk) * u64::from(plan.chunk_bytes)
                    };
                    Response {
                        status: StatusCode::Ok,
                        body: UploadProgress {
                            upload_id: plan.upload_id,
                            phase: UploadPhase::Receiving,
                            next_chunk,
                            accepted_bytes,
                            total_bytes: plan.object.byte_len,
                        }
                        .encode()
                        .to_vec(),
                    }
                }
                Operation::StorageFinalize => Response {
                    status: StatusCode::Ok,
                    body: Vec::new(),
                },
                Operation::JobStatus => Response {
                    status: StatusCode::Ok,
                    body: JobStatusReport::default().encode().unwrap().to_vec(),
                },
                other => panic!("unexpected operation {other:?}"),
            };
            job.accept_response(operation.binding.connection_id, &response)
                .unwrap();
            operations += 1;
        }
    }

    #[test]
    fn preprepare_stop_is_local_and_terminal() {
        let mut job = LiveCachedJob::try_new(request()).unwrap();
        job.request_stop().unwrap();
        assert_eq!(job.phase(), WorkerCachedJobPhaseSnapshot::Cancelled);
        assert!(job.next_operation().unwrap().is_none());
        assert_eq!(job.snapshot().validate(), Ok(()));
    }

    #[test]
    fn completed_schedule_retains_whether_a_bound_stop_request_was_missed() {
        assert_eq!(
            project_distributed_phase(DistributedSchedulePhase::Complete, true, false),
            WorkerCachedJobPhaseSnapshot::Complete
        );
        assert_eq!(
            project_distributed_phase(DistributedSchedulePhase::Complete, true, true),
            WorkerCachedJobPhaseSnapshot::CompletedAfterStopRequest
        );
        assert_eq!(
            project_distributed_phase(DistributedSchedulePhase::Complete, false, false),
            WorkerCachedJobPhaseSnapshot::RetainedComplete
        );
        assert_eq!(
            project_distributed_phase(DistributedSchedulePhase::SplitAfterAbort, true, true),
            WorkerCachedJobPhaseSnapshot::SplitAfterStopRequest
        );
    }
}
