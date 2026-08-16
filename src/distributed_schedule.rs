//! Browser-owned deterministic preparation and cached-start coordination.

use std::fmt;

use alumina_clock::BootId;
use alumina_interface_client::Response;
use alumina_interface_client::clock::{ClockProbeError, DeviceClockModel};
use alumina_interface_client::schedule::{
    ParticipantScheduleMachine, ParticipantSchedulePhase, ScheduleControlError, ScheduleOperation,
};
use alumina_interface_core::CanonicalGlobalJob2;
use alumina_job::{JobCommitId, JobCommitRequest, JobStartObservationSource};
use alumina_protocol::{DeviceCycle, DeviceId};
use alumina_storage::{ObjectKind, PublishedObject};

use crate::cache_delivery::ParticipantCacheReady;

const JOB_AXES: usize = 2;

/// Boot-local preparation identity selected for one cached participant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParticipantPreparation {
    /// Stable physical MCU identity.
    pub device_id: DeviceId,
    /// Authentication/clock boot identity that invalidates prepared work on reset.
    pub boot_id: BootId,
    /// Nonzero lifecycle correlation unique within that device boot.
    pub prepare_id: u64,
}

/// Fully decoded cached descriptor and boot identity supplied by a worker handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CachedParticipantSchedule {
    /// Stable MCU identity from the canonical global manifest.
    pub device_id: DeviceId,
    /// Current authenticated boot that derives the prepare receipt.
    pub boot_id: BootId,
    /// Exact descriptor whose partition was observed in this MCU's cache.
    pub descriptor: alumina_job::JobDescriptor,
}

/// Manifest-wide identities needed by the schedule protocol after cache delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DistributedJobIdentity {
    /// SHA-256 of the complete canonical global manifest.
    pub global_job_digest: alumina_protocol::Digest,
    /// Digest of the sorted participant-record set.
    pub participant_set_digest: alumina_protocol::Digest,
    /// Integer ticks per exact global second.
    pub global_timebase_hz: u64,
    /// Exact terminal global tick.
    pub duration_ticks: u64,
    /// Network-loss behavior committed by the manifest.
    pub network_policy: alumina_job::JobNetworkPolicy,
    /// Exact manifest publication observed on every participant MCU.
    pub global_manifest: PublishedObject,
}

/// Exact device-cycle margins derived from capabilities and UI scheduling policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParticipantStartTiming {
    /// Maximum clock interval radius accepted from this device model.
    pub maximum_uncertainty_cycles: u64,
    /// Required local synchronization certificate copied into the commit.
    pub required_sync_tolerance_cycles: u64,
    /// Cycles from confirmation deadline to scheduled start.
    pub confirmation_lead_cycles: u64,
    /// Cycles from irreversible abort guard to scheduled start.
    pub abort_guard_lead_cycles: u64,
    /// Finite execution lease after scheduled start.
    pub lease_cycles: u64,
    /// Unique browser-selected identity for this device-local commit.
    pub commit_id: JobCommitId,
}

/// Borrowed clock model and timing policy for one atomic global start plan.
pub struct ParticipantStartInput<'a> {
    /// Stable device identity used to align sorted participant records.
    pub device_id: DeviceId,
    /// Fresh exact causal clock model for the same boot used by preparation.
    pub clock: &'a DeviceClockModel,
    /// Capability/policy-derived local cycle margins.
    pub timing: ParticipantStartTiming,
}

/// One device-targeted canonical operation ready for its independent HMAC session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributedScheduleRequest {
    /// Stable target MCU.
    pub device_id: DeviceId,
    /// Exact operation and canonical native body.
    pub request: ScheduleOperation,
}

/// One participant's authenticated start observation mapped into browser time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParticipantStartReconciliation {
    /// Stable MCU identity owning the boot, commit, and observation.
    pub device_id: DeviceId,
    /// Explicit authority; simulator and software brackets remain distinguishable.
    pub source: JobStartObservationSource,
    /// Backend-local nonzero output correlation.
    pub output_token: u32,
    /// First job-owned output cycle from the installed commit.
    pub scheduled_cycle: alumina_protocol::DeviceCycle,
    /// Conservative device-cycle observation bounds.
    pub earliest_cycle: alumina_protocol::DeviceCycle,
    pub latest_cycle: alumina_protocol::DeviceCycle,
    /// Conservative browser monotonic interval from exact affine inversion.
    pub earliest_ui_ns: u64,
    pub midpoint_ui_ns: u64,
    pub latest_ui_ns: u64,
    /// Largest distance from the shared target to either mapped endpoint.
    pub maximum_target_error_ns: u64,
}

/// Conservative replay of every participant's observed start edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributedStartReconciliation {
    /// Shared browser epoch used to construct all installed commits.
    pub target_ui_ns: u64,
    /// Outer union of every participant's mapped start interval.
    pub earliest_ui_ns: u64,
    pub latest_ui_ns: u64,
    /// Worst possible participant-to-participant spread within the outer union.
    pub maximum_edge_spread_ns: u64,
    /// Largest participant endpoint displacement from the shared target.
    pub maximum_target_error_ns: u64,
    /// Canonically device-sorted participant evidence.
    pub participants: Vec<ParticipantStartReconciliation>,
}

/// Global browser interpretation of the multi-participant schedule lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributedSchedulePhase {
    /// Participants are installing cached descriptors and boot-bound receipts.
    Preparing,
    /// Every participant is cache/prefetch ready for one future-time plan.
    Ready,
    /// Exact per-device commits are being installed without start authority.
    Installing,
    /// Every participant reported the exact commit installed.
    Installed,
    /// Confirm references are being reconciled after all installs succeeded.
    Confirming,
    /// Every participant reported start authority while abort remains possible.
    Confirmed,
    /// At least one participant crossed the abort guard or began execution.
    Irrevocable,
    /// Reachable installed/confirmed participants are being aborted.
    Aborting,
    /// Every potentially installed participant is safely aborted or expired.
    Aborted,
    /// Some participants were safely stopped while others completed after the abort guard.
    SplitAfterAbort,
    /// Prepared participant actors are being cancelled before commit binding.
    Cancelling,
    /// Every precommit participant is absent or fully cancelled.
    Cancelled,
    /// Every participant reported exact completion.
    Complete,
    /// A terminal or inconsistent participant state prevented the global job.
    Faulted,
}

/// Conservative browser view of the remote confirmation/abort deadlines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributedDeadlineWindow {
    /// Every participant is provably before its confirmation deadline.
    ConfirmationOpen,
    /// Confirmation is no longer globally safe, but every abort guard is open.
    AbortOnly,
    /// At least one device may have reached its abort guard.
    PointOfNoReturn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoordinationIntent {
    Prepare,
    Install,
    Confirm,
    Abort,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParticipantCoordinator {
    device_id: DeviceId,
    control: ParticipantScheduleMachine<JOB_AXES>,
    planned_commit: Option<JobCommitRequest>,
}

/// Ordered global coordinator that never confirms before every exact install.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributedScheduleCoordinator {
    participants: Vec<ParticipantCoordinator>,
    global_job_digest: alumina_protocol::Digest,
    participant_set_digest: alumina_protocol::Digest,
    global_timebase_hz: u64,
    duration_ticks: u64,
    network_policy: alumina_job::JobNetworkPolicy,
    target_ui_ns: Option<u64>,
    intent: CoordinationIntent,
}

impl DistributedScheduleCoordinator {
    /// Binds cache-ready proofs, canonical participant packages, and boot identities.
    ///
    /// Every input set is sorted and compared atomically before any `JobPrepare`
    /// can be emitted. A cache proof for another partition, manifest, or device
    /// is rejected even when vector lengths happen to agree.
    ///
    /// # Errors
    ///
    /// Returns a participant, cache-publication, descriptor, or allocation
    /// error before constructing a partially usable coordinator.
    pub fn after_cache(
        job: &CanonicalGlobalJob2,
        cache_ready: &[ParticipantCacheReady],
        preparations: &[ParticipantPreparation],
    ) -> Result<Self, DistributedScheduleError> {
        if cache_ready.len() != job.participants().len()
            || preparations.len() != job.participants().len()
        {
            return Err(DistributedScheduleError::ParticipantSet);
        }
        let mut ready = cache_ready.to_vec();
        ready.sort_unstable_by_key(|item| item.device_id());
        let mut preparation = preparations.to_vec();
        preparation.sort_unstable_by_key(|item| item.device_id);

        let mut cached = Vec::new();
        cached
            .try_reserve_exact(job.participants().len())
            .map_err(|_| DistributedScheduleError::AllocationOverflow)?;
        for (index, package) in job.participants().iter().enumerate() {
            let ready = ready[index];
            let preparation = preparation[index];
            if ready.device_id() != package.device_id()
                || preparation.device_id != package.device_id()
                || ready.partition() != package.partition().publication()
                || ready.global_manifest() != job.publication()
            {
                return Err(DistributedScheduleError::ParticipantSet);
            }
            let descriptor = package.partition().job_descriptor(preparation.prepare_id)?;
            cached.push(CachedParticipantSchedule {
                device_id: package.device_id(),
                boot_id: preparation.boot_id,
                descriptor,
            });
        }
        let global = job.policy().global();
        Self::after_cached_artifacts(
            DistributedJobIdentity {
                global_job_digest: job.global_job_digest(),
                participant_set_digest: job.participant_set_digest(),
                global_timebase_hz: global.global_timebase_hz,
                duration_ticks: global.duration_ticks,
                network_policy: global.network_policy,
                global_manifest: job.publication(),
            },
            &ready,
            &cached,
        )
    }

