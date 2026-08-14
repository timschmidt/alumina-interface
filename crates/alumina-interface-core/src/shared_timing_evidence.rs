//! Canonical evidence for jointly retimed multi-MCU cached motion streams.

use std::error::Error as StdError;
use std::fmt;

use alumina_protocol::{DeviceId, Digest};
use alumina_storage::sha256;

use crate::motion_schedule::{
    CanonicalScheduledProgram2, CertifiedJerkSchedule2, SharedTimerCandidateOutcome2,
    SharedTimerLatticeSchedule2,
};
use crate::partition::CanonicalMachinePartition2;
use crate::schedule_evidence::{
    ScheduleEvidenceError, TranscriptEncoder, build_canonical_derivation_digests,
    push_execution_segment, push_motion_error, push_preflight,
};

const EVIDENCE_MAGIC: [u8; 8] = *b"ALMSYN01";
const EVIDENCE_VERSION: u16 = 1;
const TRANSCRIPT_MAGIC: [u8; 8] = *b"ALMSRT01";
const TRANSCRIPT_VERSION: u16 = 1;
const EXACT_REAL_FORMAT_VERSION: u16 = 1;
const AXES: u16 = 2;
const CERTIFIED_COMMON_EVENT_GRID: u32 = 1 << 0;
const CERTIFIED_COMPLETE_CANDIDATE_REPLAY: u32 = 1 << 1;
const CERTIFIED_SHARED_MINIMALITY: u32 = 1 << 2;
const CERTIFIED_PARTITION_REPLAY: u32 = 1 << 3;
const CERTIFICATION_FLAGS: u32 = CERTIFIED_COMMON_EVENT_GRID
    | CERTIFIED_COMPLETE_CANDIDATE_REPLAY
    | CERTIFIED_SHARED_MINIMALITY
    | CERTIFIED_PARTITION_REPLAY;
const EVIDENCE_WIRE_BYTES: usize = 104;

/// Result type for canonical shared-retiming evidence.
pub type SharedTimingEvidenceResult<T> = Result<T, SharedTimingEvidenceError>;

/// Upstream exact derivation plus final immutable partition for one MCU.
#[derive(Clone, Copy, Debug)]
pub struct SharedTimingEvidenceParticipant2<'a> {
    device_id: DeviceId,
    schedule: &'a CertifiedJerkSchedule2,
    program: &'a CanonicalScheduledProgram2,
    partition: &'a CanonicalMachinePartition2,
}

impl<'a> SharedTimingEvidenceParticipant2<'a> {
    /// Bind one stable MCU identity to its planner, lowering, and cache artifact.
    pub const fn new(
        device_id: DeviceId,
        schedule: &'a CertifiedJerkSchedule2,
        program: &'a CanonicalScheduledProgram2,
        partition: &'a CanonicalMachinePartition2,
    ) -> Self {
        Self {
            device_id,
            schedule,
            program,
            partition,
        }
    }

    /// Stable physical MCU identity.
    pub const fn device_id(self) -> DeviceId {
        self.device_id
    }
}

/// Compact outer record committing the full streamed exact retiming transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalSharedTimingEvidence1 {
    encoded: Vec<u8>,
    digest: Digest,
    transcript_digest: Digest,
    transcript_byte_len: u64,
    selected_factor_numerator: u32,
    selected_factor_denominator: u32,
    maximum_factor_numerator: u32,
    candidate_rounds: u32,
    participant_replays: u64,
    timer_ticks_per_second: u64,
    output_quantum_cycles: u32,
    terminal_tick: u64,
    participant_count: u32,
}

impl CanonicalSharedTimingEvidence1 {
    /// Complete canonical `ALMSYN01` outer record.
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    /// SHA-256 identity of the complete outer record.
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// SHA-256 identity of the full streamed `ALMSRT01` transcript.
    pub const fn transcript_digest(&self) -> Digest {
        self.transcript_digest
    }

    /// Exact byte length of the full streamed transcript.
    pub const fn transcript_byte_len(&self) -> u64 {
        self.transcript_byte_len
    }

