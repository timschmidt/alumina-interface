//! Canonical content evidence for exact-source scheduled machine partitions.

use std::error::Error as StdError;
use std::fmt;
use std::io::{self, Write};

use alumina_machine_ir::ExecutionSegment;
use alumina_motion::{MotionError, StepperPreflightSummary};
use alumina_protocol::Digest;
use alumina_storage::{ContentHasher, sha256};
use hypercurve::{CurveFamily2, CurveGeometry2, CurvePath2};
use hyperlimit::{Certainty, PredicatePolicy};
use hyperpath::{
    AxisProjectedMotionLimitsReport, CornerLookaheadJoinClass, JerkRampElementPhaseReport,
    JerkRampPhaseProposal, JerkRampSpanProposal, LookaheadFeedSchedule,
    LookaheadFeedScheduleReport, PlannedAxisProjectedMotionLimits,
    PlannedJerkFeasibleLookaheadSchedule, PlannedLookaheadFeedSchedule,
    PlannedMonotonicJerkTransition, TangentAlignment, TangentJoinClass, TangentJoinReport,
    TangentSpan,
};
use hyperreal::{Rational, Real, RealSign, RealSignCertificate};
use hypersolve::{
    CandidateCertificationReport, CertifiedCandidateStatus, ConstraintKind, ProposalEngineKind,
    ProposalEnginePrecision,
};

use crate::motion_schedule::{
    CanonicalScheduledProgram2, CertifiedJerkSchedule2, ScheduledLoweringEvidence2,
    TimerLatticeScheduleReport2,
};
use crate::partition::CanonicalMachinePartition2;
use crate::toolpath::CertifiedMetricPath2;

const EVIDENCE_MAGIC: [u8; 8] = *b"ALMEVD03";
const EVIDENCE_VERSION: u16 = 3;
const SOURCE_MAGIC: [u8; 8] = *b"ALMSRC02";
const SOURCE_VERSION: u16 = 2;
const METRIC_MAGIC: [u8; 8] = *b"ALMMTR01";
const METRIC_VERSION: u16 = 1;
const APPROXIMATION_MAGIC: [u8; 8] = *b"ALMAPX01";
const APPROXIMATION_VERSION: u16 = 1;
const PLANNER_MAGIC: [u8; 8] = *b"ALMPLN01";
const PLANNER_VERSION: u16 = 1;
const LOWERING_MAGIC: [u8; 8] = *b"ALMLOW01";
const LOWERING_VERSION: u16 = 1;
const EXACT_REAL_FORMAT_VERSION: u16 = 1;
const MAXIMUM_EXACT_REAL_ENCODING_BYTES: usize = 1_048_576;
const MAXIMUM_SUBTRANSCRIPT_BYTES: u64 = 64 * 1_048_576;
const AXES: u16 = 2;
const CERTIFIED_LOOKAHEAD: u32 = 1 << 0;
const CERTIFIED_JERK_REPLAY: u32 = 1 << 1;
const CERTIFIED_INTERPOLATION: u32 = 1 << 2;
const CERTIFIED_EXECUTOR_PREFLIGHT: u32 = 1 << 3;
const CERTIFIED_PARTITION_STREAM_REPLAY: u32 = 1 << 4;
const CERTIFIED_SOURCE_APPROXIMATION: u32 = 1 << 5;
const CERTIFIED_PLANNER_TRANSCRIPT: u32 = 1 << 6;
const CERTIFIED_LOWERING_TRANSCRIPT: u32 = 1 << 7;
const CERTIFICATION_FLAGS: u32 = CERTIFIED_LOOKAHEAD
    | CERTIFIED_JERK_REPLAY
    | CERTIFIED_INTERPOLATION
    | CERTIFIED_EXECUTOR_PREFLIGHT
    | CERTIFIED_PARTITION_STREAM_REPLAY
    | CERTIFIED_SOURCE_APPROXIMATION
    | CERTIFIED_PLANNER_TRANSCRIPT
    | CERTIFIED_LOWERING_TRANSCRIPT;

/// Result type for canonical schedule-evidence construction and replay.
pub type ScheduleEvidenceResult<T> = Result<T, ScheduleEvidenceError>;

/// Immutable deterministic transcript binding exact source, machine policy,
/// canonical IR, and content-addressed cached partition identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalScheduleEvidence3 {
    encoded: Vec<u8>,
    digest: Digest,
    source_digest: Digest,
    metric_path_digest: Digest,
    source_approximation_digest: Digest,
    planner_transcript_digest: Digest,
    planner_transcript_byte_len: u64,
    lowering_transcript_digest: Digest,
    lowering_transcript_byte_len: u64,
}

impl CanonicalScheduleEvidence3 {
    /// Complete canonical V3 transcript bytes.
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    /// SHA-256 identity of the complete evidence transcript.
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// SHA-256 identity of the retained exact rational source transcript.
    pub const fn source_digest(&self) -> Digest {
        self.source_digest
    }

    /// SHA-256 identity of the exact line/arc path presented to Hyperpath.
    pub const fn metric_path_digest(&self) -> Digest {
        self.metric_path_digest
    }

    /// SHA-256 identity of source-to-motion spans and exact error bounds.
    pub const fn source_approximation_digest(&self) -> Digest {
        self.source_approximation_digest
    }

    /// SHA-256 identity of exact planner policy and certification decisions.
    pub const fn planner_transcript_digest(&self) -> Digest {
        self.planner_transcript_digest
    }

    /// Canonical planner transcript length committed beside its digest.
    pub const fn planner_transcript_byte_len(&self) -> u64 {
        self.planner_transcript_byte_len
    }

    /// SHA-256 identity of interpolation, timer, and canonical lowering decisions.
    pub const fn lowering_transcript_digest(&self) -> Digest {
        self.lowering_transcript_digest
    }

    /// Canonical lowering transcript length committed beside its digest.
    pub const fn lowering_transcript_byte_len(&self) -> u64 {
        self.lowering_transcript_byte_len
    }
}