    /// Constructs the same coordinator from independently decoded worker artifacts.
    ///
    /// This is the strict UI/worker seam: the caller cannot substitute a
    /// descriptor, device, partition publication, or global manifest after the
    /// cache proofs have been formed.
    ///
    /// # Errors
    ///
    /// Rejects an empty, mismatched, duplicated, or substituted participant
    /// set, allocation failure, and any invalid participant schedule descriptor.
    pub fn after_cached_artifacts(
        identity: DistributedJobIdentity,
        cache_ready: &[ParticipantCacheReady],
        cached: &[CachedParticipantSchedule],
    ) -> Result<Self, DistributedScheduleError> {
        if cache_ready.is_empty()
            || cache_ready.len() != cached.len()
            || identity.global_job_digest.is_zero()
            || identity.participant_set_digest.is_zero()
            || identity.global_timebase_hz == 0
            || identity.duration_ticks == 0
            || identity.global_manifest.object.kind != ObjectKind::MachineJobManifest
            || identity.global_manifest.object.content.digest != identity.global_job_digest
        {
            return Err(DistributedScheduleError::ParticipantSet);
        }
        let mut ready = cache_ready.to_vec();
        ready.sort_unstable_by_key(|item| item.device_id());
        let mut cached = cached.to_vec();
        cached.sort_unstable_by_key(|item| item.device_id);
        let mut participants = Vec::new();
        participants
            .try_reserve_exact(cached.len())
            .map_err(|_| DistributedScheduleError::AllocationOverflow)?;
        for (ready, cached) in ready.into_iter().zip(cached) {
            if ready.device_id() != cached.device_id
                || ready.partition() != cached.descriptor.partition
                || ready.global_manifest() != identity.global_manifest
            {
                return Err(DistributedScheduleError::ParticipantSet);
            }
            participants.push(ParticipantCoordinator {
                device_id: cached.device_id,
                control: ParticipantScheduleMachine::new(cached.descriptor, cached.boot_id)?,
                planned_commit: None,
            });
        }
        Ok(Self {
            participants,
            global_job_digest: identity.global_job_digest,
            participant_set_digest: identity.participant_set_digest,
            global_timebase_hz: identity.global_timebase_hz,
            duration_ticks: identity.duration_ticks,
            network_policy: identity.network_policy,
            target_ui_ns: None,
            intent: CoordinationIntent::Prepare,
        })
    }

    /// Current global phase derived from every exact participant report.
    #[must_use]
    pub fn phase(&self) -> DistributedSchedulePhase {
        if self
            .participants
            .iter()
            .all(|participant| participant.control.phase() == ParticipantSchedulePhase::Complete)
        {
            return DistributedSchedulePhase::Complete;
        }
        match self.intent {
            CoordinationIntent::Prepare => {
                if self
                    .participants
                    .iter()
                    .any(|participant| participant.control.report().is_none())
                {
                    DistributedSchedulePhase::Preparing
                } else if self.participants.iter().all(|participant| {
                    participant.control.phase() == ParticipantSchedulePhase::Ready
                }) {
                    DistributedSchedulePhase::Ready
                } else if self.participants.iter().any(|participant| {
                    terminal_failure(participant.control.phase())
                        || post_guard(participant.control.phase())
                }) {
                    DistributedSchedulePhase::Faulted
                } else {
                    DistributedSchedulePhase::Preparing
                }
            }
            CoordinationIntent::Install => {
                if self.participants.iter().all(|participant| {
                    participant.control.phase() == ParticipantSchedulePhase::Installed
                }) {
                    DistributedSchedulePhase::Installed
                } else if self
                    .participants
                    .iter()
                    .any(|participant| terminal_failure(participant.control.phase()))
                {
                    DistributedSchedulePhase::Faulted
                } else if self
                    .participants
                    .iter()
                    .any(|participant| post_guard(participant.control.phase()))
                {
                    DistributedSchedulePhase::Irrevocable
                } else {
                    DistributedSchedulePhase::Installing
                }
            }
            CoordinationIntent::Confirm => {
                let all_confirmed = self.participants.iter().all(|participant| {
                    participant.control.phase() == ParticipantSchedulePhase::Confirmed
                });
                if all_confirmed {
                    DistributedSchedulePhase::Confirmed
                } else if self
                    .participants
                    .iter()
                    .any(|participant| post_guard(participant.control.phase()))
                {
                    DistributedSchedulePhase::Irrevocable
                } else if self
                    .participants
                    .iter()
                    .any(|participant| terminal_failure(participant.control.phase()))
                {
                    DistributedSchedulePhase::Faulted
                } else {
                    DistributedSchedulePhase::Confirming
                }
            }
            CoordinationIntent::Abort => {
                if self.participants.iter().all(participant_safe_after_abort) {
                    DistributedSchedulePhase::Aborted
                } else if split_after_abort_is_terminal(&self.participants) {
                    DistributedSchedulePhase::SplitAfterAbort
                } else if self
                    .participants
                    .iter()
                    .any(|participant| post_guard(participant.control.phase()))
                {
                    DistributedSchedulePhase::Irrevocable
                } else {
                    DistributedSchedulePhase::Aborting
                }
            }
            CoordinationIntent::Cancel => {
                if self.participants.iter().all(|participant| {
                    matches!(
                        participant.control.phase(),
                        ParticipantSchedulePhase::Empty | ParticipantSchedulePhase::Cancelled
                    )
                }) {
                    DistributedSchedulePhase::Cancelled
                } else {
                    DistributedSchedulePhase::Cancelling
                }
            }
        }
    }

    /// Shared future browser-worker epoch selected for all participant commits.
    #[must_use]
    pub const fn target_ui_ns(&self) -> Option<u64> {
        self.target_ui_ns
    }

    /// Exact observed phase for one stable device identity.
    #[must_use]
    pub fn participant_phase(&self, device_id: DeviceId) -> Option<ParticipantSchedulePhase> {
        self.participant(device_id)
            .map(|participant| participant.control.phase())
    }

    /// Exact planned commit for one participant after start binding.
    #[must_use]
    pub fn participant_commit(&self, device_id: DeviceId) -> Option<JobCommitRequest> {
        self.participant(device_id)?.planned_commit
    }

    /// Device-reported local start cycle after exact descriptor reconciliation.
    #[must_use]
    pub fn participant_local_start_cycle(&self, device_id: DeviceId) -> Option<DeviceCycle> {
        let participant = self.participant(device_id)?;
        participant
            .control
            .commit()
            .map(|commit| commit.local_start_cycle)
            .or_else(|| {
                participant
                    .control
                    .report()
                    .and_then(|report| report.schedule)
                    .filter(|schedule| schedule.policy.is_some())
                    .map(|schedule| schedule.local_start_cycle)
            })
    }

