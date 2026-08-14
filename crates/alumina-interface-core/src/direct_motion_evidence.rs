//! Replayable exact evidence for direct finite-difference cached jobs.

use std::error::Error as StdError;
use std::fmt;

use alumina_machine_ir::{ExecutionKind, FiniteDifferenceAxis, FiniteDifferenceSegment};
use alumina_protocol::Digest;
use alumina_storage::sha256;

use crate::direct_motion::{
    CanonicalDirectFiniteDifferenceProgram2, DirectCoefficientProjection2,
    DirectFiniteDifferencePolicy2,
};
use crate::motion_schedule::CertifiedJerkSchedule2;
use crate::partition::CanonicalMachinePartition2;
use crate::schedule_evidence::{
    ScheduleEvidenceError, TranscriptEncoder, build_canonical_planner_digests,
};

const EVIDENCE_MAGIC: [u8; 8] = *b"ALMDFE01";
const EVIDENCE_VERSION: u16 = 1;
const TRANSCRIPT_MAGIC: [u8; 8] = *b"ALMDFT01";
const TRANSCRIPT_VERSION: u16 = 1;
const EXACT_REAL_FORMAT_VERSION: u16 = 1;
const AXES: u16 = 2;
const EVIDENCE_BYTES: usize = 344;
const CERTIFIED_GRID_REPLAY: u32 = 1 << 0;
const CERTIFIED_COEFFICIENT_INTERVALS: u32 = 1 << 1;
const CERTIFIED_PROPAGATED_ERROR: u32 = 1 << 2;
const CERTIFIED_ELECTRICAL_PREFLIGHT: u32 = 1 << 3;
const CERTIFIED_PARTITION_REPLAY: u32 = 1 << 4;
const CERTIFICATION_FLAGS: u32 = CERTIFIED_GRID_REPLAY
    | CERTIFIED_COEFFICIENT_INTERVALS
    | CERTIFIED_PROPAGATED_ERROR
    | CERTIFIED_ELECTRICAL_PREFLIGHT
    | CERTIFIED_PARTITION_REPLAY;

/// Result type for canonical direct-motion evidence.
pub type DirectMotionEvidenceResult<T> = Result<T, DirectMotionEvidenceError>;

/// Compact outer record committing a bounded streamed exact transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalDirectMotionEvidence1 {
    encoded: Vec<u8>,
    digest: Digest,
    transcript_digest: Digest,
    transcript_byte_len: u64,
    source_digest: Digest,
    metric_path_digest: Digest,
    planner_transcript_digest: Digest,
    record_count: u32,
    update_count: u64,
}

impl CanonicalDirectMotionEvidence1 {
    /// Canonical `ALMDFE01` outer bytes.
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    /// SHA-256 identity of the outer record.
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// SHA-256 identity of the streamed `ALMDFT01` transcript.
    pub const fn transcript_digest(&self) -> Digest {
        self.transcript_digest
    }

    /// Exact transcript byte count covered by its digest.
    pub const fn transcript_byte_len(&self) -> u64 {
        self.transcript_byte_len
    }

    /// Retained exact source-path identity.
    pub const fn source_digest(&self) -> Digest {
        self.source_digest
    }

    /// Exact metric path presented to Hyperpath.
    pub const fn metric_path_digest(&self) -> Digest {
        self.metric_path_digest
    }

    /// Exact Hyperpath/Hypersolve planner transcript identity.
    pub const fn planner_transcript_digest(&self) -> Digest {
        self.planner_transcript_digest
    }

    /// Number of Q31.32 records committed by the transcript.
    pub const fn record_count(&self) -> u32 {
        self.record_count
    }

    /// Number of dense recurrence updates committed by the transcript.
    pub const fn update_count(&self) -> u64 {
        self.update_count
    }
}