    /// Selected numerator on the common exact factor lattice.
    pub const fn selected_factor_numerator(&self) -> u32 {
        self.selected_factor_numerator
    }

    /// Denominator of the common exact factor lattice.
    pub const fn selected_factor_denominator(&self) -> u32 {
        self.selected_factor_denominator
    }

    /// Inclusive caller-owned search ceiling numerator.
    pub const fn maximum_factor_numerator(&self) -> u32 {
        self.maximum_factor_numerator
    }

    /// Number of complete all-participant candidate rounds.
    pub const fn candidate_rounds(&self) -> u32 {
        self.candidate_rounds
    }

    /// Number of production-preflight participant replays.
    pub const fn participant_replays(&self) -> u64 {
        self.participant_replays
    }

    /// Shared exact local timer frequency.
    pub const fn timer_ticks_per_second(&self) -> u64 {
        self.timer_ticks_per_second
    }

    /// Shared exact output quantum.
    pub const fn output_quantum_cycles(&self) -> u32 {
        self.output_quantum_cycles
    }

    /// Common final stream-relative tick.
    pub const fn terminal_tick(&self) -> u64 {
        self.terminal_tick
    }

    /// Canonically ordered MCU count.
    pub const fn participant_count(&self) -> u32 {
        self.participant_count
    }
}