    /// Replays authenticated device-cycle start evidence into browser time.
    ///
    /// This operation is read-only and preserves each observation authority.
    /// A simulated latch or software bracket therefore cannot be presented as a
    /// physical peripheral capture. Every report must match the already bound
    /// participant, boot, and local start cycle before any aggregate is returned.
    ///
    /// # Errors
    ///
    /// Returns an error when the participant set, clock boot, installed commit,
    /// observation identity, clock freshness, or requested uncertainty bound is
    /// not exact and complete.
    pub fn reconcile_start_observations(
        &self,
        now_ui_ns: u64,
        inputs: &[ParticipantStartInput<'_>],
        maximum_mapping_uncertainty_ns: u64,
    ) -> Result<DistributedStartReconciliation, DistributedScheduleError> {
        let target_ui_ns = self.target_ui_ns.ok_or(DistributedScheduleError::State)?;
        if inputs.len() != self.participants.len() || self.participants.is_empty() {
            return Err(DistributedScheduleError::ParticipantSet);
        }
        let mut order: Vec<usize> = (0..inputs.len()).collect();
        order.sort_unstable_by_key(|index| inputs[*index].device_id);
        let mut reconciled = Vec::new();
        reconciled
            .try_reserve_exact(self.participants.len())
            .map_err(|_| DistributedScheduleError::AllocationOverflow)?;

        for (participant, input_index) in self.participants.iter().zip(order) {
            let input = &inputs[input_index];
            if input.device_id != participant.device_id
                || input.clock.boot_id() != Some(participant.control.boot_id())
            {
                return Err(DistributedScheduleError::ParticipantSet);
            }
            let commit = participant
                .planned_commit
                .ok_or(DistributedScheduleError::State)?;
            let observation = participant
                .control
                .report()
                .and_then(|report| report.schedule)
                .and_then(|schedule| schedule.start_observation)
                .ok_or(DistributedScheduleError::StartObservationUnavailable)?;
            if observation.scheduled_cycle != commit.local_start_cycle {
                return Err(DistributedScheduleError::StartObservationIdentity);
            }
            let estimate = input.clock.estimate_ui_for_cycle_interval(
                now_ui_ns,
                observation.earliest_cycle,
                observation.latest_cycle,
                maximum_mapping_uncertainty_ns,
            )?;
            if estimate.boot_id != participant.control.boot_id() {
                return Err(DistributedScheduleError::StartObservationIdentity);
            }
            reconciled.push(ParticipantStartReconciliation {
                device_id: participant.device_id,
                source: observation.source,
                output_token: observation.output_token,
                scheduled_cycle: observation.scheduled_cycle,
                earliest_cycle: observation.earliest_cycle,
                latest_cycle: observation.latest_cycle,
                earliest_ui_ns: estimate.earliest_ui_ns,
                midpoint_ui_ns: estimate.midpoint_ui_ns,
                latest_ui_ns: estimate.latest_ui_ns,
                maximum_target_error_ns: estimate
                    .earliest_ui_ns
                    .abs_diff(target_ui_ns)
                    .max(estimate.latest_ui_ns.abs_diff(target_ui_ns)),
            });
        }

        let earliest_ui_ns = reconciled
            .iter()
            .map(|participant| participant.earliest_ui_ns)
            .min()
            .ok_or(DistributedScheduleError::StartObservationUnavailable)?;
        let latest_ui_ns = reconciled
            .iter()
            .map(|participant| participant.latest_ui_ns)
            .max()
            .ok_or(DistributedScheduleError::StartObservationUnavailable)?;
        let maximum_target_error_ns = reconciled
            .iter()
            .map(|participant| participant.maximum_target_error_ns)
            .max()
            .ok_or(DistributedScheduleError::StartObservationUnavailable)?;
        Ok(DistributedStartReconciliation {
            target_ui_ns,
            earliest_ui_ns,
            latest_ui_ns,
            maximum_edge_spread_ns: latest_ui_ns - earliest_ui_ns,
            maximum_target_error_ns,
            participants: reconciled,
        })
    }

    /// Conservatively classifies all participant deadlines at one UI instant.
    ///
    /// Each device's exact causal interval is checked independently. An interval
    /// touching a deadline closes that operation because firmware rejects at
    /// equality. A stale, unhealthy, changed-boot, or overly uncertain model is
    /// an error rather than evidence that either window remains open.
    ///
    /// # Errors
    ///
    /// Rejects an unbound start, a mismatched participant set, unavailable
    /// commit, or any clock-policy/identity failure.
    pub fn deadline_window(
        &self,
        now_ui_ns: u64,
        inputs: &[ParticipantStartInput<'_>],
    ) -> Result<DistributedDeadlineWindow, DistributedScheduleError> {
        if self.target_ui_ns.is_none() || inputs.len() != self.participants.len() {
            return Err(DistributedScheduleError::State);
        }
        let mut order: Vec<usize> = (0..inputs.len()).collect();
        order.sort_unstable_by_key(|index| inputs[*index].device_id);
        let mut confirmation_open = true;
        for (participant, input_index) in self.participants.iter().zip(order) {
            let input = &inputs[input_index];
            if input.device_id != participant.device_id
                || input.clock.boot_id() != Some(participant.control.boot_id())
            {
                return Err(DistributedScheduleError::ParticipantSet);
            }
            validate_timing(input.timing)?;
            let estimate = input
                .clock
                .estimate_at(now_ui_ns, input.timing.maximum_uncertainty_cycles)?;
            if estimate.boot_id != participant.control.boot_id()
                || estimate.ui_ns != now_ui_ns
                || estimate.uncertainty_cycles > input.timing.required_sync_tolerance_cycles
            {
                return Err(DistributedScheduleError::ClockIdentity);
            }
            let commit = participant
                .planned_commit
                .ok_or(DistributedScheduleError::State)?;
            if estimate.latest_cycle.0 >= commit.abort_guard_cycle.0 {
                return Ok(DistributedDeadlineWindow::PointOfNoReturn);
            }
            if estimate.latest_cycle.0 >= commit.confirm_deadline_cycle.0 {
                confirmation_open = false;
            }
        }
        Ok(if confirmation_open {
            DistributedDeadlineWindow::ConfirmationOpen
        } else {
            DistributedDeadlineWindow::AbortOnly
        })
    }

    /// Atomically maps one global future UI epoch into every local commit.
    ///
    /// Clock predictions are produced here from fresh boot-bound models. The
    /// confirmation deadline must remain strictly after each prediction's
    /// conservative latest current cycle. The finite lease must cover the
    /// manifest's exact global duration after ceiling conversion into that
    /// device's declared clock domain.
    ///
    /// # Errors
    ///
    /// Returns without binding any commit if participant identity, clock
    /// freshness, timing order, uncertainty, duration, or canonical encoding
    /// fails for any device.
    pub fn bind_start(
        &mut self,
        now_ui_ns: u64,
        target_ui_ns: u64,
        inputs: &[ParticipantStartInput<'_>],
    ) -> Result<(), DistributedScheduleError> {
        if self.phase() != DistributedSchedulePhase::Ready || self.target_ui_ns.is_some() {
            return Err(DistributedScheduleError::State);
        }
        if inputs.len() != self.participants.len() {
            return Err(DistributedScheduleError::ParticipantSet);
        }
        let mut order: Vec<usize> = (0..inputs.len()).collect();
        order.sort_unstable_by_key(|index| inputs[*index].device_id);
        let mut commits = Vec::new();
        commits
            .try_reserve_exact(self.participants.len())
            .map_err(|_| DistributedScheduleError::AllocationOverflow)?;

        for (participant, input_index) in self.participants.iter().zip(order) {
            let input = &inputs[input_index];
            if input.device_id != participant.device_id
                || input.clock.boot_id() != Some(participant.control.boot_id())
            {
                return Err(DistributedScheduleError::ParticipantSet);
            }
            let timing = input.timing;
            validate_timing(timing)?;
            let prediction =
                input
                    .clock
                    .predict(now_ui_ns, target_ui_ns, timing.maximum_uncertainty_cycles)?;
            let latest = input
                .clock
                .latest_response()
                .ok_or(DistributedScheduleError::ClockUnavailable)?;
            if prediction.boot_id != participant.control.boot_id()
                || prediction.target_ui_ns != target_ui_ns
                || prediction.now_ui_ns != now_ui_ns
                || prediction.uncertainty_cycles > timing.required_sync_tolerance_cycles
            {
                return Err(DistributedScheduleError::ClockIdentity);
            }
            let start = prediction.scheduled_cycle.0;
            let confirm = start
                .checked_sub(timing.confirmation_lead_cycles)
                .ok_or(DistributedScheduleError::Timing)?;
            let abort = start
                .checked_sub(timing.abort_guard_lead_cycles)
                .ok_or(DistributedScheduleError::Timing)?;
            let lease = start
                .checked_add(timing.lease_cycles)
                .ok_or(DistributedScheduleError::Timing)?;
            if confirm <= prediction.now_latest_cycle.0 || confirm >= abort || abort >= start {
                return Err(DistributedScheduleError::Timing);
            }
            let required_job_cycles = ceil_product_ratio(
                self.duration_ticks,
                latest.frequency_hz,
                self.global_timebase_hz,
            )?;
            if timing.lease_cycles < required_job_cycles {
                return Err(DistributedScheduleError::LeaseTooShort {
                    supplied: timing.lease_cycles,
                    required: required_job_cycles,
                });
            }
            let descriptor = participant.control.descriptor();
            let commit = JobCommitRequest {
                policy: self.network_policy,
                prepare_id: descriptor.prepare_id,
                boot_id: participant.control.boot_id(),
                global_job_digest: self.global_job_digest,
                participant_set_digest: self.participant_set_digest,
                prepared_token: participant.control.prepared_token(),
                partition_digest: descriptor.partition.object.content.digest,
                local_start_cycle: alumina_protocol::DeviceCycle(start),
                confirm_deadline_cycle: alumina_protocol::DeviceCycle(confirm),
                abort_guard_cycle: alumina_protocol::DeviceCycle(abort),
                lease_expiry_cycle: alumina_protocol::DeviceCycle(lease),
                clock_probe_id: prediction.latest_probe_id,
                clock_uncertainty_cycles: prediction.uncertainty_cycles,
                required_sync_tolerance_cycles: timing.required_sync_tolerance_cycles,
                commit_id: timing.commit_id,
            };
            commit.encode()?;
            commits.push(commit);
        }

        for (participant, commit) in self.participants.iter_mut().zip(commits) {
            participant.planned_commit = Some(commit);
        }
        self.target_ui_ns = Some(target_ui_ns);
        self.intent = CoordinationIntent::Install;
        Ok(())
    }

    /// Opens the confirmation phase only after every exact commit was observed installed.
    ///
    /// # Errors
    ///
    /// Returns [`DistributedScheduleError::State`] unless all participants are
    /// in the exact installed phase.
    pub fn begin_confirmation(
        &mut self,
        now_ui_ns: u64,
        inputs: &[ParticipantStartInput<'_>],
    ) -> Result<(), DistributedScheduleError> {
        if self.phase() != DistributedSchedulePhase::Installed {
            return Err(DistributedScheduleError::State);
        }
        match self.deadline_window(now_ui_ns, inputs)? {
            DistributedDeadlineWindow::ConfirmationOpen => {}
            DistributedDeadlineWindow::AbortOnly => {
                return Err(DistributedScheduleError::ConfirmationClosed);
            }
            DistributedDeadlineWindow::PointOfNoReturn => {
                return Err(DistributedScheduleError::PointOfNoReturn);
            }
        }
        self.intent = CoordinationIntent::Confirm;
        Ok(())
    }

    /// Switches all still-reachable installed work to pre-guard abort reconciliation.
    ///
    /// # Errors
    ///
    /// Rejects a completed/already-aborted job or any state known to be beyond
    /// the remote abort point of no return.
    pub fn begin_abort(&mut self) -> Result<(), DistributedScheduleError> {
        if self.target_ui_ns.is_none() {
            return Err(DistributedScheduleError::State);
        }
        if matches!(
            self.phase(),
            DistributedSchedulePhase::Irrevocable
                | DistributedSchedulePhase::Complete
                | DistributedSchedulePhase::Aborted
                | DistributedSchedulePhase::SplitAfterAbort
        ) {
            return Err(DistributedScheduleError::PointOfNoReturn);
        }
        self.intent = CoordinationIntent::Abort;
        Ok(())
    }

    /// Switches a precommit preparation to idempotent actor cleanup.
    ///
    /// # Errors
    ///
    /// Rejects cleanup after any future commit has been bound or after a prior
    /// cleanup has reached its terminal global state.
    pub fn begin_cancel(&mut self) -> Result<(), DistributedScheduleError> {
        if self.target_ui_ns.is_some()
            || matches!(
                self.phase(),
                DistributedSchedulePhase::Cancelled | DistributedSchedulePhase::Complete
            )
        {
            return Err(DistributedScheduleError::State);
        }
        self.intent = CoordinationIntent::Cancel;
        Ok(())
    }

    /// Starts one concurrent read-only status round across every participant.
    ///
    /// This allows the browser worker to reconcile priming, observed start,
    /// completion, and fault transitions after mutation traffic stops.
    ///
    /// # Errors
    ///
    /// Rejects a round while any participant already owns a pending operation
    /// or when request-vector allocation cannot be completed atomically.
    pub fn begin_status_round(
        &mut self,
    ) -> Result<Vec<DistributedScheduleRequest>, DistributedScheduleError> {
        if self
            .participants
            .iter()
            .any(|participant| participant.control.has_pending_request())
        {
            return Err(DistributedScheduleError::ParticipantState);
        }
        let mut requests = Vec::new();
        requests
            .try_reserve_exact(self.participants.len())
            .map_err(|_| DistributedScheduleError::AllocationOverflow)?;
        for participant in &mut self.participants {
            requests.push(DistributedScheduleRequest {
                device_id: participant.device_id,
                request: participant.control.begin_status()?,
            });
        }
        Ok(requests)
    }

    /// Emits the next unique participant operation; independent callers may keep
    /// one request pending per device and therefore perform Wi-Fi I/O concurrently.
    ///
    /// # Errors
    ///
    /// Returns a participant state/control error when observed device state no
    /// longer agrees with the global prepare/install/confirm/abort intent.
    pub fn next_request(
        &mut self,
    ) -> Result<Option<DistributedScheduleRequest>, DistributedScheduleError> {
        if matches!(
            self.phase(),
            DistributedSchedulePhase::Aborted
                | DistributedSchedulePhase::SplitAfterAbort
                | DistributedSchedulePhase::Cancelled
                | DistributedSchedulePhase::Complete
                | DistributedSchedulePhase::Faulted
        ) {
            return Ok(None);
        }
        let initial_status_round = self.intent == CoordinationIntent::Prepare
            && self
                .participants
                .iter()
                .any(|participant| participant.control.report().is_none());
        for participant in &mut self.participants {
            if participant.control.has_pending_request() {
                continue;
            }
            let operation =
                if participant.control.reconciliation_required() {
                    Some(participant.control.begin_status()?)
                } else if initial_status_round {
                    if participant.control.report().is_none() {
                        Some(participant.control.begin_status()?)
                    } else {
                        None
                    }
                } else {
                    let phase = participant.control.phase();
                    match self.intent {
                        CoordinationIntent::Prepare => match phase {
                            ParticipantSchedulePhase::Empty => {
                                Some(participant.control.begin_prepare()?)
                            }
                            ParticipantSchedulePhase::Preparing => {
                                Some(participant.control.begin_status()?)
                            }
                            ParticipantSchedulePhase::Ready => None,
                            _ => return Err(DistributedScheduleError::ParticipantState),
                        },
                        CoordinationIntent::Install => match phase {
                            ParticipantSchedulePhase::Ready
                            | ParticipantSchedulePhase::Installing => Some(
                                participant.control.begin_install(
                                    participant
                                        .planned_commit
                                        .ok_or(DistributedScheduleError::State)?,
                                )?,
                            ),
                            ParticipantSchedulePhase::Installed => None,
                            _ => return Err(DistributedScheduleError::ParticipantState),
                        },
                        CoordinationIntent::Confirm => match phase {
                            ParticipantSchedulePhase::Installed => {
                                Some(participant.control.begin_confirm()?)
                            }
                            ParticipantSchedulePhase::Confirmed => None,
                            phase if post_guard(phase) => None,
                            _ => return Err(DistributedScheduleError::ParticipantState),
                        },
                        CoordinationIntent::Abort => next_abort_operation(participant)?,
                        CoordinationIntent::Cancel => match phase {
                            ParticipantSchedulePhase::Empty
                            | ParticipantSchedulePhase::Cancelled => None,
                            ParticipantSchedulePhase::Preparing
                            | ParticipantSchedulePhase::Ready
                            | ParticipantSchedulePhase::Faulted
                                if participant.control.commit().is_none() =>
                            {
                                Some(participant.control.begin_cancel()?)
                            }
                            _ => return Err(DistributedScheduleError::ParticipantState),
                        },
                    }
                };
            if let Some(request) = operation {
                return Ok(Some(DistributedScheduleRequest {
                    device_id: participant.device_id,
                    request,
                }));
            }
        }
        Ok(None)
    }

    /// Applies one authenticated/correlated response to its stable device owner.
    ///
    /// # Errors
    ///
    /// Rejects an unknown device or any malformed, foreign, or illegal
    /// participant response without advancing global authority.
    pub fn accept_response(
        &mut self,
        device_id: DeviceId,
        response: &Response,
    ) -> Result<DistributedSchedulePhase, DistributedScheduleError> {
        self.participant_mut(device_id)?
            .control
            .accept_response(response)?;
        Ok(self.phase())
    }

    /// Marks one device request ambiguous, forcing read-only reconciliation.
    ///
    /// # Errors
    ///
    /// Returns a participant-set error when `device_id` is not in this job.
    pub fn abandon_pending(
        &mut self,
        device_id: DeviceId,
    ) -> Result<bool, DistributedScheduleError> {
        Ok(self.participant_mut(device_id)?.control.abandon_pending())
    }

    fn participant(&self, device_id: DeviceId) -> Option<&ParticipantCoordinator> {
        self.participants
            .binary_search_by_key(&device_id, |participant| participant.device_id)
            .ok()
            .map(|index| &self.participants[index])
    }

    fn participant_mut(
        &mut self,
        device_id: DeviceId,
    ) -> Result<&mut ParticipantCoordinator, DistributedScheduleError> {
        let index = self
            .participants
            .binary_search_by_key(&device_id, |participant| participant.device_id)
            .map_err(|_| DistributedScheduleError::ParticipantSet)?;
        Ok(&mut self.participants[index])
    }
}

fn validate_timing(timing: ParticipantStartTiming) -> Result<(), DistributedScheduleError> {
    if timing.maximum_uncertainty_cycles == 0
        || timing.required_sync_tolerance_cycles == 0
        || timing.maximum_uncertainty_cycles > timing.required_sync_tolerance_cycles
        || timing.confirmation_lead_cycles <= timing.abort_guard_lead_cycles
        || timing.abort_guard_lead_cycles == 0
        || timing.lease_cycles == 0
    {
        Err(DistributedScheduleError::Timing)
    } else {
        Ok(())
    }
}

fn ceil_product_ratio(
    value: u64,
    multiplier: u64,
    divisor: u64,
) -> Result<u64, DistributedScheduleError> {
    if divisor == 0 {
        return Err(DistributedScheduleError::Timing);
    }
    let numerator = u128::from(value)
        .checked_mul(u128::from(multiplier))
        .ok_or(DistributedScheduleError::Timing)?;
    let divisor = u128::from(divisor);
    let rounded = numerator
        .checked_add(divisor - 1)
        .ok_or(DistributedScheduleError::Timing)?
        / divisor;
    u64::try_from(rounded).map_err(|_| DistributedScheduleError::Timing)
}

const fn terminal_failure(phase: ParticipantSchedulePhase) -> bool {
    matches!(
        phase,
        ParticipantSchedulePhase::Aborted
            | ParticipantSchedulePhase::Cancelled
            | ParticipantSchedulePhase::Expired
            | ParticipantSchedulePhase::Faulted
    )
}

const fn post_guard(phase: ParticipantSchedulePhase) -> bool {
    matches!(
        phase,
        ParticipantSchedulePhase::Priming
            | ParticipantSchedulePhase::Primed
            | ParticipantSchedulePhase::Running
            | ParticipantSchedulePhase::Complete
    )
}

fn participant_safe_after_abort(participant: &ParticipantCoordinator) -> bool {
    if participant.control.commit().is_none() {
        return matches!(
            participant.control.phase(),
            ParticipantSchedulePhase::Empty | ParticipantSchedulePhase::Cancelled
        );
    }
    matches!(
        participant.control.phase(),
        ParticipantSchedulePhase::Aborted | ParticipantSchedulePhase::Expired
    )
}

fn next_abort_operation(
    participant: &mut ParticipantCoordinator,
) -> Result<Option<ScheduleOperation>, DistributedScheduleError> {
    let phase = participant.control.phase();
    if participant.control.commit().is_none() {
        return match phase {
            ParticipantSchedulePhase::Empty | ParticipantSchedulePhase::Cancelled => Ok(None),
            ParticipantSchedulePhase::Preparing
            | ParticipantSchedulePhase::Ready
            | ParticipantSchedulePhase::Faulted => participant
                .control
                .begin_cancel()
                .map(Some)
                .map_err(Into::into),
            _ => Err(DistributedScheduleError::ParticipantState),
        };
    }
    match phase {
        ParticipantSchedulePhase::Installing
        | ParticipantSchedulePhase::Installed
        | ParticipantSchedulePhase::Confirmed => participant
            .control
            .begin_abort()
            .map(Some)
            .map_err(Into::into),
        ParticipantSchedulePhase::Aborted | ParticipantSchedulePhase::Expired => Ok(None),
        phase if post_guard(phase) => Ok(None),
        _ => Err(DistributedScheduleError::ParticipantState),
    }
}

fn split_after_abort_is_terminal(participants: &[ParticipantCoordinator]) -> bool {
    let mut stopped = false;
    let mut completed = false;
    for participant in participants {
        match participant.control.phase() {
            ParticipantSchedulePhase::Aborted | ParticipantSchedulePhase::Expired => {
                stopped = true;
            }
            ParticipantSchedulePhase::Complete => completed = true,
            _ => return false,
        }
    }
    stopped && completed
}

/// Global cache, clock, timing, participant, or schedule-control rejection.
#[derive(Debug)]
pub enum DistributedScheduleError {
    /// Cache proofs, preparation records, clocks, or devices did not form one set.
    ParticipantSet,
    /// Global or participant coordinator was called outside its legal phase.
    State,
    /// A participant entered a state incompatible with the global intent.
    ParticipantState,
    /// Browser can no longer guarantee pre-guard abort.
    PointOfNoReturn,
    /// At least one participant may have reached its confirmation deadline.
    ConfirmationClosed,
    /// Clock model had no fully accepted heartbeat facts.
    ClockUnavailable,
    /// Clock boot, timestamps, or prediction did not match the participant plan.
    ClockIdentity,
    /// One or more participants have not reported their first output edge.
    StartObservationUnavailable,
    /// Start evidence did not match the participant boot or installed cycle.
    StartObservationIdentity,
    /// Cycle margins, order, duration conversion, or arithmetic were invalid.
    Timing,
    /// Finite lease did not cover the exact manifest duration.
    LeaseTooShort {
        /// Caller-supplied local device cycles.
        supplied: u64,
        /// Minimum exact ceiling-converted duration cycles.
        required: u64,
    },
    /// Vector allocation failed before any participant was mutated.
    AllocationOverflow,
    /// Canonical partition could not produce its firmware descriptor.
    Partition(alumina_interface_core::MachinePartitionError),
    /// Exact heartbeat model rejected prediction.
    Clock(ClockProbeError),
    /// Participant request or report state machine rejected an operation.
    Schedule(ScheduleControlError),
    /// Canonical commit construction failed.
    Commit(alumina_job::JobScheduleWireError),
}

impl From<alumina_interface_core::MachinePartitionError> for DistributedScheduleError {
    fn from(value: alumina_interface_core::MachinePartitionError) -> Self {
        Self::Partition(value)
    }
}

impl From<ClockProbeError> for DistributedScheduleError {
    fn from(value: ClockProbeError) -> Self {
        Self::Clock(value)
    }
}

impl From<ScheduleControlError> for DistributedScheduleError {
    fn from(value: ScheduleControlError) -> Self {
        Self::Schedule(value)
    }
}

impl From<alumina_job::JobScheduleWireError> for DistributedScheduleError {
    fn from(value: alumina_job::JobScheduleWireError) -> Self {
        Self::Commit(value)
    }
}

impl fmt::Display for DistributedScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParticipantSet => formatter.write_str("distributed participant set differs"),
            Self::State => formatter.write_str("distributed coordinator state rejects operation"),
            Self::ParticipantState => {
                formatter.write_str("participant phase conflicts with global intent")
            }
            Self::PointOfNoReturn => formatter.write_str("abort guard has closed"),
            Self::ConfirmationClosed => formatter.write_str("confirmation deadline has closed"),
            Self::ClockUnavailable => formatter.write_str("participant clock is unavailable"),
            Self::ClockIdentity => formatter.write_str("participant clock identity differs"),
            Self::StartObservationUnavailable => {
                formatter.write_str("participant start observation is unavailable")
            }
            Self::StartObservationIdentity => {
                formatter.write_str("participant start observation identity differs")
            }
            Self::Timing => formatter.write_str("distributed timing policy is invalid"),
            Self::LeaseTooShort { supplied, required } => write!(
                formatter,
                "execution lease {supplied} cycles is shorter than required {required}"
            ),
            Self::AllocationOverflow => formatter.write_str("participant allocation failed"),
            Self::Partition(error) => write!(formatter, "partition rejected: {error}"),
            Self::Clock(error) => write!(formatter, "clock rejected: {error}"),
            Self::Schedule(error) => write!(formatter, "schedule rejected: {error}"),
            Self::Commit(error) => write!(formatter, "commit rejected: {error:?}"),
        }
    }
}

impl std::error::Error for DistributedScheduleError {}

#[cfg(test)]
mod tests {
    use alumina_clock::{ClockEstimationPolicy, ClockFlags, ClockHeartbeatResponse, ClockSource};
    use alumina_interface_core::{
        compile_representative_global_job, compile_representative_program,
    };
    use alumina_job::{
        JOB_COMMIT_ID_BYTES, JobScheduleAdmission, JobStatusReport, PreparedJobSchedule,
        RealtimeJobReport, RealtimeJobState, ServiceJobReport, ServiceJobState,
    };
    use alumina_machine_ir::StreamTick;
    use alumina_protocol::{DeviceCycle, Digest, Operation, StatusCode};