/// Build exact direct-motion evidence after independently checking every
/// schedule, program, preflight, and immutable-partition boundary.
pub fn build_direct_motion_evidence(
    schedule: &CertifiedJerkSchedule2,
    program: &CanonicalDirectFiniteDifferenceProgram2,
    partition: &CanonicalMachinePartition2,
) -> DirectMotionEvidenceResult<CanonicalDirectMotionEvidence1> {
    validate_inputs(schedule, program, partition)?;
    let planner = build_canonical_planner_digests(schedule)?;
    let evidence = program.evidence();
    let preflight = program.executor_preflight();
    let publication = partition.publication();
    let terminal = partition.terminal_progress();

    let mut transcript = TranscriptEncoder::new();
    transcript.raw(&TRANSCRIPT_MAGIC)?;
    transcript.u16(TRANSCRIPT_VERSION)?;
    transcript.u16(EXACT_REAL_FORMAT_VERSION)?;
    transcript.u16(AXES)?;
    transcript.u16(0)?;
    transcript.u32(CERTIFICATION_FLAGS)?;
    transcript.raw(&program.configuration_digest().0)?;
    transcript.raw(&program.capability_digest().0)?;
    transcript.raw(&planner.source_digest.0)?;
    transcript.raw(&planner.metric_path_digest.0)?;
    transcript.raw(&planner.source_approximation_digest.0)?;
    transcript.raw(&planner.planner_transcript_digest.0)?;
    transcript.u64(planner.planner_transcript_byte_len)?;
    transcript.u64(program.timer_ticks_per_second())?;
    transcript.u32(program.output_quantum_cycles())?;

    let budget = program.resolution_budget();
    transcript.rational(budget.requested_total_error_mm_exact())?;
    transcript.rational(budget.source_curve_allocation_mm_exact())?;
    transcript.rational(budget.controller_interpolation_allocation_mm_exact())?;
    push_policy(&mut transcript, evidence.policy())?;

    transcript.usize(program.grid_phases().len())?;
    for phases in program.grid_phases() {
        transcript.usize(phases.len())?;
        for phase in phases {
            transcript.real(&phase.path_length)?;
            transcript.real(&phase.ramp.start_feed)?;
            transcript.real(&phase.ramp.end_feed)?;
            transcript.real(&phase.ramp.start_acceleration)?;
            transcript.real(&phase.ramp.end_acceleration)?;
            transcript.real(&phase.ramp.traversal_time)?;
        }
    }
    transcript.usize(evidence.phase_evidence().len())?;
    for phase in evidence.phase_evidence() {
        transcript.usize(phase.element_index())?;
        transcript.usize(phase.phase_index())?;
        transcript.real(phase.original_duration_seconds())?;
        transcript.rational(phase.grid_duration_seconds())?;
        transcript.real(phase.nonnegative_padding_seconds())?;
        transcript.u64(phase.update_count())?;
    }

    transcript.usize(evidence.record_evidence().len())?;
    for record in evidence.record_evidence() {
        transcript.usize(record.element_index())?;
        transcript.usize(record.phase_index())?;
        transcript.u64(record.first_phase_update())?;
        push_segment(&mut transcript, record.segment())?;
        for axis in record.axes() {
            push_projection(&mut transcript, axis.first_difference())?;
            push_projection(&mut transcript, axis.second_difference())?;
            push_projection(&mut transcript, axis.third_difference())?;
            transcript.rational(axis.incoming_position_error_steps())?;
            transcript.rational(axis.terminal_position_error_steps())?;
        }
    }
    for error in evidence.maximum_axis_position_error_steps() {
        transcript.rational(error)?;
    }
    transcript.rational(evidence.maximum_position_error_mm())?;
    transcript.u64(evidence.total_update_count())?;

    for coordinate in program.initial_position() {
        transcript.i64(coordinate)?;
    }
    for coordinate in program.final_position() {
        transcript.i64(coordinate)?;
    }
    transcript.u64(preflight.end_tick.0)?;
    for coordinate in preflight.position {
        transcript.i64(coordinate)?;
    }
    for coordinate in preflight.terminal_finite_position {
        transcript.i64(coordinate)?;
    }
    for count in preflight.emitted_steps {
        transcript.u64(count)?;
    }
    transcript.u32(preflight.segment_count)?;
    transcript.u64(preflight.update_count)?;
    transcript.u64(preflight.earliest_finish_cycle.0)?;
    transcript.u64(preflight.maximum_position_error)?;

    transcript.raw(&partition.policy().stream_id().0)?;
    transcript.raw(&publication.object.content.digest.0)?;
    transcript.raw(&publication.manifest.digest.0)?;
    transcript.u64(publication.object.byte_len)?;
    transcript.u32(partition.block_count())?;
    transcript.usize(partition.maximum_segments_per_block())?;
    transcript.u64(partition.maximum_observed_block_ticks())?;
    transcript.u32(partition.maximum_finite_difference_updates())?;
    transcript.raw(&terminal.block_digest.0)?;
    transcript.u64(terminal.end_tick.0)?;
    for coordinate in terminal.position {
        transcript.i64(coordinate)?;
    }
    for coordinate in partition
        .terminal_finite_position()
        .ok_or(DirectMotionEvidenceError::PartitionMismatch)?
    {
        transcript.i64(coordinate)?;
    }
    transcript.u64(
        partition
            .finite_difference_update_count()
            .ok_or(DirectMotionEvidenceError::PartitionMismatch)?,
    )?;
    let (transcript_digest, transcript_byte_len) = transcript.finish();

    let record_count = u32::try_from(program.records().len())
        .map_err(|_| DirectMotionEvidenceError::CounterOverflow)?;
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(EVIDENCE_BYTES)
        .map_err(|_| DirectMotionEvidenceError::AllocationOverflow)?;
    encoded.extend_from_slice(&EVIDENCE_MAGIC);
    encoded.extend_from_slice(&EVIDENCE_VERSION.to_le_bytes());
    encoded.extend_from_slice(&AXES.to_le_bytes());
    encoded.extend_from_slice(&CERTIFICATION_FLAGS.to_le_bytes());
    encoded.extend_from_slice(&program.configuration_digest().0);
    encoded.extend_from_slice(&program.capability_digest().0);
    encoded.extend_from_slice(&planner.source_digest.0);
    encoded.extend_from_slice(&planner.metric_path_digest.0);
    encoded.extend_from_slice(&planner.source_approximation_digest.0);
    encoded.extend_from_slice(&planner.planner_transcript_digest.0);
    encoded.extend_from_slice(&planner.planner_transcript_byte_len.to_le_bytes());
    encoded.extend_from_slice(&transcript_digest.0);
    encoded.extend_from_slice(&transcript_byte_len.to_le_bytes());
    encoded.extend_from_slice(&publication.object.content.digest.0);
    encoded.extend_from_slice(&publication.manifest.digest.0);
    encoded.extend_from_slice(&publication.object.byte_len.to_le_bytes());
    encoded.extend_from_slice(&partition.block_count().to_le_bytes());
    encoded.extend_from_slice(&record_count.to_le_bytes());
    encoded.extend_from_slice(&evidence.total_update_count().to_le_bytes());
    debug_assert_eq!(encoded.len(), EVIDENCE_BYTES);
    let digest = sha256(&encoded).digest;
    Ok(CanonicalDirectMotionEvidence1 {
        encoded,
        digest,
        transcript_digest,
        transcript_byte_len,
        source_digest: planner.source_digest,
        metric_path_digest: planner.metric_path_digest,
        planner_transcript_digest: planner.planner_transcript_digest,
        record_count,
        update_count: evidence.total_update_count(),
    })
}

