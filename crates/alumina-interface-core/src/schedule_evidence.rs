//! Canonical content evidence for exact-source scheduled machine partitions.

use std::error::Error as StdError;
use std::fmt;

use alumina_protocol::Digest;
use alumina_storage::sha256;
use hypercurve::{CurveGeometry2, CurvePath2};
use hyperreal::{Rational, Real};

use crate::motion_schedule::CanonicalScheduledProgram2;
use crate::partition::CanonicalMachinePartition2;

const EVIDENCE_MAGIC: [u8; 8] = *b"ALMEVD01";
const EVIDENCE_VERSION: u16 = 1;
const SOURCE_VERSION: u16 = 1;
const AXES: u16 = 2;
const CERTIFIED_LOOKAHEAD: u32 = 1 << 0;
const CERTIFIED_JERK_REPLAY: u32 = 1 << 1;
const CERTIFIED_INTERPOLATION: u32 = 1 << 2;
const CERTIFIED_EXECUTOR_PREFLIGHT: u32 = 1 << 3;
const CERTIFIED_PARTITION_STREAM_REPLAY: u32 = 1 << 4;
const CERTIFICATION_FLAGS: u32 = CERTIFIED_LOOKAHEAD
    | CERTIFIED_JERK_REPLAY
    | CERTIFIED_INTERPOLATION
    | CERTIFIED_EXECUTOR_PREFLIGHT
    | CERTIFIED_PARTITION_STREAM_REPLAY;

/// Result type for canonical schedule-evidence construction and replay.
pub type ScheduleEvidenceResult<T> = Result<T, ScheduleEvidenceError>;

/// Immutable deterministic transcript binding exact source, machine policy,
/// canonical IR, and content-addressed cached partition identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalScheduleEvidence2 {
    encoded: Vec<u8>,
    digest: Digest,
    source_digest: Digest,
}

impl CanonicalScheduleEvidence2 {
    /// Complete canonical V1 transcript bytes.
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    /// SHA-256 identity of the complete evidence transcript.
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// SHA-256 identity of the exact rational line/arc source transcript.
    pub const fn source_digest(&self) -> Digest {
        self.source_digest
    }
}

/// Construct deterministic evidence only after identity and terminal replay
/// facts agree across the scheduled program and packaged partition.
pub fn build_canonical_schedule_evidence(
    program: &CanonicalScheduledProgram2,
    partition: &CanonicalMachinePartition2,
) -> ScheduleEvidenceResult<CanonicalScheduleEvidence2> {
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
    if partition.initial_position() != initial_position
        || partition.final_position() != final_position
        || partition.local_timer_hz() != program.timer_ticks_per_second()
        || partition.terminal_progress().end_tick != preflight.end_tick
        || preflight.position != final_position
        || usize::try_from(preflight.segment_count).ok() != Some(program.segments().len())
    {
        return Err(ScheduleEvidenceError::TerminalMismatch);
    }

    let source = encode_exact_source(program.source())?;
    let source_digest = sha256(&source).digest;
    let publication = partition.publication();
    let budget = program.resolution_budget();
    let requested_interpolation = program.evidence().requested_interpolation_error_mm_exact();

    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(320)
        .map_err(|_| ScheduleEvidenceError::AllocationOverflow)?;
    encoded.extend_from_slice(&EVIDENCE_MAGIC);
    push_u16(&mut encoded, EVIDENCE_VERSION);
    push_u16(&mut encoded, AXES);
    push_u32(&mut encoded, CERTIFICATION_FLAGS);
    encoded.extend_from_slice(&program.configuration_digest().0);
    encoded.extend_from_slice(&program.capability_digest().0);
    encoded.extend_from_slice(&source_digest.0);
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
        requested_interpolation,
    ] {
        push_rational(&mut encoded, rational)?;
    }

    let digest = sha256(&encoded).digest;
    Ok(CanonicalScheduleEvidence2 {
        encoded,
        digest,
        source_digest,
    })
}

/// Rebuild the canonical transcript and require byte-for-byte identity. This
/// reruns exact source serialization and all cross-artifact consistency gates;
/// it does not trust fields parsed from the supplied evidence.
pub fn replay_canonical_schedule_evidence(
    evidence: &CanonicalScheduleEvidence2,
    program: &CanonicalScheduledProgram2,
    partition: &CanonicalMachinePartition2,
) -> ScheduleEvidenceResult<()> {
    verify_canonical_schedule_evidence_bytes(
        evidence.encoded(),
        evidence.digest,
        program,
        partition,
    )?;
    let rebuilt = build_canonical_schedule_evidence(program, partition)?;
    (rebuilt.source_digest == evidence.source_digest)
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
    program: &CanonicalScheduledProgram2,
    partition: &CanonicalMachinePartition2,
) -> ScheduleEvidenceResult<()> {
    if sha256(encoded).digest != expected_digest {
        return Err(ScheduleEvidenceError::DigestMismatch);
    }
    let rebuilt = build_canonical_schedule_evidence(program, partition)?;
    if encoded != rebuilt.encoded() || expected_digest != rebuilt.digest() {
        return Err(ScheduleEvidenceError::ReplayMismatch);
    }
    Ok(())
}

fn encode_exact_source(source: &CurvePath2) -> ScheduleEvidenceResult<Vec<u8>> {
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(128)
        .map_err(|_| ScheduleEvidenceError::AllocationOverflow)?;
    push_u16(&mut encoded, SOURCE_VERSION);
    push_usize(&mut encoded, source.curves().len())?;
    for (element, curve) in source.curves().iter().enumerate() {
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
            _ => return Err(ScheduleEvidenceError::UnsupportedSource { element }),
        }
    }
    Ok(encoded)
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
    /// Program and target partition identities differed.
    IdentityMismatch,
    /// A required program boundary was absent.
    EmptyProgram,
    /// Program, executor, and partition terminal facts differed.
    TerminalMismatch,
    /// V1 source evidence supports retained lines and circular arcs only.
    UnsupportedSource {
        /// Zero-based retained path element.
        element: usize,
    },
    /// V1 source evidence requires exact-rational primitive parameters.
    NonRationalSource {
        /// Zero-based retained path element.
        element: usize,
    },
    /// A canonical length/count did not fit its V1 field.
    CounterOverflow,
    /// Transcript storage could not be reserved.
    AllocationOverflow,
    /// Rebuilt bytes or their SHA-256 identity differed.
    ReplayMismatch,
    /// Supplied evidence bytes did not match their declared SHA-256.
    DigestMismatch,
}

impl fmt::Display for ScheduleEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdentityMismatch => {
                formatter.write_str("program and partition identities differ")
            }
            Self::EmptyProgram => formatter.write_str("scheduled program is empty"),
            Self::TerminalMismatch => {
                formatter.write_str("program, executor, and partition terminal facts differ")
            }
            Self::UnsupportedSource { element } => write!(
                formatter,
                "source element {element} is not a V1 retained line or circular arc"
            ),
            Self::NonRationalSource { element } => write!(
                formatter,
                "source element {element} contains a non-rational exact parameter"
            ),
            Self::CounterOverflow => formatter.write_str("evidence counter does not fit V1"),
            Self::AllocationOverflow => formatter.write_str("evidence storage reservation failed"),
            Self::ReplayMismatch => formatter.write_str("canonical evidence replay differed"),
            Self::DigestMismatch => formatter.write_str("canonical evidence digest differed"),
        }
    }
}

impl StdError for ScheduleEvidenceError {}