    use super::*;

    struct Authority {
        device_id: DeviceId,
        schedule: PreparedJobSchedule,
        descriptor: alumina_job::JobDescriptor,
    }

    fn boot(byte: u8) -> BootId {
        BootId::new([byte; 16]).unwrap()
    }

    fn clock_model(boot_id: BootId, offset: u64) -> DeviceClockModel {
        let mut model = DeviceClockModel::new(ClockEstimationPolicy {
            maximum_round_trip_ns: 3_000_000,
            maximum_device_processing_cycles: 1_000,
            maximum_drift_ppm: 100,
            maximum_sample_age_ns: 10_000_000_000,
            maximum_schedule_horizon_ns: 10_000_000_000,
            minimum_schedule_lead_ns: 100_000_000,
            minimum_samples: 3,
        })
        .unwrap();
        for (index, (up, processing, down)) in [
            (120_000, 50_000, 700_000),
            (650_000, 60_000, 120_000),
            (300_000, 40_000, 600_000),
        ]
        .into_iter()
        .enumerate()
        {
            let send = u64::try_from(index + 1).unwrap() * 1_000_000_000;
            let request = model.begin_probe(send).unwrap();
            let receive_ns = send + up;
            let transmit_ns = receive_ns + processing;
            let browser_receive_ns = transmit_ns + down;
            let cycle_at = |ui_ns: u64| DeviceCycle(offset + ui_ns / 1_000);
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
            model
                .accept_response(
                    &Response {
                        status: StatusCode::Ok,
                        body: heartbeat.encode().unwrap().to_vec(),
                    },
                    browser_receive_ns,
                )
                .unwrap();
        }
        model
    }