/// Rebuild every exact field and require byte-for-byte transcript identity.
pub fn replay_direct_motion_evidence(
    evidence: &CanonicalDirectMotionEvidence1,
    schedule: &CertifiedJerkSchedule2,
    program: &CanonicalDirectFiniteDifferenceProgram2,
    partition: &CanonicalMachinePartition2,
) -> DirectMotionEvidenceResult<()> {
    verify_direct_motion_evidence_bytes(
        evidence.encoded(),
        evidence.digest(),
        schedule,
        program,
        partition,
    )?;
    let rebuilt = build_direct_motion_evidence(schedule, program, partition)?;
    (rebuilt.transcript_digest == evidence.transcript_digest
        && rebuilt.transcript_byte_len == evidence.transcript_byte_len
        && rebuilt.source_digest == evidence.source_digest
        && rebuilt.metric_path_digest == evidence.metric_path_digest
        && rebuilt.planner_transcript_digest == evidence.planner_transcript_digest
        && rebuilt.record_count == evidence.record_count
        && rebuilt.update_count == evidence.update_count)
        .then_some(())
        .ok_or(DirectMotionEvidenceError::ReplayMismatch)
}

/// Verify externally stored outer bytes against their expected digest and a
/// fresh reconstruction from the exact in-memory derivation.
pub fn verify_direct_motion_evidence_bytes(
    encoded: &[u8],
    expected_digest: Digest,
    schedule: &CertifiedJerkSchedule2,
    program: &CanonicalDirectFiniteDifferenceProgram2,
    partition: &CanonicalMachinePartition2,
) -> DirectMotionEvidenceResult<()> {
    if sha256(encoded).digest != expected_digest {
        return Err(DirectMotionEvidenceError::DigestMismatch);
    }
    let rebuilt = build_direct_motion_evidence(schedule, program, partition)?;
    if encoded != rebuilt.encoded() || expected_digest != rebuilt.digest() {
        return Err(DirectMotionEvidenceError::ReplayMismatch);
    }
    Ok(())
}