/// Construct deterministic evidence only after identity and terminal replay
/// facts agree across the scheduled program and packaged partition.
pub fn build_canonical_schedule_evidence(
    schedule: &CertifiedJerkSchedule2,
    program: &CanonicalScheduledProgram2,
    partition: &CanonicalMachinePartition2,
) -> ScheduleEvidenceResult<CanonicalScheduleEvidence3> {
    if schedule.configuration_digest() != program.configuration_digest()
        || schedule.capability_digest() != program.capability_digest()
        || schedule.source() != program.source()
        || schedule.metric_path() != program.metric_path()
    {
        return Err(ScheduleEvidenceError::PlannerMismatch);
    }
    if program.configuration_digest() != partition.policy().config_digest()
        || program.capability_digest() != partition.policy().capability_digest()
    {
        return Err(ScheduleEvidenceError::IdentityMismatch);
    }
    let first = program
        .points()
        .first()
        .ok_or(ScheduleEvidenceError::EmptyProgram)?;
    let last = program
        .points()
        .last()
        .ok_or(ScheduleEvidenceError::EmptyProgram)?;
    let initial_position = [first.steps()[0].get(), first.steps()[1].get()];
    let final_position = [last.steps()[0].get(), last.steps()[1].get()];
    let preflight = program.executor_preflight();
    if last.ideal_time_seconds() != schedule.total_traversal_time_seconds()
        || program
            .evidence()
            .timer_lattice_schedule()
            .ideal_total_time_seconds()
            != last.ideal_time_seconds()
    {
        return Err(ScheduleEvidenceError::PlannerMismatch);
    }
    if partition.initial_position() != initial_position
        || partition.final_position() != final_position
        || partition.local_timer_hz() != program.timer_ticks_per_second()
        || partition.terminal_progress().end_tick != preflight.end_tick
        || preflight.position != final_position
        || usize::try_from(preflight.segment_count).ok() != Some(program.segments().len())
    {
        return Err(ScheduleEvidenceError::TerminalMismatch);
    }

    let source = encode_exact_path(program.source(), ExactPathDomain::Source)?;
    let source_digest = sha256(&source).digest;
    let metric_path = encode_exact_path(program.metric_path().path(), ExactPathDomain::Metric)?;
    let metric_path_digest = sha256(&metric_path).digest;
    let source_approximation =
        encode_source_approximation(program.source(), program.metric_path())?;
    let source_approximation_digest = sha256(&source_approximation).digest;
    let (planner_transcript_digest, planner_transcript_byte_len) =
        encode_planner_transcript(schedule)?;
    let (lowering_transcript_digest, lowering_transcript_byte_len) =
        encode_lowering_transcript(program)?;
    let publication = partition.publication();
    let budget = program.resolution_budget();
    let requested_interpolation = program.evidence().requested_interpolation_error_mm_exact();

    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(512)
        .map_err(|_| ScheduleEvidenceError::AllocationOverflow)?;
    encoded.extend_from_slice(&EVIDENCE_MAGIC);
    push_u16(&mut encoded, EVIDENCE_VERSION);
    push_u16(&mut encoded, AXES);
    push_u32(&mut encoded, CERTIFICATION_FLAGS);
    encoded.extend_from_slice(&program.configuration_digest().0);
    encoded.extend_from_slice(&program.capability_digest().0);
    encoded.extend_from_slice(&source_digest.0);
    encoded.extend_from_slice(&metric_path_digest.0);
    encoded.extend_from_slice(&source_approximation_digest.0);
    encoded.extend_from_slice(&planner_transcript_digest.0);
    push_u64(&mut encoded, planner_transcript_byte_len);
    encoded.extend_from_slice(&lowering_transcript_digest.0);
    push_u64(&mut encoded, lowering_transcript_byte_len);
    encoded.extend_from_slice(&publication.object.content.digest.0);
    encoded.extend_from_slice(&publication.manifest.digest.0);
    push_u64(&mut encoded, publication.object.byte_len);
    push_u64(&mut encoded, program.timer_ticks_per_second());
    push_u32(&mut encoded, program.output_quantum_cycles());
    push_u32(&mut encoded, partition.block_count());
    push_usize(&mut encoded, program.points().len())?;
    push_usize(&mut encoded, program.segments().len())?;
    for coordinate in initial_position {
        push_i64(&mut encoded, coordinate);
    }
    for coordinate in final_position {
        push_i64(&mut encoded, coordinate);
    }
    push_u64(&mut encoded, preflight.end_tick.0);
    push_u64(&mut encoded, preflight.earliest_finish_cycle.0);
    push_u32(&mut encoded, preflight.segment_count);
    for count in preflight.emitted_steps {
        push_u64(&mut encoded, count);
    }
    for rational in [
        budget.requested_total_error_mm_exact(),
        budget.source_curve_allocation_mm_exact(),
        budget.controller_interpolation_allocation_mm_exact(),
        program.evidence().maximum_source_to_motion_error_mm_exact(),
        requested_interpolation,
    ] {
        push_rational(&mut encoded, rational)?;
    }

    let digest = sha256(&encoded).digest;
    Ok(CanonicalScheduleEvidence3 {
        encoded,
        digest,
        source_digest,
        metric_path_digest,
        source_approximation_digest,
        planner_transcript_digest,
        planner_transcript_byte_len,
        lowering_transcript_digest,
        lowering_transcript_byte_len,
    })
}

/// Rebuild the canonical transcript and require byte-for-byte identity. This
/// reruns exact source serialization and all cross-artifact consistency gates;
/// it does not trust fields parsed from the supplied evidence.
pub fn replay_canonical_schedule_evidence(
    evidence: &CanonicalScheduleEvidence3,
    schedule: &CertifiedJerkSchedule2,
    program: &CanonicalScheduledProgram2,
    partition: &CanonicalMachinePartition2,
) -> ScheduleEvidenceResult<()> {
    verify_canonical_schedule_evidence_bytes(
        evidence.encoded(),
        evidence.digest,
        schedule,
        program,
        partition,
    )?;
    let rebuilt = build_canonical_schedule_evidence(schedule, program, partition)?;
    (rebuilt.source_digest == evidence.source_digest
        && rebuilt.metric_path_digest == evidence.metric_path_digest
        && rebuilt.source_approximation_digest == evidence.source_approximation_digest
        && rebuilt.planner_transcript_digest == evidence.planner_transcript_digest
        && rebuilt.planner_transcript_byte_len == evidence.planner_transcript_byte_len
        && rebuilt.lowering_transcript_digest == evidence.lowering_transcript_digest
        && rebuilt.lowering_transcript_byte_len == evidence.lowering_transcript_byte_len)
        .then_some(())
        .ok_or(ScheduleEvidenceError::ReplayMismatch)
}

/// Verify externally stored evidence bytes against an expected SHA-256 and a
/// fresh exact reconstruction. Unknown versions, reordered fields, truncated
/// rationals, and otherwise canonical-looking substitutions all fail by byte
/// inequality without becoming a second parser or source of truth.
pub fn verify_canonical_schedule_evidence_bytes(
    encoded: &[u8],
    expected_digest: Digest,
    schedule: &CertifiedJerkSchedule2,
    program: &CanonicalScheduledProgram2,
    partition: &CanonicalMachinePartition2,
) -> ScheduleEvidenceResult<()> {
    if sha256(encoded).digest != expected_digest {
        return Err(ScheduleEvidenceError::DigestMismatch);
    }
    let rebuilt = build_canonical_schedule_evidence(schedule, program, partition)?;
    if encoded != rebuilt.encoded() || expected_digest != rebuilt.digest() {
        return Err(ScheduleEvidenceError::ReplayMismatch);
    }
    Ok(())
}

struct TranscriptEncoder {
    hasher: ContentHasher,
    byte_len: u64,
}

struct BoundedExactRealEncoding {
    encoded: Vec<u8>,
    limit_exceeded: bool,
    allocation_failed: bool,
}