    fn fixture() -> (
        CanonicalGlobalJob2,
        Vec<ParticipantCacheReady>,
        Vec<ParticipantPreparation>,
        Vec<DeviceClockModel>,
    ) {
        let program = compile_representative_program().unwrap();
        let job = compile_representative_global_job(&program).unwrap();
        let mut ready = Vec::new();
        let mut preparations = Vec::new();
        let mut clocks = Vec::new();
        for (index, participant) in job.participants().iter().enumerate() {
            ready.push(ParticipantCacheReady::for_test(
                participant.device_id(),
                participant.partition().publication(),
                job.publication(),
            ));
            let byte = 0x21 + u8::try_from(index).unwrap();
            let boot_id = boot(byte);
            preparations.push(ParticipantPreparation {
                device_id: participant.device_id(),
                boot_id,
                prepare_id: 100 + u64::try_from(index).unwrap(),
            });
            clocks.push(clock_model(
                boot_id,
                75_000 + u64::try_from(index).unwrap() * 250_000,
            ));
        }
        (job, ready, preparations, clocks)
    }

    fn ready_status(authority: &Authority) -> JobStatusReport {
        let descriptor = authority.descriptor;
        JobStatusReport {
            service: Some(ServiceJobReport {
                prepare_id: descriptor.prepare_id,
                state: ServiceJobState::Prefetching,
                axis_count: descriptor.axis_count,
                validated_blocks: 2.min(descriptor.block_count),
                sent_blocks: 2.min(descriptor.block_count),
                total_blocks: descriptor.block_count,
                storage_chunks_read: 2,
                queue_free: 0,
                queue_depth: 0,
                final_progress: None,
            }),
            realtime: Some(RealtimeJobReport {
                prepare_id: descriptor.prepare_id,
                state: RealtimeJobState::Admitted,
                admitted_blocks: 2.min(descriptor.block_count),
                completed_blocks: 0,
                total_blocks: descriptor.block_count,
                queue_depth: 0,
                admitted_progress: Some((StreamTick(10), Digest([0x81; 32]))),
                completed_progress: None,
                outstanding: true,
            }),
            schedule: Some(authority.schedule.report()),
        }
    }