fn validate_inputs(
    schedule: &CertifiedJerkSchedule2,
    program: &CanonicalDirectFiniteDifferenceProgram2,
    partition: &CanonicalMachinePartition2,
) -> DirectMotionEvidenceResult<()> {
    if schedule.configuration_digest() != program.configuration_digest()
        || schedule.capability_digest() != program.capability_digest()
        || schedule.source() != program.source()
        || schedule.metric_path() != program.metric_path()
    {
        return Err(DirectMotionEvidenceError::PlannerMismatch);
    }
    if partition.execution_kind() != ExecutionKind::FiniteDifference
        || partition.policy().config_digest() != program.configuration_digest()
        || partition.policy().capability_digest() != program.capability_digest()
        || partition.local_timer_hz() != program.timer_ticks_per_second()
    {
        return Err(DirectMotionEvidenceError::PartitionMismatch);
    }
    let evidence = program.evidence();
    let preflight = program.executor_preflight();
    if !program.grid_jerk_report().all_satisfied()
        || evidence.phase_evidence().len()
            != program.grid_phases().iter().map(Vec::len).sum::<usize>()
        || evidence.record_evidence().len() != program.records().len()
        || evidence
            .record_evidence()
            .iter()
            .zip(program.records())
            .any(|(record, segment)| record.segment() != *segment)
        || evidence.maximum_position_error_mm() > evidence.policy().maximum_position_error_mm()
        || partition.initial_position() != program.initial_position()
        || partition.final_position() != program.final_position()
        || partition.terminal_progress().end_tick != preflight.end_tick
        || partition.terminal_progress().position
            != relative_position(program.initial_position(), program.final_position())?
        || partition.terminal_finite_position() != Some(preflight.terminal_finite_position)
        || partition.finite_difference_update_count() != Some(preflight.update_count)
        || evidence.total_update_count() != preflight.update_count
        || usize::try_from(preflight.segment_count).ok() != Some(program.records().len())
    {
        return Err(DirectMotionEvidenceError::PartitionMismatch);
    }
    Ok(())
}