/// Build a deterministic shared-retiming transcript only after every final
/// partition replays against its exact local derivation and selected stream.
pub fn build_shared_timing_evidence(
    shared: &SharedTimerLatticeSchedule2,
    mut participants: Vec<SharedTimingEvidenceParticipant2<'_>>,
) -> SharedTimingEvidenceResult<CanonicalSharedTimingEvidence1> {
    participants.sort_unstable_by_key(|participant| participant.device_id);
    validate_participants(shared, &participants)?;
    validate_search_trace(shared)?;

    let mut transcript = TranscriptEncoder::new();
    transcript.raw(&TRANSCRIPT_MAGIC)?;
    transcript.u16(TRANSCRIPT_VERSION)?;
    transcript.u16(EXACT_REAL_FORMAT_VERSION)?;
    transcript.u16(AXES)?;
    transcript.u16(0)?;
    transcript.u32(CERTIFICATION_FLAGS)?;
    transcript.u32(shared.selected_factor_numerator())?;
    transcript.u32(shared.selected_factor_denominator())?;
    transcript.u32(shared.maximum_factor_numerator())?;
    transcript.rational(&shared.selected_factor())?;
    transcript.u32(shared.candidate_rounds())?;
    transcript.u64(shared.participant_replays())?;
    transcript.u64(shared.timer_ticks_per_second())?;
    transcript.u32(shared.output_quantum_cycles())?;
    transcript.u64(shared.terminal_tick().get())?;
    transcript.real(shared.ideal_total_time_seconds())?;
    transcript.real(shared.scheduled_total_time_seconds())?;
    transcript.usize(participants.len())?;

    transcript.usize(shared.candidate_reports().len())?;
    for report in shared.candidate_reports() {
        transcript.u32(report.factor_numerator())?;
        transcript.usize(report.outcomes().len())?;
        for outcome in report.outcomes() {
            push_candidate_outcome(&mut transcript, *outcome)?;
        }
    }

    for (input, retimed) in participants.iter().zip(shared.participants()) {
        let derivation = build_canonical_derivation_digests(input.schedule, input.program)?;
        transcript.raw(&input.device_id.0)?;
        transcript.raw(&retimed.configuration_digest().0)?;
        transcript.raw(&retimed.capability_digest().0)?;
        transcript.raw(&derivation.source_digest.0)?;
        transcript.raw(&derivation.metric_path_digest.0)?;
        transcript.raw(&derivation.source_approximation_digest.0)?;
        transcript.raw(&derivation.planner_transcript_digest.0)?;
        transcript.u64(derivation.planner_transcript_byte_len)?;
        transcript.raw(&derivation.lowering_transcript_digest.0)?;
        transcript.u64(derivation.lowering_transcript_byte_len)?;

        let policy = input.partition.policy();
        let publication = input.partition.publication();
        let terminal = input.partition.terminal_progress();
        transcript.raw(&policy.stream_id().0)?;
        transcript.raw(&publication.object.content.digest.0)?;
        transcript.raw(&publication.manifest.digest.0)?;
        transcript.u64(publication.object.byte_len)?;
        transcript.u32(input.partition.block_count())?;
        transcript.u64(input.partition.local_timer_hz())?;
        for coordinate in input.partition.initial_position() {
            transcript.i64(coordinate)?;
        }
        for coordinate in input.partition.final_position() {
            transcript.i64(coordinate)?;
        }
        transcript.raw(&terminal.block_digest.0)?;
        transcript.u64(terminal.end_tick.0)?;

        push_candidate_outcome(&mut transcript, retimed.unit_factor_outcome())?;
        push_optional_candidate_outcome(&mut transcript, retimed.predecessor_outcome())?;
        transcript.real(retimed.ideal_total_time_seconds())?;
        transcript.real(retimed.scheduled_total_time_seconds())?;
        transcript.real(retimed.maximum_cumulative_delay_seconds())?;
        transcript.real(retimed.maximum_segment_extension_seconds())?;
        transcript.real(retimed.maximum_output_grid_padding_seconds())?;
        transcript.usize(retimed.ticks().len())?;
        for tick in retimed.ticks() {
            transcript.u64(tick.get())?;
        }
        transcript.usize(retimed.segments().len())?;
        for segment in retimed.segments() {
            push_execution_segment(&mut transcript, segment)?;
        }
        push_preflight(&mut transcript, retimed.executor_preflight())?;
    }
    let (transcript_digest, transcript_byte_len) = transcript.finish();

    let participant_count = u32::try_from(participants.len())
        .map_err(|_| SharedTimingEvidenceError::CounterOverflow)?;
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(EVIDENCE_WIRE_BYTES)
        .map_err(|_| SharedTimingEvidenceError::AllocationOverflow)?;
    encoded.extend_from_slice(&EVIDENCE_MAGIC);
    push_u16(&mut encoded, EVIDENCE_VERSION);
    push_u16(&mut encoded, AXES);
    push_u32(&mut encoded, CERTIFICATION_FLAGS);
    encoded.extend_from_slice(&transcript_digest.0);
    push_u64(&mut encoded, transcript_byte_len);
    push_u32(&mut encoded, shared.selected_factor_numerator());
    push_u32(&mut encoded, shared.selected_factor_denominator());
    push_u32(&mut encoded, shared.maximum_factor_numerator());
    push_u32(&mut encoded, shared.candidate_rounds());
    push_u64(&mut encoded, shared.participant_replays());
    push_u64(&mut encoded, shared.timer_ticks_per_second());
    push_u64(&mut encoded, shared.terminal_tick().get());
    push_u32(&mut encoded, shared.output_quantum_cycles());
    push_u32(&mut encoded, participant_count);
    if encoded.len() != EVIDENCE_WIRE_BYTES {
        return Err(SharedTimingEvidenceError::Encoding);
    }
    let digest = sha256(&encoded).digest;
    Ok(CanonicalSharedTimingEvidence1 {
        encoded,
        digest,
        transcript_digest,
        transcript_byte_len,
        selected_factor_numerator: shared.selected_factor_numerator(),
        selected_factor_denominator: shared.selected_factor_denominator(),
        maximum_factor_numerator: shared.maximum_factor_numerator(),
        candidate_rounds: shared.candidate_rounds(),
        participant_replays: shared.participant_replays(),
        timer_ticks_per_second: shared.timer_ticks_per_second(),
        output_quantum_cycles: shared.output_quantum_cycles(),
        terminal_tick: shared.terminal_tick().get(),
        participant_count,
    })
}