    fn cancelled_status(authority: &Authority) -> JobStatusReport {
        let mut report = ready_status(authority);
        report.service.as_mut().unwrap().state = ServiceJobState::Cancelled;
        let realtime = report.realtime.as_mut().unwrap();
        realtime.state = RealtimeJobState::Cancelled;
        realtime.admitted_progress = None;
        realtime.outstanding = false;
        report.schedule = None;
        report
    }

    fn response(report: &JobStatusReport) -> Response {
        Response {
            status: StatusCode::Ok,
            body: report.encode().unwrap().to_vec(),
        }
    }

    fn admission(descriptor: &alumina_job::JobDescriptor) -> JobScheduleAdmission {
        JobScheduleAdmission {
            now: DeviceCycle(3_000_000),
            active_config: descriptor.config_digest,
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

    fn prepare_all(coordinator: &mut DistributedScheduleCoordinator) -> Vec<Authority> {
        let mut authorities = Vec::new();
        while let Some(request) = coordinator.next_request().unwrap() {
            match request.request.operation {
                Operation::JobStatus => {
                    assert!(request.request.body.is_empty());
                    coordinator
                        .accept_response(request.device_id, &response(&JobStatusReport::default()))
                        .unwrap();
                }
                Operation::JobPrepare => {
                    let descriptor =
                        alumina_job::JobDescriptor::decode::<2>(&request.request.body).unwrap();
                    let boot_id = coordinator
                        .participant(request.device_id)
                        .unwrap()
                        .control
                        .boot_id();
                    let authority = Authority {
                        device_id: request.device_id,
                        schedule: PreparedJobSchedule::prepare::<2>(boot_id, descriptor).unwrap(),
                        descriptor,
                    };
                    coordinator
                        .accept_response(request.device_id, &response(&ready_status(&authority)))
                        .unwrap();
                    authorities.push(authority);
                }
                operation => panic!("unexpected preparation operation {operation:?}"),
            }
        }
        authorities.sort_unstable_by_key(|authority| authority.device_id);
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Ready);
        authorities
    }

    fn start_inputs<'a>(
        job: &CanonicalGlobalJob2,
        clocks: &'a [DeviceClockModel],
    ) -> Vec<ParticipantStartInput<'a>> {
        job.participants()
            .iter()
            .enumerate()
            .map(|(index, participant)| ParticipantStartInput {
                device_id: participant.device_id(),
                clock: &clocks[index],
                timing: ParticipantStartTiming {
                    maximum_uncertainty_cycles: 2_000,
                    required_sync_tolerance_cycles: 2_000,
                    confirmation_lead_cycles: 400_000,
                    abort_guard_lead_cycles: 200_000,
                    lease_cycles: 3_000_000,
                    commit_id: JobCommitId::new(
                        [0x91 + u8::try_from(index).unwrap(); JOB_COMMIT_ID_BYTES],
                    )
                    .unwrap(),
                },
            })
            .collect()
    }

    fn install_and_confirm(
        coordinator: &mut DistributedScheduleCoordinator,
        authorities: &mut [Authority],
        inputs: &[ParticipantStartInput<'_>],
    ) {
        coordinator
            .bind_start(3_001_000_000, 5_000_000_000, inputs)
            .unwrap();
        while let Some(request) = coordinator.next_request().unwrap() {
            assert_eq!(request.request.operation, Operation::JobCommit);
            let commit = JobCommitRequest::decode(&request.request.body).unwrap();
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            authority
                .schedule
                .install(commit, admission(&authority.descriptor))
                .unwrap();
            coordinator
                .accept_response(request.device_id, &response(&ready_status(authority)))
                .unwrap();
        }
        coordinator
            .begin_confirmation(3_002_000_000, inputs)
            .unwrap();
        while let Some(request) = coordinator.next_request().unwrap() {
            assert_eq!(request.request.operation, Operation::JobConfirm);
            let reference =
                alumina_job::JobScheduleReference::decode(&request.request.body).unwrap();
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            authority
                .schedule
                .confirm(reference, DeviceCycle(4_000_000))
                .unwrap();
            coordinator
                .accept_response(request.device_id, &response(&ready_status(authority)))
                .unwrap();
        }
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Confirmed);
    }

    fn complete_authority(authority: &mut Authority, commit: JobCommitRequest, output_token: u32) {
        assert!(matches!(
            authority.schedule.advance(commit.abort_guard_cycle),
            alumina_job::JobScheduleAction::PrimeHardware { .. }
        ));
        authority
            .schedule
            .mark_primed(commit.abort_guard_cycle)
            .unwrap();
        assert!(matches!(
            authority.schedule.advance(commit.local_start_cycle),
            alumina_job::JobScheduleAction::Start { .. }
        ));
        authority
            .schedule
            .record_start_observation(alumina_job::JobStartObservation {
                source: JobStartObservationSource::SimulatedLatch,
                output_token,
                scheduled_cycle: commit.local_start_cycle,
                earliest_cycle: commit.local_start_cycle,
                latest_cycle: commit.local_start_cycle,
            })
            .unwrap();
        authority
            .schedule
            .complete(DeviceCycle(commit.local_start_cycle.0 + 1))
            .unwrap();
    }