impl BoundedExactRealEncoding {
    const fn new() -> Self {
        Self {
            encoded: Vec::new(),
            limit_exceeded: false,
            allocation_failed: false,
        }
    }
}

impl Write for BoundedExactRealEncoding {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(next_len) = self.encoded.len().checked_add(bytes.len()) else {
            self.limit_exceeded = true;
            return Err(io::Error::other("exact Real encoding length overflow"));
        };
        if next_len > MAXIMUM_EXACT_REAL_ENCODING_BYTES {
            self.limit_exceeded = true;
            return Err(io::Error::other("exact Real encoding limit exceeded"));
        }
        if self.encoded.try_reserve(bytes.len()).is_err() {
            self.allocation_failed = true;
            return Err(io::Error::other("exact Real encoding allocation failed"));
        }
        self.encoded.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TranscriptEncoder {
    fn new() -> Self {
        Self {
            hasher: ContentHasher::new(),
            byte_len: 0,
        }
    }

    fn finish(self) -> (Digest, u64) {
        (self.hasher.finalize().digest, self.byte_len)
    }

    fn raw(&mut self, bytes: &[u8]) -> ScheduleEvidenceResult<()> {
        let next_byte_len = self
            .byte_len
            .checked_add(
                u64::try_from(bytes.len()).map_err(|_| ScheduleEvidenceError::CounterOverflow)?,
            )
            .ok_or(ScheduleEvidenceError::CounterOverflow)?;
        if next_byte_len > MAXIMUM_SUBTRANSCRIPT_BYTES {
            return Err(ScheduleEvidenceError::SubtranscriptTooLarge);
        }
        self.byte_len = next_byte_len;
        self.hasher.update(bytes);
        Ok(())
    }

    fn u8(&mut self, value: u8) -> ScheduleEvidenceResult<()> {
        self.raw(&[value])
    }

    fn bool(&mut self, value: bool) -> ScheduleEvidenceResult<()> {
        self.u8(u8::from(value))
    }

    fn u16(&mut self, value: u16) -> ScheduleEvidenceResult<()> {
        self.raw(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) -> ScheduleEvidenceResult<()> {
        self.raw(&value.to_le_bytes())
    }

    fn i32(&mut self, value: i32) -> ScheduleEvidenceResult<()> {
        self.raw(&value.to_le_bytes())
    }

    fn u64(&mut self, value: u64) -> ScheduleEvidenceResult<()> {
        self.raw(&value.to_le_bytes())
    }

    fn i64(&mut self, value: i64) -> ScheduleEvidenceResult<()> {
        self.raw(&value.to_le_bytes())
    }

    fn usize(&mut self, value: usize) -> ScheduleEvidenceResult<()> {
        self.u32(u32::try_from(value).map_err(|_| ScheduleEvidenceError::CounterOverflow)?)
    }

    fn bytes(&mut self, bytes: &[u8]) -> ScheduleEvidenceResult<()> {
        self.usize(bytes.len())?;
        self.raw(bytes)
    }

    fn string(&mut self, value: &str) -> ScheduleEvidenceResult<()> {
        self.bytes(value.as_bytes())
    }

    fn rational(&mut self, value: &Rational) -> ScheduleEvidenceResult<()> {
        self.u8(if value.is_zero() {
            0
        } else if value.is_negative() {
            2
        } else {
            1
        })?;
        self.bytes(&value.numerator().to_bytes_le())?;
        self.bytes(&value.denominator().to_bytes_le())
    }

    fn real(&mut self, value: &Real) -> ScheduleEvidenceResult<()> {
        let mut encoded = BoundedExactRealEncoding::new();
        if serde_json::to_writer(&mut encoded, value).is_err() {
            if encoded.limit_exceeded {
                return Err(ScheduleEvidenceError::ExactRealEncodingTooLarge);
            }
            if encoded.allocation_failed {
                return Err(ScheduleEvidenceError::AllocationOverflow);
            }
            return Err(ScheduleEvidenceError::ExactRealEncoding);
        }
        self.bytes(&encoded.encoded)
    }

    fn optional_real(&mut self, value: Option<&Real>) -> ScheduleEvidenceResult<()> {
        match value {
            Some(value) => {
                self.u8(1)?;
                self.real(value)
            }
            None => self.u8(0),
        }
    }
}

fn push_real_sign(encoded: &mut TranscriptEncoder, sign: RealSign) -> ScheduleEvidenceResult<()> {
    encoded.u8(match sign {
        RealSign::Negative => 0,
        RealSign::Zero => 1,
        RealSign::Positive => 2,
    })
}

fn push_sign_certificate(
    encoded: &mut TranscriptEncoder,
    certificate: RealSignCertificate,
) -> ScheduleEvidenceResult<()> {
    match certificate {
        RealSignCertificate::StructuralFacts => encoded.u8(0),
        RealSignCertificate::ExactZeroScale => encoded.u8(1),
        RealSignCertificate::BoundedRefinement { min_precision } => {
            encoded.u8(2)?;
            encoded.i32(min_precision)
        }
    }
}

fn push_certainty(
    encoded: &mut TranscriptEncoder,
    certainty: Certainty,
) -> ScheduleEvidenceResult<()> {
    encoded.u8(match certainty {
        Certainty::Exact => 0,
        Certainty::Filtered => 1,
        Certainty::Approximate => 2,
    })
}

fn push_constraint_kind(
    encoded: &mut TranscriptEncoder,
    kind: ConstraintKind,
) -> ScheduleEvidenceResult<()> {
    encoded.u8(match kind {
        ConstraintKind::Equality => 0,
        ConstraintKind::LessOrEqual => 1,
        ConstraintKind::GreaterOrEqual => 2,
        ConstraintKind::Soft => 3,
    })
}

fn push_proposal_engine(
    encoded: &mut TranscriptEncoder,
    engine: ProposalEngineKind,
) -> ScheduleEvidenceResult<()> {
    encoded.u8(match engine {
        ProposalEngineKind::DampedLeastSquares => 0,
        ProposalEngineKind::PowellHybrid => 1,
        ProposalEngineKind::LevenbergMarquardt => 2,
        ProposalEngineKind::ModifiedNewtonLeastSquares => 3,
        ProposalEngineKind::Dogleg => 4,
        ProposalEngineKind::Bfgs => 5,
        ProposalEngineKind::Sqp => 6,
    })
}

fn push_proposal_precision(
    encoded: &mut TranscriptEncoder,
    precision: ProposalEnginePrecision,
) -> ScheduleEvidenceResult<()> {
    encoded.u8(match precision {
        ProposalEnginePrecision::LossyF64 => 0,
        ProposalEnginePrecision::Unsupported => 1,
    })
}

fn push_candidate_certification(
    encoded: &mut TranscriptEncoder,
    report: &CandidateCertificationReport,
) -> ScheduleEvidenceResult<()> {
    encoded.usize(report.rows.len())?;
    encoded.usize(report.certified_satisfied_rows)?;
    encoded.usize(report.certified_violation_rows)?;
    encoded.usize(report.bounded_unknown_rows)?;
    encoded.usize(report.lossy_adapter_only_rows)?;
    encoded.usize(report.domain_failure_rows)?;
    for row in &report.rows {
        encoded.usize(row.constraint_index)?;
        encoded.string(&row.name)?;
        push_constraint_kind(encoded, row.kind)?;
        encoded.optional_real(row.signed_residual.as_ref())?;
        match &row.status {
            CertifiedCandidateStatus::CertifiedZero { certificate } => {
                encoded.u8(0)?;
                push_sign_certificate(encoded, *certificate)?;
            }
            CertifiedCandidateStatus::CertifiedSatisfiedInequality { certificate } => {
                encoded.u8(1)?;
                push_sign_certificate(encoded, *certificate)?;
            }
            CertifiedCandidateStatus::CertifiedViolation { sign, certificate } => {
                encoded.u8(2)?;
                push_real_sign(encoded, *sign)?;
                push_sign_certificate(encoded, *certificate)?;
            }
            CertifiedCandidateStatus::BoundedUnknown { min_precision } => {
                encoded.u8(3)?;
                encoded.i32(*min_precision)?;
            }
            CertifiedCandidateStatus::BallCertified {
                sign,
                certainty,
                satisfied,
            } => {
                encoded.u8(4)?;
                push_real_sign(encoded, *sign)?;
                push_certainty(encoded, *certainty)?;
                encoded.bool(*satisfied)?;
            }
            CertifiedCandidateStatus::InvalidBallRadius => encoded.u8(5)?,
            CertifiedCandidateStatus::DomainFailure { message } => {
                encoded.u8(6)?;
                encoded.string(message)?;
            }
            CertifiedCandidateStatus::LossyAdapterOnly {
                requested,
                used,
                precision,
            } => {
                encoded.u8(7)?;
                push_proposal_engine(encoded, *requested)?;
                match used {
                    Some(engine) => {
                        encoded.u8(1)?;
                        push_proposal_engine(encoded, *engine)?;
                    }
                    None => encoded.u8(0)?,
                }
                push_proposal_precision(encoded, *precision)?;
            }
        }
    }
    Ok(())
}

fn push_candidate_certifications(
    encoded: &mut TranscriptEncoder,
    reports: &[CandidateCertificationReport],
) -> ScheduleEvidenceResult<()> {
    encoded.usize(reports.len())?;
    for report in reports {
        push_candidate_certification(encoded, report)?;
    }
    Ok(())
}

fn push_tangent_alignment(
    encoded: &mut TranscriptEncoder,
    alignment: TangentAlignment,
) -> ScheduleEvidenceResult<()> {
    encoded.u8(match alignment {
        TangentAlignment::SameDirection => 0,
        TangentAlignment::OppositeDirection => 1,
        TangentAlignment::NotParallel => 2,
        TangentAlignment::Degenerate => 3,
        TangentAlignment::Unknown => 4,
    })
}

fn push_tangent_join_class(
    encoded: &mut TranscriptEncoder,
    class: TangentJoinClass,
) -> ScheduleEvidenceResult<()> {
    encoded.u8(match class {
        TangentJoinClass::G1Continuous => 0,
        TangentJoinClass::ReversedTangent => 1,
        TangentJoinClass::Corner => 2,
        TangentJoinClass::DegenerateTangent => 3,
        TangentJoinClass::EndpointMismatch => 4,
        TangentJoinClass::Unknown => 5,
    })
}

fn push_tangent_join_report(
    encoded: &mut TranscriptEncoder,
    report: TangentJoinReport,
) -> ScheduleEvidenceResult<()> {
    push_tangent_join_class(encoded, report.class)?;
    match report.endpoints_equal {
        Some(value) => {
            encoded.u8(1)?;
            encoded.bool(value)?;
        }
        None => encoded.u8(0)?,
    }
    match report.alignment {
        Some(value) => {
            encoded.u8(1)?;
            push_tangent_alignment(encoded, value)?;
        }
        None => encoded.u8(0)?,
    }
    Ok(())
}

fn push_corner_class(
    encoded: &mut TranscriptEncoder,
    class: CornerLookaheadJoinClass,
) -> ScheduleEvidenceResult<()> {
    encoded.u8(match class {
        CornerLookaheadJoinClass::StraightThrough => 0,
        CornerLookaheadJoinClass::RadiusLimitedCorner => 1,
        CornerLookaheadJoinClass::ReversalStop => 2,
    })
}

fn push_exact_point(
    encoded: &mut TranscriptEncoder,
    point: &hyperlimit::Point2,
) -> ScheduleEvidenceResult<()> {
    encoded.real(&point.x)?;
    encoded.real(&point.y)
}

fn push_tangent_span(
    encoded: &mut TranscriptEncoder,
    span: &TangentSpan,
) -> ScheduleEvidenceResult<()> {
    push_exact_point(encoded, &span.start)?;
    push_exact_point(encoded, &span.start_tangent)?;
    push_exact_point(encoded, &span.end)?;
    push_exact_point(encoded, &span.end_tangent)
}

fn push_axis_projection_report(
    encoded: &mut TranscriptEncoder,
    report: &AxisProjectedMotionLimitsReport,
) -> ScheduleEvidenceResult<()> {
    encoded.real(&report.maximum_path_feed)?;
    encoded.real(&report.maximum_path_acceleration)?;
    encoded.real(&report.maximum_path_jerk)?;
    encoded.usize(report.rows.len())?;
    for row in &report.rows {
        encoded.u64(row.span_index)?;
        encoded.u64(row.axis_index)?;
        encoded.real(&row.absolute_axis_derivative)?;
        encoded.real(&row.axis_limits.maximum_velocity)?;
        encoded.real(&row.axis_limits.maximum_acceleration)?;
        encoded.real(&row.axis_limits.maximum_jerk)?;
        push_candidate_certification(encoded, &row.certification)?;
    }
    Ok(())
}

fn push_axis_projection(
    encoded: &mut TranscriptEncoder,
    projection: Option<&PlannedAxisProjectedMotionLimits>,
) -> ScheduleEvidenceResult<()> {
    let Some(projection) = projection else {
        return encoded.u8(0);
    };
    encoded.u8(1)?;
    encoded.real(&projection.maximum_path_feed)?;
    encoded.real(&projection.maximum_path_acceleration)?;
    encoded.real(&projection.maximum_path_jerk)?;
    for bottleneck in [
        projection.feed_bottleneck,
        projection.acceleration_bottleneck,
        projection.jerk_bottleneck,
    ] {
        encoded.u64(bottleneck.span_index)?;
        encoded.u64(bottleneck.axis_index)?;
    }
    push_axis_projection_report(encoded, &projection.certification)?;
    push_candidate_certification(encoded, &projection.bottleneck_certification)
}

fn push_lookahead_schedule(
    encoded: &mut TranscriptEncoder,
    schedule: &LookaheadFeedSchedule,
) -> ScheduleEvidenceResult<()> {
    encoded.real(&schedule.entry_feed)?;
    encoded.usize(schedule.corner_feeds.len())?;
    for feed in &schedule.corner_feeds {
        encoded.real(feed)?;
    }
    encoded.usize(schedule.corner_radii.len())?;
    for radius in &schedule.corner_radii {
        encoded.real(radius)?;
    }
    encoded.real(&schedule.exit_feed)
}

fn push_lookahead_report(
    encoded: &mut TranscriptEncoder,
    report: &LookaheadFeedScheduleReport,
) -> ScheduleEvidenceResult<()> {
    encoded.usize(report.corners.joins.len())?;
    for join in &report.corners.joins {
        encoded.u64(join.index)?;
        push_tangent_join_report(encoded, join.tangent_join)?;
        push_corner_class(encoded, join.class)?;
        encoded.real(&join.candidate_corner_feed)?;
        encoded.real(&join.max_feed_rate)?;
        encoded.real(&join.max_acceleration)?;
        encoded.real(&join.corner_radius)?;
        push_candidate_certification(encoded, &join.certification)?;
    }
    encoded.usize(report.spans.len())?;
    for span in &report.spans {
        encoded.u64(span.index)?;
        encoded.real(&span.path_length)?;
        encoded.real(&span.start_feed)?;
        encoded.real(&span.end_feed)?;
        encoded.real(&span.max_feed_rate)?;
        encoded.real(&span.max_acceleration)?;
        push_candidate_certification(encoded, &span.certification)?;
    }
    Ok(())
}

fn push_acceleration_lookahead_plan(
    encoded: &mut TranscriptEncoder,
    plan: &PlannedLookaheadFeedSchedule,
) -> ScheduleEvidenceResult<()> {
    encoded.usize(plan.effective_node_feed_limits.len())?;
    for feed in &plan.effective_node_feed_limits {
        encoded.real(feed)?;
    }
    encoded.usize(plan.forward_node_feeds.len())?;
    for feed in &plan.forward_node_feeds {
        encoded.real(feed)?;
    }
    push_lookahead_schedule(encoded, &plan.schedule)?;
    push_candidate_certifications(encoded, &plan.caller_limit_certifications)?;
    push_lookahead_report(encoded, &plan.certification)
}

fn push_jerk_ramp_span(
    encoded: &mut TranscriptEncoder,
    ramp: &JerkRampSpanProposal,
) -> ScheduleEvidenceResult<()> {
    encoded.real(&ramp.start_feed)?;
    encoded.real(&ramp.end_feed)?;
    encoded.real(&ramp.start_acceleration)?;
    encoded.real(&ramp.end_acceleration)?;
    encoded.real(&ramp.traversal_time)
}

fn push_jerk_phase(
    encoded: &mut TranscriptEncoder,
    phase: &JerkRampPhaseProposal,
) -> ScheduleEvidenceResult<()> {
    encoded.real(&phase.path_length)?;
    push_jerk_ramp_span(encoded, &phase.ramp)
}

fn push_jerk_element_report(
    encoded: &mut TranscriptEncoder,
    report: &JerkRampElementPhaseReport,
) -> ScheduleEvidenceResult<()> {
    encoded.u64(report.index)?;
    encoded.real(&report.route_length)?;
    encoded.usize(report.phases.len())?;
    for phase in &report.phases {
        encoded.u64(phase.index)?;
        push_jerk_phase(encoded, &phase.proposal)?;
        push_candidate_certification(encoded, &phase.certification)?;
    }
    push_candidate_certification(encoded, &report.length_certification)?;
    push_candidate_certifications(encoded, &report.continuity)
}

fn push_monotonic_transition(
    encoded: &mut TranscriptEncoder,
    transition: &PlannedMonotonicJerkTransition,
) -> ScheduleEvidenceResult<()> {
    encoded.usize(transition.phases.len())?;
    for phase in &transition.phases {
        push_jerk_phase(encoded, phase)?;
    }
    push_candidate_certification(encoded, &transition.construction_certification)?;
    push_jerk_element_report(encoded, &transition.certification)
}

fn push_jerk_lookahead_plan(
    encoded: &mut TranscriptEncoder,
    plan: &PlannedJerkFeasibleLookaheadSchedule,
) -> ScheduleEvidenceResult<()> {
    push_acceleration_lookahead_plan(encoded, &plan.acceleration_plan)?;
    encoded.usize(plan.positive_node_components.len())?;
    for component in &plan.positive_node_components {
        encoded.u64(component.first_node_index)?;
        encoded.u64(component.last_node_index)?;
        encoded.u32(component.uniform_halvings)?;
    }
    push_lookahead_schedule(encoded, &plan.schedule)?;
    push_candidate_certifications(encoded, &plan.caller_limit_certifications)?;
    push_lookahead_report(encoded, &plan.lookahead_certification)?;
    encoded.usize(plan.span_transitions.len())?;
    for transition in &plan.span_transitions {
        match transition {
            Some(transition) => {
                encoded.u8(1)?;
                push_monotonic_transition(encoded, transition)?;
            }
            None => encoded.u8(0)?,
        }
    }
    Ok(())
}

fn encode_planner_transcript(
    schedule: &CertifiedJerkSchedule2,
) -> ScheduleEvidenceResult<(Digest, u64)> {
    if !schedule.lookahead_plan().all_satisfied()
        || !schedule.jerk_report().all_satisfied()
        || schedule
            .limits()
            .affine_axis_projection()
            .is_some_and(|projection| !projection.all_satisfied())
        || schedule.route().len() != schedule.tangent_spans().len()
        || schedule.route().len() != schedule.phases().len()
    {
        return Err(ScheduleEvidenceError::PlannerMismatch);
    }

    let mut encoded = TranscriptEncoder::new();
    encoded.raw(&PLANNER_MAGIC)?;
    encoded.u16(PLANNER_VERSION)?;
    encoded.u16(EXACT_REAL_FORMAT_VERSION)?;
    encoded.raw(&schedule.configuration_digest().0)?;
    encoded.raw(&schedule.capability_digest().0)?;
    // All current planner predicates use the non-approximating strict policy.
    encoded.u8(0)?;
    encoded.i32(PredicatePolicy::MAX_REFINEMENT_PRECISION)?;

    let approximation = schedule.approximation_limits();
    encoded.usize(approximation.maximum_motion_elements())?;
    encoded.usize(approximation.maximum_subdivision_depth())?;
    encoded.u32(schedule.maximum_jerk_component_halvings())?;
    encoded.usize(schedule.route().len())?;
    encoded.usize(schedule.tangent_spans().len())?;
    for span in schedule.tangent_spans() {
        push_tangent_span(&mut encoded, span)?;
    }

    let travel = schedule.travel_envelope();
    for point in [
        travel.source_minimum_mm(),
        travel.source_maximum_mm(),
        travel.usable_minimum_mm(),
        travel.usable_maximum_mm(),
    ] {
        for coordinate in point {
            encoded.real(coordinate)?;
        }
    }

    let limits = schedule.limits();
    encoded.real(limits.maximum_feed_mm_per_second())?;
    encoded.real(limits.maximum_acceleration_mm_per_second_squared())?;
    encoded.real(limits.maximum_jerk_mm_per_second_cubed())?;
    encoded.real(limits.maximum_spatial_acceleration_mm_per_second_squared())?;
    push_axis_projection(&mut encoded, limits.affine_axis_projection())?;

    let caller = schedule.lookahead_limits();
    encoded.real(&caller.maximum_entry_feed)?;
    encoded.usize(caller.maximum_corner_feeds.len())?;
    for feed in &caller.maximum_corner_feeds {
        encoded.real(feed)?;
    }
    encoded.usize(caller.corner_radii.len())?;
    for radius in &caller.corner_radii {
        encoded.real(radius)?;
    }
    encoded.real(&caller.maximum_exit_feed)?;

    push_jerk_lookahead_plan(&mut encoded, schedule.lookahead_plan())?;
    encoded.usize(schedule.phases().len())?;
    for phases in schedule.phases() {
        encoded.usize(phases.len())?;
        for phase in phases {
            push_jerk_phase(&mut encoded, phase)?;
        }
    }
    encoded.usize(schedule.jerk_report().elements.len())?;
    for element in &schedule.jerk_report().elements {
        push_jerk_element_report(&mut encoded, element)?;
    }
    encoded.real(schedule.total_path_length_mm())?;
    encoded.real(schedule.total_traversal_time_seconds())?;
    Ok(encoded.finish())
}

fn push_motion_error(
    encoded: &mut TranscriptEncoder,
    error: MotionError,
) -> ScheduleEvidenceResult<()> {
    match error {
        MotionError::AxisCount => encoded.u8(0)?,
        MotionError::Timing => encoded.u8(1)?,
        MotionError::State => encoded.u8(2)?,
        MotionError::SegmentFlags => encoded.u8(3)?,
        MotionError::SegmentOrder => encoded.u8(4)?,
        MotionError::EmptySegment => encoded.u8(5)?,
        MotionError::PositionOverflow { axis } => {
            encoded.u8(6)?;
            encoded.usize(axis)?;
        }
        MotionError::EpochOverflow => encoded.u8(7)?,
        MotionError::Arithmetic => encoded.u8(8)?,
        MotionError::OutputGrid {
            cycle,
            quantum_cycles,
        } => {
            encoded.u8(9)?;
            encoded.u64(cycle)?;
            encoded.u32(quantum_cycles)?;
        }
        MotionError::Rate { axis } => {
            encoded.u8(10)?;
            encoded.usize(axis)?;
        }
        MotionError::PulseBoundary { axis } => {
            encoded.u8(11)?;
            encoded.usize(axis)?;
        }
        MotionError::PulseLow { axis } => {
            encoded.u8(12)?;
            encoded.usize(axis)?;
        }
        MotionError::DirectionSetup { axis } => {
            encoded.u8(13)?;
            encoded.usize(axis)?;
        }
        MotionError::DirectionHold { axis } => {
            encoded.u8(14)?;
            encoded.usize(axis)?;
        }
        MotionError::EnableSetup { axis } => {
            encoded.u8(15)?;
            encoded.usize(axis)?;
        }
        MotionError::EnableHold { axis } => {
            encoded.u8(16)?;
            encoded.usize(axis)?;
        }
        MotionError::ConfigurationAxisCount {
            configured,
            expected,
        } => {
            encoded.u8(17)?;
            encoded.u8(configured)?;
            encoded.usize(expected)?;
        }
        MotionError::ConfigurationAxisLayout { axis } => {
            encoded.u8(18)?;
            encoded.usize(axis)?;
        }
        MotionError::Deadline {
            scheduled,
            observed,
            maximum_lateness_cycles,
        } => {
            encoded.u8(19)?;
            encoded.u64(scheduled.0)?;
            encoded.u64(observed.0)?;
            encoded.u32(maximum_lateness_cycles)?;
        }
        MotionError::OutputInvariant => encoded.u8(20)?,
    }
    Ok(())
}

fn push_optional_motion_error(
    encoded: &mut TranscriptEncoder,
    error: Option<MotionError>,
) -> ScheduleEvidenceResult<()> {
    match error {
        Some(error) => {
            encoded.u8(1)?;
            push_motion_error(encoded, error)
        }
        None => encoded.u8(0),
    }
}

fn push_timer_lattice_report(
    encoded: &mut TranscriptEncoder,
    report: &TimerLatticeScheduleReport2,
) -> ScheduleEvidenceResult<()> {
    encoded.u32(report.selected_factor_numerator())?;
    encoded.u32(report.selected_factor_denominator())?;
    encoded.u32(report.maximum_factor_numerator())?;
    encoded.rational(&report.selected_factor())?;
    encoded.u32(report.candidate_replays())?;
    push_optional_motion_error(encoded, report.unit_factor_rejection())?;
    push_optional_motion_error(encoded, report.predecessor_rejection())?;
    encoded.real(report.ideal_total_time_seconds())?;
    encoded.real(report.scheduled_total_time_seconds())?;
    encoded.real(report.maximum_cumulative_delay_seconds())?;
    encoded.real(report.maximum_segment_extension_seconds())?;
    encoded.real(report.maximum_output_grid_padding_seconds())
}

fn push_lowering_evidence(
    encoded: &mut TranscriptEncoder,
    evidence: &ScheduledLoweringEvidence2,
) -> ScheduleEvidenceResult<()> {
    encoded.rational(evidence.maximum_source_to_motion_error_mm_exact())?;
    encoded.real(evidence.maximum_source_to_motion_error_mm())?;
    encoded.rational(evidence.requested_interpolation_error_mm_exact())?;
    encoded.real(evidence.requested_interpolation_error_mm())?;
    encoded.real(evidence.maximum_chord_interpolation_error_mm())?;
    for error in evidence.maximum_axis_quantization_error_mm() {
        encoded.real(error)?;
    }
    encoded.real(evidence.maximum_position_quantization_error_mm())?;
    encoded.real(evidence.maximum_step_event_tracking_error_mm())?;
    encoded.real(evidence.maximum_curve_to_canonical_error_mm())?;
    let limits = evidence.lowering_limits();
    encoded.usize(limits.maximum_points())?;
    let timer = limits.timer_dilation_policy();
    encoded.u32(timer.factor_denominator())?;
    encoded.u32(timer.maximum_factor_numerator())?;
    push_timer_lattice_report(encoded, evidence.timer_lattice_schedule())
}

fn push_execution_segment(
    encoded: &mut TranscriptEncoder,
    segment: &ExecutionSegment<2>,
) -> ScheduleEvidenceResult<()> {
    encoded.u64(segment.start_tick.0)?;
    encoded.u64(segment.end_tick.0)?;
    for delta in segment.delta_steps {
        encoded.i64(delta)?;
    }
    encoded.u32(segment.flags)
}

fn push_preflight(
    encoded: &mut TranscriptEncoder,
    preflight: StepperPreflightSummary<2>,
) -> ScheduleEvidenceResult<()> {
    encoded.u64(preflight.end_tick.0)?;
    for position in preflight.position {
        encoded.i64(position)?;
    }
    for count in preflight.emitted_steps {
        encoded.u64(count)?;
    }
    encoded.u32(preflight.segment_count)?;
    encoded.u64(preflight.earliest_finish_cycle.0)
}

fn encode_lowering_transcript(
    program: &CanonicalScheduledProgram2,
) -> ScheduleEvidenceResult<(Digest, u64)> {
    if program.points().len() != program.segments().len().saturating_add(1)
        || program.points().first().map(|point| point.tick().get()) != Some(0)
        || program.points().last().map(|point| point.tick().get())
            != Some(program.executor_preflight().end_tick.0)
        || program.executor_preflight().segment_count as usize != program.segments().len()
    {
        return Err(ScheduleEvidenceError::LoweringMismatch);
    }

    let report = program.evidence().timer_lattice_schedule();
    let lowering_limits = program.evidence().lowering_limits();
    let timer_policy = lowering_limits.timer_dilation_policy();
    if report.selected_factor_denominator() != timer_policy.factor_denominator()
        || report.maximum_factor_numerator() != timer_policy.maximum_factor_numerator()
        || report.selected_factor_numerator() < report.selected_factor_denominator()
        || report.selected_factor_numerator() > report.maximum_factor_numerator()
    {
        return Err(ScheduleEvidenceError::LoweringMismatch);
    }

    let mut encoded = TranscriptEncoder::new();
    encoded.raw(&LOWERING_MAGIC)?;
    encoded.u16(LOWERING_VERSION)?;
    encoded.u16(EXACT_REAL_FORMAT_VERSION)?;
    encoded.raw(&program.configuration_digest().0)?;
    encoded.raw(&program.capability_digest().0)?;
    encoded.u64(program.timer_ticks_per_second())?;
    encoded.u32(program.output_quantum_cycles())?;

    let budget = program.resolution_budget();
    encoded.raw(&budget.configuration_digest().0)?;
    encoded.raw(&budget.capability_digest().0)?;
    encoded.rational(budget.requested_total_error_mm_exact())?;
    encoded.rational(budget.source_curve_allocation_mm_exact())?;
    encoded.rational(budget.controller_interpolation_allocation_mm_exact())?;
    for value in [
        budget.requested_total_error_mm(),
        budget.source_curve_allocation_mm(),
        budget.controller_interpolation_allocation_mm(),
        budget.endpoint_quantization_error_mm(),
        budget.step_event_tracking_error_mm(),
        budget.command_lattice_error_mm(),
        budget.calibration_error_mm(),
        budget.following_error_mm(),
        budget.output_grid_position_error_mm(),
        budget.required_total_error_mm(),
    ] {
        encoded.real(value)?;
    }
    push_lowering_evidence(&mut encoded, program.evidence())?;

    encoded.usize(program.points().len())?;
    for point in program.points() {
        encoded.usize(point.source_element())?;
        encoded.usize(point.motion_element())?;
        encoded.usize(point.phase_index())?;
        encoded.usize(point.subdivision_index())?;
        encoded.real(point.exact_point_mm().x())?;
        encoded.real(point.exact_point_mm().y())?;
        encoded.real(point.ideal_time_seconds())?;
        for step in point.steps() {
            encoded.i64(step.get())?;
        }
        encoded.u64(point.tick().get())?;
    }

    encoded.usize(program.segments().len())?;
    for segment in program.segments() {
        push_execution_segment(&mut encoded, segment)?;
    }
    push_preflight(&mut encoded, program.executor_preflight())?;
    Ok(encoded.finish())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExactPathDomain {
    Source,
    Metric,
}

fn encode_exact_path(
    path: &CurvePath2,
    domain: ExactPathDomain,
) -> ScheduleEvidenceResult<Vec<u8>> {
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(160)
        .map_err(|_| ScheduleEvidenceError::AllocationOverflow)?;
    let (magic, version) = match domain {
        ExactPathDomain::Source => (SOURCE_MAGIC, SOURCE_VERSION),
        ExactPathDomain::Metric => (METRIC_MAGIC, METRIC_VERSION),
    };
    encoded.extend_from_slice(&magic);
    push_u16(&mut encoded, version);
    push_usize(&mut encoded, path.curves().len())?;
    for (element, curve) in path.curves().iter().enumerate() {
        match curve.geometry() {
            CurveGeometry2::Line(line) => {
                encoded.push(1);
                push_point(&mut encoded, line.start(), element)?;
                push_point(&mut encoded, line.end(), element)?;
            }
            CurveGeometry2::CircularArc(arc) => {
                encoded.push(2);
                push_point(&mut encoded, arc.start(), element)?;
                push_point(&mut encoded, arc.end(), element)?;
                push_point(&mut encoded, arc.center(), element)?;
                push_real_rational(&mut encoded, arc.radius_squared_ref(), element)?;
                encoded.push(u8::from(arc.is_clockwise()));
                match arc.bulge() {
                    Some(bulge) => {
                        encoded.push(1);
                        push_real_rational(&mut encoded, bulge, element)?;
                    }
                    None => encoded.push(0),
                }
            }
            CurveGeometry2::CubicBezier(cubic) if domain == ExactPathDomain::Source => {
                encoded.push(3);
                push_point(&mut encoded, cubic.start(), element)?;
                push_point(&mut encoded, cubic.control1(), element)?;
                push_point(&mut encoded, cubic.control2(), element)?;
                push_point(&mut encoded, cubic.end(), element)?;
            }
            _ => return Err(ScheduleEvidenceError::UnsupportedSource { element }),
        }
    }
    Ok(encoded)
}

fn encode_source_approximation(
    source: &CurvePath2,
    metric_path: &CertifiedMetricPath2,
) -> ScheduleEvidenceResult<Vec<u8>> {
    if metric_path.spans().len() != source.curves().len() {
        return Err(ScheduleEvidenceError::SourceApproximationMismatch);
    }

    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(160)
        .map_err(|_| ScheduleEvidenceError::AllocationOverflow)?;
    encoded.extend_from_slice(&APPROXIMATION_MAGIC);
    push_u16(&mut encoded, APPROXIMATION_VERSION);
    push_usize(&mut encoded, source.curves().len())?;
    push_usize(&mut encoded, metric_path.path().curves().len())?;
    push_rational(&mut encoded, metric_path.maximum_source_error_mm_exact())?;
    push_usize(&mut encoded, metric_path.spans().len())?;

    let mut motion_cursor = 0_usize;
    let mut maximum_span_error = Rational::zero();
    for (expected_source_element, span) in metric_path.spans().iter().enumerate() {
        let expected_family = source.curves()[expected_source_element].geometry().family();
        if span.source_element() != expected_source_element
            || span.source_family() != expected_family
            || span.motion_element_start() != motion_cursor
            || span.motion_element_count() == 0
        {
            return Err(ScheduleEvidenceError::SourceApproximationMismatch);
        }
        let motion_end = motion_cursor
            .checked_add(span.motion_element_count())
            .ok_or(ScheduleEvidenceError::CounterOverflow)?;
        if motion_end > metric_path.path().curves().len()
            || (motion_cursor..motion_end).any(|motion_element| {
                metric_path.source_element_for_motion(motion_element)
                    != Some(expected_source_element)
            })
        {
            return Err(ScheduleEvidenceError::SourceApproximationMismatch);
        }
        if span.maximum_error_mm_exact() > &maximum_span_error {
            maximum_span_error = span.maximum_error_mm_exact().clone();
        }

        push_usize(&mut encoded, span.source_element())?;
        encoded.push(curve_family_tag(span.source_family()));
        push_usize(&mut encoded, span.motion_element_start())?;
        push_usize(&mut encoded, span.motion_element_count())?;
        push_rational(&mut encoded, span.maximum_error_mm_exact())?;
        push_usize(&mut encoded, span.maximum_subdivision_depth())?;
        motion_cursor = motion_end;
    }
    if motion_cursor != metric_path.path().curves().len()
        || maximum_span_error != *metric_path.maximum_source_error_mm_exact()
    {
        return Err(ScheduleEvidenceError::SourceApproximationMismatch);
    }
    Ok(encoded)
}

const fn curve_family_tag(family: CurveFamily2) -> u8 {
    match family {
        CurveFamily2::Line => 1,
        CurveFamily2::CircularArc => 2,
        CurveFamily2::QuadraticBezier => 3,
        CurveFamily2::CubicBezier => 4,
        CurveFamily2::RationalQuadraticBezier => 5,
        CurveFamily2::RationalBezier => 6,
        CurveFamily2::PolynomialBSpline => 7,
        CurveFamily2::Nurbs => 8,
    }
}

fn push_point(
    encoded: &mut Vec<u8>,
    point: &hypercurve::Point2,
    element: usize,
) -> ScheduleEvidenceResult<()> {
    push_real_rational(encoded, point.x(), element)?;
    push_real_rational(encoded, point.y(), element)
}

fn push_real_rational(
    encoded: &mut Vec<u8>,
    value: &Real,
    element: usize,
) -> ScheduleEvidenceResult<()> {
    let rational = value
        .exact_rational_ref()
        .ok_or(ScheduleEvidenceError::NonRationalSource { element })?;
    push_rational(encoded, rational)
}

fn push_rational(encoded: &mut Vec<u8>, value: &Rational) -> ScheduleEvidenceResult<()> {
    encoded.push(if value.is_zero() {
        0
    } else if value.is_negative() {
        2
    } else {
        1
    });
    let numerator = value.numerator().to_bytes_le();
    let denominator = value.denominator().to_bytes_le();
    push_bytes(encoded, &numerator)?;
    push_bytes(encoded, &denominator)
}

fn push_bytes(encoded: &mut Vec<u8>, bytes: &[u8]) -> ScheduleEvidenceResult<()> {
    push_usize(encoded, bytes.len())?;
    encoded.extend_from_slice(bytes);
    Ok(())
}

fn push_usize(encoded: &mut Vec<u8>, value: usize) -> ScheduleEvidenceResult<()> {
    push_u32(
        encoded,
        u32::try_from(value).map_err(|_| ScheduleEvidenceError::CounterOverflow)?,
    );
    Ok(())
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

fn push_i64(encoded: &mut Vec<u8>, value: i64) {
    encoded.extend_from_slice(&value.to_le_bytes());
}

/// Failure to bind or replay canonical exact-schedule evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleEvidenceError {
    /// Planner source, metric path, identities, or retained proofs disagreed.
    PlannerMismatch,
    /// Canonical points, segments, limits, timer report, or preflight disagreed.
    LoweringMismatch,
    /// Program and target partition identities differed.
    IdentityMismatch,
    /// A required program boundary was absent.
    EmptyProgram,
    /// Program, executor, and partition terminal facts differed.
    TerminalMismatch,
    /// A retained source or metric path contained an unsupported exact family.
    UnsupportedSource {
        /// Zero-based retained path element.
        element: usize,
    },
    /// Exact path evidence requires exact-rational primitive parameters.
    NonRationalSource {
        /// Zero-based retained path element.
        element: usize,
    },
    /// A canonical length/count did not fit its V3 field.
    CounterOverflow,
    /// Source spans, motion provenance, and retained paths did not agree.
    SourceApproximationMismatch,
    /// Transcript storage could not be reserved.
    AllocationOverflow,
    /// Hyperreal's exact structural representation could not be serialized.
    ExactRealEncoding,
    /// One exact-real structural representation exceeded its explicit bound.
    ExactRealEncodingTooLarge,
    /// One planner or lowering subtranscript exceeded its explicit bound.
    SubtranscriptTooLarge,
    /// Rebuilt bytes or their SHA-256 identity differed.
    ReplayMismatch,
    /// Supplied evidence bytes did not match their declared SHA-256.
    DigestMismatch,
}

impl fmt::Display for ScheduleEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlannerMismatch => formatter
                .write_str("planner source, metric path, policy, or certification disagrees"),
            Self::LoweringMismatch => formatter
                .write_str("canonical lowering, timer policy, or executor replay disagrees"),
            Self::IdentityMismatch => {
                formatter.write_str("program and partition identities differ")
            }
            Self::EmptyProgram => formatter.write_str("scheduled program is empty"),
            Self::TerminalMismatch => {
                formatter.write_str("program, executor, and partition terminal facts differ")
            }
            Self::UnsupportedSource { element } => write!(
                formatter,
                "exact path element {element} is not supported by canonical evidence"
            ),
            Self::NonRationalSource { element } => write!(
                formatter,
                "source element {element} contains a non-rational exact parameter"
            ),
            Self::CounterOverflow => formatter.write_str("evidence counter does not fit V3"),
            Self::SourceApproximationMismatch => formatter
                .write_str("source-to-motion approximation evidence is internally inconsistent"),
            Self::AllocationOverflow => formatter.write_str("evidence storage reservation failed"),
            Self::ExactRealEncoding => {
                formatter.write_str("exact Real structural serialization failed")
            }
            Self::ExactRealEncodingTooLarge => formatter
                .write_str("exact Real structural serialization exceeded its bounded field"),
            Self::SubtranscriptTooLarge => {
                formatter.write_str("canonical planner or lowering subtranscript exceeded 64 MiB")
            }
            Self::ReplayMismatch => formatter.write_str("canonical evidence replay differed"),
            Self::DigestMismatch => formatter.write_str("canonical evidence digest differed"),
        }
    }
}

impl StdError for ScheduleEvidenceError {}