/// Rebuild all exact derivation, search, stream, and partition facts and require
/// byte-for-byte shared evidence identity.
pub fn replay_shared_timing_evidence(
    evidence: &CanonicalSharedTimingEvidence1,
    shared: &SharedTimerLatticeSchedule2,
    participants: Vec<SharedTimingEvidenceParticipant2<'_>>,
) -> SharedTimingEvidenceResult<()> {
    let rebuilt = build_shared_timing_evidence(shared, participants)?;
    if evidence != &rebuilt {
        return Err(SharedTimingEvidenceError::ReplayMismatch);
    }
    Ok(())
}

/// Validate the compact outer record's canonical framing and declared digest.
pub fn verify_shared_timing_evidence_bytes(
    encoded: &[u8],
    expected_digest: Digest,
) -> SharedTimingEvidenceResult<()> {
    if encoded.len() != EVIDENCE_WIRE_BYTES
        || encoded.get(..8) != Some(EVIDENCE_MAGIC.as_slice())
        || encoded.get(8..10) != Some(EVIDENCE_VERSION.to_le_bytes().as_slice())
        || encoded.get(10..12) != Some(AXES.to_le_bytes().as_slice())
        || encoded.get(12..16) != Some(CERTIFICATION_FLAGS.to_le_bytes().as_slice())
    {
        return Err(SharedTimingEvidenceError::Encoding);
    }
    if sha256(encoded).digest != expected_digest {
        return Err(SharedTimingEvidenceError::DigestMismatch);
    }
    Ok(())
}