    fn install_confirm_and_complete(
        coordinator: &mut DistributedScheduleCoordinator,
        authorities: &mut [Authority],
        inputs: &[ParticipantStartInput<'_>],
    ) {
        install_and_confirm(coordinator, authorities, inputs);
        for authority in authorities {
            let commit = coordinator.participant_commit(authority.device_id).unwrap();
            complete_authority(authority, commit, 17);
        }
    }

    #[test]
    fn every_install_precedes_the_first_confirmation() {
        let (job, ready, preparations, clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut authorities = prepare_all(&mut coordinator);
        let inputs = start_inputs(&job, &clocks);
        coordinator
            .bind_start(3_001_000_000, 5_000_000_000, &inputs)
            .unwrap();

        let mut installs = 0;
        while let Some(request) = coordinator.next_request().unwrap() {
            assert_eq!(request.request.operation, Operation::JobCommit);
            installs += 1;
            let commit = JobCommitRequest::decode(&request.request.body).unwrap();
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            authority
                .schedule
                .install(commit, admission(&authority.descriptor))
                .unwrap();
            coordinator
                .accept_response(request.device_id, &response(&ready_status(authority)))
                .unwrap();
        }
        assert_eq!(installs, job.participants().len());
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Installed);
        assert_eq!(coordinator.next_request().unwrap(), None);

        assert!(matches!(
            coordinator.begin_confirmation(4_700_000_000, &inputs),
            Err(DistributedScheduleError::ConfirmationClosed)
        ));
        coordinator
            .begin_confirmation(3_002_000_000, &inputs)
            .unwrap();
        let mut confirms = 0;
        while let Some(request) = coordinator.next_request().unwrap() {
            assert_eq!(request.request.operation, Operation::JobConfirm);
            confirms += 1;
            let reference =
                alumina_job::JobScheduleReference::decode(&request.request.body).unwrap();
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            authority
                .schedule
                .confirm(reference, DeviceCycle(4_000_000))
                .unwrap();
            coordinator
                .accept_response(request.device_id, &response(&ready_status(authority)))
                .unwrap();
        }
        assert_eq!(confirms, job.participants().len());
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Confirmed);
        assert!(matches!(
            coordinator.reconcile_start_observations(3_100_000_000, &inputs, 2_000_000),
            Err(DistributedScheduleError::StartObservationUnavailable)
        ));

        let status_round = coordinator.begin_status_round().unwrap();
        assert_eq!(status_round.len(), job.participants().len());
        for request in status_round {
            assert_eq!(request.request.operation, Operation::JobStatus);
            let authority = authorities
                .iter()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            coordinator
                .accept_response(request.device_id, &response(&ready_status(authority)))
                .unwrap();
        }
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Confirmed);
    }

    #[test]
    fn lost_abort_responses_reconcile_before_the_next_participant_mutates() {
        let (job, ready, preparations, clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut authorities = prepare_all(&mut coordinator);
        let inputs = start_inputs(&job, &clocks);
        coordinator
            .bind_start(3_001_000_000, 5_000_000_000, &inputs)
            .unwrap();
        while let Some(request) = coordinator.next_request().unwrap() {
            assert_eq!(request.request.operation, Operation::JobCommit);
            let commit = JobCommitRequest::decode(&request.request.body).unwrap();
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            authority
                .schedule
                .install(commit, admission(&authority.descriptor))
                .unwrap();
            coordinator
                .accept_response(request.device_id, &response(&ready_status(authority)))
                .unwrap();
        }
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Installed);
        coordinator.begin_abort().unwrap();

        let mut lost_aborts = 0;
        let mut reconciled_aborts = 0;
        while let Some(request) = coordinator.next_request().unwrap() {
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            match request.request.operation {
                Operation::JobAbort => {
                    assert_eq!(lost_aborts, reconciled_aborts);
                    let reference =
                        alumina_job::JobScheduleReference::decode(&request.request.body).unwrap();
                    let commit = coordinator.participant_commit(request.device_id).unwrap();
                    authority
                        .schedule
                        .abort(reference, DeviceCycle(commit.confirm_deadline_cycle.0 - 1))
                        .unwrap();
                    assert!(coordinator.abandon_pending(request.device_id).unwrap());
                    lost_aborts += 1;
                }
                Operation::JobStatus => {
                    coordinator
                        .accept_response(request.device_id, &response(&ready_status(authority)))
                        .unwrap();
                    reconciled_aborts += 1;
                }
                operation => panic!("unexpected abort-recovery operation {operation:?}"),
            }
        }
        assert_eq!(lost_aborts, authorities.len());
        assert_eq!(reconciled_aborts, authorities.len());
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Aborted);
    }

    #[test]
    fn bound_stop_aborts_installed_and_cancels_never_installed_participants() {
        let (job, ready, preparations, clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut authorities = prepare_all(&mut coordinator);
        let inputs = start_inputs(&job, &clocks);
        coordinator
            .bind_start(3_001_000_000, 5_000_000_000, &inputs)
            .unwrap();

        let install = coordinator.next_request().unwrap().unwrap();
        assert_eq!(install.request.operation, Operation::JobCommit);
        let commit = JobCommitRequest::decode(&install.request.body).unwrap();
        let installed_device = install.device_id;
        let authority = authorities
            .iter_mut()
            .find(|authority| authority.device_id == installed_device)
            .unwrap();
        authority
            .schedule
            .install(commit, admission(&authority.descriptor))
            .unwrap();
        coordinator
            .accept_response(installed_device, &response(&ready_status(authority)))
            .unwrap();
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Installing);
        coordinator.begin_abort().unwrap();

        let mut cancelled_device = None;
        while let Some(request) = coordinator.next_request().unwrap() {
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            match request.request.operation {
                Operation::JobAbort => {
                    assert_eq!(request.device_id, installed_device);
                    let reference =
                        alumina_job::JobScheduleReference::decode(&request.request.body).unwrap();
                    authority
                        .schedule
                        .abort(reference, DeviceCycle(commit.confirm_deadline_cycle.0 - 1))
                        .unwrap();
                    coordinator
                        .accept_response(request.device_id, &response(&ready_status(authority)))
                        .unwrap();
                }
                Operation::JobCancel => {
                    assert_ne!(request.device_id, installed_device);
                    cancelled_device = Some(request.device_id);
                    coordinator
                        .accept_response(request.device_id, &response(&cancelled_status(authority)))
                        .unwrap();
                }
                operation => panic!("unexpected bound-stop operation {operation:?}"),
            }
        }
        let cancelled_device = cancelled_device.unwrap();
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Aborted);
        assert_eq!(
            coordinator.participant_phase(installed_device),
            Some(ParticipantSchedulePhase::Aborted)
        );
        assert_eq!(
            coordinator.participant_phase(cancelled_device),
            Some(ParticipantSchedulePhase::Cancelled)
        );
        assert!(
            coordinator
                .participant_local_start_cycle(installed_device)
                .is_some()
        );
        assert_eq!(
            coordinator.participant_local_start_cycle(cancelled_device),
            None
        );
    }

    #[test]
    fn lost_abort_requests_reconcile_unchanged_state_then_retry_each_participant() {
        let (job, ready, preparations, clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut authorities = prepare_all(&mut coordinator);
        let inputs = start_inputs(&job, &clocks);
        install_and_confirm(&mut coordinator, &mut authorities, &inputs);
        coordinator.begin_abort().unwrap();

        let mut lost_requests = Vec::new();
        let mut reconciled_requests = Vec::new();
        let mut applied_retries = Vec::new();
        while let Some(request) = coordinator.next_request().unwrap() {
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            match request.request.operation {
                Operation::JobAbort if !lost_requests.contains(&request.device_id) => {
                    assert_eq!(lost_requests.len(), applied_retries.len());
                    assert!(coordinator.abandon_pending(request.device_id).unwrap());
                    lost_requests.push(request.device_id);
                }
                Operation::JobStatus => {
                    assert!(lost_requests.contains(&request.device_id));
                    assert!(!reconciled_requests.contains(&request.device_id));
                    assert!(!applied_retries.contains(&request.device_id));
                    coordinator
                        .accept_response(request.device_id, &response(&ready_status(authority)))
                        .unwrap();
                    assert_eq!(
                        coordinator
                            .participant(request.device_id)
                            .unwrap()
                            .control
                            .phase(),
                        ParticipantSchedulePhase::Confirmed
                    );
                    reconciled_requests.push(request.device_id);
                }
                Operation::JobAbort => {
                    assert!(reconciled_requests.contains(&request.device_id));
                    assert!(!applied_retries.contains(&request.device_id));
                    let reference =
                        alumina_job::JobScheduleReference::decode(&request.request.body).unwrap();
                    let commit = coordinator.participant_commit(request.device_id).unwrap();
                    authority
                        .schedule
                        .abort(reference, DeviceCycle(commit.confirm_deadline_cycle.0 - 1))
                        .unwrap();
                    coordinator
                        .accept_response(request.device_id, &response(&ready_status(authority)))
                        .unwrap();
                    applied_retries.push(request.device_id);
                }
                operation => panic!("unexpected abort-request-recovery operation {operation:?}"),
            }
        }
        assert_eq!(lost_requests.len(), authorities.len());
        assert_eq!(reconciled_requests.len(), authorities.len());
        assert_eq!(applied_retries.len(), authorities.len());
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Aborted);
    }

    #[test]
    fn full_abort_status_outage_retains_facts_until_exact_completion_is_observed() {
        let (job, ready, preparations, clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut authorities = prepare_all(&mut coordinator);
        let inputs = start_inputs(&job, &clocks);
        install_and_confirm(&mut coordinator, &mut authorities, &inputs);
        coordinator.begin_abort().unwrap();

        for completed in 0..authorities.len() {
            let abort = coordinator.next_request().unwrap().unwrap();
            assert_eq!(abort.request.operation, Operation::JobAbort);
            assert!(coordinator.abandon_pending(abort.device_id).unwrap());

            for _ in 0..3 {
                let status = coordinator.next_request().unwrap().unwrap();
                assert_eq!(status.device_id, abort.device_id);
                assert_eq!(status.request.operation, Operation::JobStatus);
                assert!(coordinator.abandon_pending(status.device_id).unwrap());
                assert_eq!(
                    coordinator.participant_phase(status.device_id),
                    Some(ParticipantSchedulePhase::Confirmed)
                );
                assert_eq!(
                    coordinator.phase(),
                    if completed == 0 {
                        DistributedSchedulePhase::Aborting
                    } else {
                        DistributedSchedulePhase::Irrevocable
                    }
                );
            }

            let commit = coordinator.participant_commit(abort.device_id).unwrap();
            let authority = authorities
                .iter_mut()
                .find(|authority| authority.device_id == abort.device_id)
                .unwrap();
            complete_authority(authority, commit, 19);
            let observed = ready_status(authority);
            let status = coordinator.next_request().unwrap().unwrap();
            assert_eq!(status.device_id, abort.device_id);
            assert_eq!(status.request.operation, Operation::JobStatus);
            coordinator
                .accept_response(status.device_id, &response(&observed))
                .unwrap();
            assert_eq!(
                coordinator.phase(),
                if completed + 1 == authorities.len() {
                    DistributedSchedulePhase::Complete
                } else {
                    DistributedSchedulePhase::Irrevocable
                }
            );
        }

        assert_eq!(coordinator.next_request().unwrap(), None);
        assert!(authorities.iter().all(|authority| {
            coordinator.participant_phase(authority.device_id)
                == Some(ParticipantSchedulePhase::Complete)
        }));
    }

    #[test]
    fn partial_abort_then_remote_completion_is_an_exact_terminal_split() {
        let (job, ready, preparations, clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut authorities = prepare_all(&mut coordinator);
        let inputs = start_inputs(&job, &clocks);
        install_and_confirm(&mut coordinator, &mut authorities, &inputs);
        coordinator.begin_abort().unwrap();

        let stopped_request = coordinator.next_request().unwrap().unwrap();
        assert_eq!(stopped_request.request.operation, Operation::JobAbort);
        let stopped_reference =
            alumina_job::JobScheduleReference::decode(&stopped_request.request.body).unwrap();
        let stopped_commit = coordinator
            .participant_commit(stopped_request.device_id)
            .unwrap();
        let stopped_authority = authorities
            .iter_mut()
            .find(|authority| authority.device_id == stopped_request.device_id)
            .unwrap();
        stopped_authority
            .schedule
            .abort(
                stopped_reference,
                DeviceCycle(stopped_commit.confirm_deadline_cycle.0 - 1),
            )
            .unwrap();
        coordinator
            .accept_response(
                stopped_request.device_id,
                &response(&ready_status(stopped_authority)),
            )
            .unwrap();
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Aborting);

        let missed_request = coordinator.next_request().unwrap().unwrap();
        assert_eq!(missed_request.request.operation, Operation::JobAbort);
        assert_ne!(missed_request.device_id, stopped_request.device_id);
        assert!(
            coordinator
                .abandon_pending(missed_request.device_id)
                .unwrap()
        );
        let missed_commit = coordinator
            .participant_commit(missed_request.device_id)
            .unwrap();
        let missed_authority = authorities
            .iter_mut()
            .find(|authority| authority.device_id == missed_request.device_id)
            .unwrap();
        complete_authority(missed_authority, missed_commit, 19);

        let status_request = coordinator.next_request().unwrap().unwrap();
        assert_eq!(status_request.device_id, missed_request.device_id);
        assert_eq!(status_request.request.operation, Operation::JobStatus);
        coordinator
            .accept_response(
                status_request.device_id,
                &response(&ready_status(missed_authority)),
            )
            .unwrap();
        assert_eq!(
            coordinator.phase(),
            DistributedSchedulePhase::SplitAfterAbort
        );
        assert_eq!(coordinator.next_request().unwrap(), None);
        assert_eq!(
            coordinator.participant_phase(stopped_request.device_id),
            Some(ParticipantSchedulePhase::Aborted)
        );
        assert_eq!(
            coordinator.participant_phase(missed_request.device_id),
            Some(ParticipantSchedulePhase::Complete)
        );
    }

    #[test]
    fn passive_initial_status_reattaches_only_the_exact_completed_attempt() {
        let (job, ready, preparations, clocks) = fixture();
        let mut original =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut authorities = prepare_all(&mut original);
        let inputs = start_inputs(&job, &clocks);
        install_confirm_and_complete(&mut original, &mut authorities, &inputs);

        let mut reattached =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut observed = 0;
        while let Some(request) = reattached.next_request().unwrap() {
            assert_eq!(request.request.operation, Operation::JobStatus);
            assert!(request.request.body.is_empty());
            let authority = authorities
                .iter()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            reattached
                .accept_response(request.device_id, &response(&ready_status(authority)))
                .unwrap();
            observed += 1;
        }
        assert_eq!(observed, authorities.len());
        assert_eq!(reattached.phase(), DistributedSchedulePhase::Complete);
        assert_eq!(reattached.target_ui_ns(), None);
        for authority in &authorities {
            assert_eq!(reattached.participant_commit(authority.device_id), None);
            assert_eq!(
                reattached.participant_local_start_cycle(authority.device_id),
                Some(authority.schedule.report().local_start_cycle)
            );
        }

        let mut divergent =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let first = divergent.next_request().unwrap().unwrap();
        let first_authority = authorities
            .iter()
            .find(|authority| authority.device_id == first.device_id)
            .unwrap();
        divergent
            .accept_response(first.device_id, &response(&ready_status(first_authority)))
            .unwrap();
        let second = divergent.next_request().unwrap().unwrap();
        assert_eq!(second.request.operation, Operation::JobStatus);
        divergent
            .accept_response(second.device_id, &response(&JobStatusReport::default()))
            .unwrap();
        assert_eq!(divergent.phase(), DistributedSchedulePhase::Faulted);
        assert_eq!(divergent.next_request().unwrap(), None);
    }

    #[test]
    fn lost_install_response_reconciles_without_duplicate_mutation() {
        let (job, ready, preparations, clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let mut authorities = prepare_all(&mut coordinator);
        coordinator
            .bind_start(3_001_000_000, 5_000_000_000, &start_inputs(&job, &clocks))
            .unwrap();

        let install = coordinator.next_request().unwrap().unwrap();
        let commit = JobCommitRequest::decode(&install.request.body).unwrap();
        let authority = authorities
            .iter_mut()
            .find(|authority| authority.device_id == install.device_id)
            .unwrap();
        authority
            .schedule
            .install(commit, admission(&authority.descriptor))
            .unwrap();
        assert!(coordinator.abandon_pending(install.device_id).unwrap());
        let inspect = coordinator.next_request().unwrap().unwrap();
        assert_eq!(inspect.device_id, install.device_id);
        assert_eq!(inspect.request.operation, Operation::JobStatus);
        coordinator
            .accept_response(inspect.device_id, &response(&ready_status(authority)))
            .unwrap();
        assert_eq!(
            coordinator.participant_phase(install.device_id),
            Some(ParticipantSchedulePhase::Installed)
        );
    }

    #[test]
    fn short_lease_and_changed_boot_fail_before_any_commit_is_bound() {
        let (job, ready, preparations, mut clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        prepare_all(&mut coordinator);
        let mut inputs = start_inputs(&job, &clocks);
        inputs[0].timing.lease_cycles = 1;
        assert!(matches!(
            coordinator.bind_start(3_001_000_000, 5_000_000_000, &inputs),
            Err(DistributedScheduleError::LeaseTooShort { .. })
        ));
        assert!(job.participants().iter().all(|participant| {
            coordinator
                .participant_commit(participant.device_id())
                .is_none()
        }));

        clocks[0].reset().unwrap();
        let inputs = start_inputs(&job, &clocks);
        assert!(matches!(
            coordinator.bind_start(3_001_000_000, 5_000_000_000, &inputs),
            Err(DistributedScheduleError::ParticipantSet)
        ));
    }

    #[test]
    fn deadline_intervals_close_confirmation_before_abort_authority() {
        let (job, ready, preparations, clocks) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        prepare_all(&mut coordinator);
        let inputs = start_inputs(&job, &clocks);
        coordinator
            .bind_start(3_001_000_000, 5_000_000_000, &inputs)
            .unwrap();

        assert!(matches!(
            coordinator.deadline_window(4_500_000_000, &inputs),
            Ok(DistributedDeadlineWindow::ConfirmationOpen)
        ));
        assert!(matches!(
            coordinator.deadline_window(4_700_000_000, &inputs),
            Ok(DistributedDeadlineWindow::AbortOnly)
        ));
        assert!(matches!(
            coordinator.deadline_window(4_900_000_000, &inputs),
            Ok(DistributedDeadlineWindow::PointOfNoReturn)
        ));
    }

    #[test]
    fn precommit_cancellation_reconciles_every_prepared_actor() {
        let (job, ready, preparations, _) = fixture();
        let mut coordinator =
            DistributedScheduleCoordinator::after_cache(&job, &ready, &preparations).unwrap();
        let authorities = prepare_all(&mut coordinator);
        coordinator.begin_cancel().unwrap();

        let mut cancelled = 0;
        while let Some(request) = coordinator.next_request().unwrap() {
            assert_eq!(request.request.operation, Operation::JobCancel);
            let authority = authorities
                .iter()
                .find(|authority| authority.device_id == request.device_id)
                .unwrap();
            coordinator
                .accept_response(request.device_id, &response(&cancelled_status(authority)))
                .unwrap();
            cancelled += 1;
        }
        assert_eq!(cancelled, job.participants().len());
        assert_eq!(coordinator.phase(), DistributedSchedulePhase::Cancelled);
    }
}