fn relative_position(
    initial: [i64; 2],
    terminal: [i64; 2],
) -> DirectMotionEvidenceResult<[i64; 2]> {
    Ok([
        terminal[0]
            .checked_sub(initial[0])
            .ok_or(DirectMotionEvidenceError::CounterOverflow)?,
        terminal[1]
            .checked_sub(initial[1])
            .ok_or(DirectMotionEvidenceError::CounterOverflow)?,
    ])
}

fn push_policy(
    transcript: &mut TranscriptEncoder,
    policy: &DirectFiniteDifferencePolicy2,
) -> Result<(), ScheduleEvidenceError> {
    transcript.usize(policy.maximum_records())?;
    transcript.u32(policy.maximum_updates_per_record())?;
    transcript.u64(policy.maximum_steps_per_record())?;
    transcript.u16(policy.coefficient_precision_bits())?;
    transcript.rational(policy.maximum_position_error_mm())
}

fn push_segment(
    transcript: &mut TranscriptEncoder,
    segment: FiniteDifferenceSegment<2>,
) -> Result<(), ScheduleEvidenceError> {
    transcript.u64(segment.start_tick.0)?;
    transcript.u64(segment.end_tick.0)?;
    transcript.u32(segment.update_period_ticks)?;
    transcript.u32(segment.update_count)?;
    transcript.u32(segment.flags)?;
    for axis in segment.axes {
        push_axis(transcript, axis)?;
    }
    Ok(())
}

fn push_axis(
    transcript: &mut TranscriptEncoder,
    axis: FiniteDifferenceAxis,
) -> Result<(), ScheduleEvidenceError> {
    transcript.i64(axis.initial_position)?;
    transcript.i64(axis.first_difference)?;
    transcript.i64(axis.second_difference)?;
    transcript.i64(axis.third_difference)
}

fn push_projection(
    transcript: &mut TranscriptEncoder,
    projection: &DirectCoefficientProjection2,
) -> Result<(), ScheduleEvidenceError> {
    transcript.real(projection.ideal_steps())?;
    transcript.rational(&projection.scaled_interval()[0])?;
    transcript.rational(&projection.scaled_interval()[1])?;
    transcript.i64(projection.encoded_q31_32())?;
    transcript.rational(projection.maximum_error_steps())
}

/// Failure while constructing or replaying exact direct-motion evidence.
#[derive(Debug)]
pub enum DirectMotionEvidenceError {
    /// Planner and direct-program identities or retained paths differed.
    PlannerMismatch,
    /// Direct program, firmware preflight, and immutable partition diverged.
    PartitionMismatch,
    /// A bounded count did not fit its canonical field.
    CounterOverflow,
    /// Outer-record allocation failed.
    AllocationOverflow,
    /// Supplied outer bytes did not match their claimed SHA-256.
    DigestMismatch,
    /// Fresh exact reconstruction was not byte-identical.
    ReplayMismatch,
    /// Shared exact source/planner transcript construction failed.
    Schedule(ScheduleEvidenceError),
}

impl fmt::Display for DirectMotionEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlannerMismatch => formatter.write_str("direct evidence planner mismatch"),
            Self::PartitionMismatch => {
                formatter.write_str("direct evidence partition or terminal mismatch")
            }
            Self::CounterOverflow => formatter.write_str("direct evidence counter overflow"),
            Self::AllocationOverflow => formatter.write_str("direct evidence allocation failed"),
            Self::DigestMismatch => formatter.write_str("direct evidence digest mismatch"),
            Self::ReplayMismatch => formatter.write_str("direct evidence replay mismatch"),
            Self::Schedule(source) => write!(formatter, "direct exact transcript failed: {source}"),
        }
    }
}

impl StdError for DirectMotionEvidenceError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Schedule(source) => Some(source),
            _ => None,
        }
    }
}

impl From<ScheduleEvidenceError> for DirectMotionEvidenceError {
    fn from(value: ScheduleEvidenceError) -> Self {
        Self::Schedule(value)
    }
}