fn validate_participants(
    shared: &SharedTimerLatticeSchedule2,
    participants: &[SharedTimingEvidenceParticipant2<'_>],
) -> SharedTimingEvidenceResult<()> {
    if participants.is_empty() || participants.len() != shared.participants().len() {
        return Err(SharedTimingEvidenceError::ParticipantSetMismatch);
    }
    for pair in participants.windows(2) {
        if pair[0].device_id == pair[1].device_id {
            return Err(SharedTimingEvidenceError::ParticipantSetMismatch);
        }
    }
    for (input, retimed) in participants.iter().zip(shared.participants()) {
        if input.device_id != retimed.device_id()
            || input.program.configuration_digest() != retimed.configuration_digest()
            || input.program.capability_digest() != retimed.capability_digest()
            || input.program.timer_ticks_per_second() != retimed.timer_ticks_per_second()
            || input.program.output_quantum_cycles() != retimed.output_quantum_cycles()
            || input.partition.policy().config_digest() != retimed.configuration_digest()
            || input.partition.policy().capability_digest() != retimed.capability_digest()
        {
            return Err(SharedTimingEvidenceError::IdentityMismatch {
                device_id: input.device_id,
            });
        }
        let first =
            input
                .program
                .points()
                .first()
                .ok_or(SharedTimingEvidenceError::TerminalMismatch {
                    device_id: input.device_id,
                })?;
        let last =
            input
                .program
                .points()
                .last()
                .ok_or(SharedTimingEvidenceError::TerminalMismatch {
                    device_id: input.device_id,
                })?;
        let initial_position = [first.steps()[0].get(), first.steps()[1].get()];
        let final_position = [last.steps()[0].get(), last.steps()[1].get()];
        let preflight = retimed.executor_preflight();
        if input.partition.initial_position() != initial_position
            || input.partition.final_position() != final_position
            || input.partition.local_timer_hz() != shared.timer_ticks_per_second()
            || input.partition.terminal_progress().end_tick != preflight.end_tick
            || preflight.end_tick.0 != shared.terminal_tick().get()
            || preflight.position != final_position
            || usize::try_from(preflight.segment_count).ok() != Some(retimed.segments().len())
        {
            return Err(SharedTimingEvidenceError::TerminalMismatch {
                device_id: input.device_id,
            });
        }
    }
    Ok(())
}

fn validate_search_trace(shared: &SharedTimerLatticeSchedule2) -> SharedTimingEvidenceResult<()> {
    let reports = shared.candidate_reports();
    let participant_count = shared.participants().len();
    if usize::try_from(shared.candidate_rounds()).ok() != Some(reports.len())
        || shared.participant_replays()
            != u64::from(shared.candidate_rounds())
                .checked_mul(
                    u64::try_from(participant_count)
                        .map_err(|_| SharedTimingEvidenceError::CounterOverflow)?,
                )
                .ok_or(SharedTimingEvidenceError::CounterOverflow)?
        || reports
            .iter()
            .any(|report| report.outcomes().len() != participant_count)
    {
        return Err(SharedTimingEvidenceError::SearchTraceMismatch);
    }
    for report in reports {
        if report.factor_numerator() < shared.selected_factor_denominator()
            || report.factor_numerator() > shared.maximum_factor_numerator()
            || report.outcomes().iter().any(|outcome| {
                outcome
                    .rejection()
                    .is_some_and(|error| !error.is_time_dilation_candidate())
            })
        {
            return Err(SharedTimingEvidenceError::SearchTraceMismatch);
        }
    }

    let denominator = shared.selected_factor_denominator();
    let selected = shared.selected_factor_numerator();
    if selected == denominator {
        if reports.len() != 1
            || reports[0].factor_numerator() != denominator
            || !all_accepted(reports[0].outcomes())
        {
            return Err(SharedTimingEvidenceError::SearchTraceMismatch);
        }
        for (index, participant) in shared.participants().iter().enumerate() {
            if participant.unit_factor_outcome() != reports[0].outcomes()[index]
                || participant.predecessor_outcome().is_some()
                || participant.executor_preflight()
                    != accepted_preflight(reports[0].outcomes()[index])?
            {
                return Err(SharedTimingEvidenceError::SearchTraceMismatch);
            }
        }
        return Ok(());
    }

    if reports.len() < 4
        || reports[0].factor_numerator() != denominator
        || all_accepted(reports[0].outcomes())
        || reports[1].factor_numerator() != shared.maximum_factor_numerator()
        || !all_accepted(reports[1].outcomes())
    {
        return Err(SharedTimingEvidenceError::SearchTraceMismatch);
    }
    let mut rejected = denominator;
    let mut admitted = shared.maximum_factor_numerator();
    for report in &reports[2..reports.len() - 2] {
        let expected = rejected + (admitted - rejected) / 2;
        if report.factor_numerator() != expected {
            return Err(SharedTimingEvidenceError::SearchTraceMismatch);
        }
        if all_accepted(report.outcomes()) {
            admitted = expected;
        } else {
            rejected = expected;
        }
    }
    let selected_report = &reports[reports.len() - 2];
    let predecessor_report = &reports[reports.len() - 1];
    if admitted != selected
        || selected_report.factor_numerator() != selected
        || !all_accepted(selected_report.outcomes())
        || predecessor_report.factor_numerator() != selected - 1
        || all_accepted(predecessor_report.outcomes())
    {
        return Err(SharedTimingEvidenceError::SearchTraceMismatch);
    }
    for (index, participant) in shared.participants().iter().enumerate() {
        if participant.unit_factor_outcome() != reports[0].outcomes()[index]
            || participant.predecessor_outcome() != Some(predecessor_report.outcomes()[index])
            || participant.executor_preflight()
                != accepted_preflight(selected_report.outcomes()[index])?
        {
            return Err(SharedTimingEvidenceError::SearchTraceMismatch);
        }
    }
    Ok(())
}

fn all_accepted(outcomes: &[SharedTimerCandidateOutcome2]) -> bool {
    outcomes
        .iter()
        .all(|outcome| matches!(outcome, SharedTimerCandidateOutcome2::Accepted(_)))
}

fn accepted_preflight(
    outcome: SharedTimerCandidateOutcome2,
) -> SharedTimingEvidenceResult<alumina_motion::StepperPreflightSummary<2>> {
    match outcome {
        SharedTimerCandidateOutcome2::Accepted(preflight) => Ok(preflight),
        SharedTimerCandidateOutcome2::Rejected(_) => {
            Err(SharedTimingEvidenceError::SearchTraceMismatch)
        }
    }
}

fn push_candidate_outcome(
    encoded: &mut TranscriptEncoder,
    outcome: SharedTimerCandidateOutcome2,
) -> Result<(), ScheduleEvidenceError> {
    match outcome {
        SharedTimerCandidateOutcome2::Accepted(preflight) => {
            encoded.u8(1)?;
            push_preflight(encoded, preflight)
        }
        SharedTimerCandidateOutcome2::Rejected(error) => {
            encoded.u8(0)?;
            push_motion_error(encoded, error)
        }
    }
}

fn push_optional_candidate_outcome(
    encoded: &mut TranscriptEncoder,
    outcome: Option<SharedTimerCandidateOutcome2>,
) -> Result<(), ScheduleEvidenceError> {
    match outcome {
        Some(outcome) => {
            encoded.u8(1)?;
            push_candidate_outcome(encoded, outcome)
        }
        None => encoded.u8(0),
    }
}

fn push_u16(encoded: &mut Vec<u8>, value: u16) {
    encoded.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(encoded: &mut Vec<u8>, value: u32) {
    encoded.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(encoded: &mut Vec<u8>, value: u64) {
    encoded.extend_from_slice(&value.to_le_bytes());
}

/// Failure to construct or replay canonical shared-retiming evidence.
#[derive(Debug)]
pub enum SharedTimingEvidenceError {
    /// Stable identities did not form the same canonical participant set.
    ParticipantSetMismatch,
    /// Program, retiming, and partition identity facts diverged.
    IdentityMismatch {
        /// Stable physical identity of the mismatched participant.
        device_id: DeviceId,
    },
    /// Selected stream and independently replayed partition terminal facts diverged.
    TerminalMismatch {
        /// Stable physical identity of the mismatched participant.
        device_id: DeviceId,
    },
    /// Candidate order, outcomes, binary search, or boundary replays diverged.
    SearchTraceMismatch,
    /// One upstream exact planner/lowering transcript failed.
    Derivation(ScheduleEvidenceError),
    /// Canonical evidence allocation could not be represented.
    AllocationOverflow,
    /// A canonical counter did not fit its wire representation.
    CounterOverflow,
    /// Compact evidence framing was not canonical.
    Encoding,
    /// Rebuilt evidence differed from the supplied record.
    ReplayMismatch,
    /// Supplied evidence bytes differed from their declared SHA-256.
    DigestMismatch,
}

impl fmt::Display for SharedTimingEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParticipantSetMismatch => {
                formatter.write_str("shared timing participant set is not canonical")
            }
            Self::IdentityMismatch { device_id } => write!(
                formatter,
                "shared timing identity facts diverged for participant {device_id:?}"
            ),
            Self::TerminalMismatch { device_id } => write!(
                formatter,
                "shared timing terminal facts diverged for participant {device_id:?}"
            ),
            Self::SearchTraceMismatch => {
                formatter.write_str("shared timer candidate search trace is inconsistent")
            }
            Self::Derivation(source) => {
                write!(
                    formatter,
                    "upstream exact derivation evidence failed: {source}"
                )
            }
            Self::AllocationOverflow => {
                formatter.write_str("shared timing evidence allocation failed")
            }
            Self::CounterOverflow => {
                formatter.write_str("shared timing evidence counter overflowed")
            }
            Self::Encoding => formatter.write_str("shared timing evidence framing is invalid"),
            Self::ReplayMismatch => formatter.write_str("shared timing evidence replay differed"),
            Self::DigestMismatch => formatter.write_str("shared timing evidence digest differed"),
        }
    }
}

impl StdError for SharedTimingEvidenceError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Derivation(source) => Some(source),
            _ => None,
        }
    }
}

impl From<ScheduleEvidenceError> for SharedTimingEvidenceError {
    fn from(value: ScheduleEvidenceError) -> Self {
        Self::Derivation(value)
    }
}
