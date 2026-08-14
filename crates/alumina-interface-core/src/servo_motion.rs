//! Exact browser/WASM servo trajectories lowered onto the firmware lattices.
//!
//! The browser remains the numerical authority. It supplies exact Hyperreal
//! cubic Newton-forward recurrences in configured-axis and normalized
//! feed-forward units. This module performs the sole Q31.32/Q2.30 projection,
//! retains certified error evidence, splits encoded recurrences at discrete
//! extrema and hardware horizons, and independently replays every resulting
//! firmware record. No floating-point value crosses this boundary.

use std::error::Error as StdError;
use std::fmt;

use alumina_machine_ir::{
    MAX_SERVO_EXECUTION_AXES, SERVO_FINITE_DIFFERENCE_POSITION_FRACTION_BITS,
    SERVO_FINITE_DIFFERENCE_Q30_FRACTION_BITS, ServoFiniteDifferenceAxis,
    ServoFiniteDifferenceError, ServoFiniteDifferenceSegment, ServoFiniteDifferenceState,
    ServoQ30FiniteDifferenceAxis, ServoSignal, StreamTick,
};
use alumina_motion::{CachedServoAdmissionProfile, CachedServoSetpointError};
use alumina_protocol::Digest;
use hyperreal::{Rational, Real};

/// Result type for exact servo recurrence lowering.
pub type ServoMotionResult<T> = Result<T, ServoMotionError>;

const MINIMUM_COEFFICIENT_PRECISION_BITS: u16 = 40;
const MAXIMUM_COEFFICIENT_PRECISION_BITS: u16 = 512;

/// Exact cubic Newton-forward coefficients for one configured servo axis.
///
/// Each array is `[initial, first difference, second difference, third
/// difference]`. Position is expressed in configured-axis lattice units;
/// feed-forward values are normalized so exact one maps to Q2.30 one.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactServoAxisRecurrence {
    position: [Real; 4],
    velocity_feed_forward: [Real; 4],
    quadrature_current_feed_forward: [Real; 4],
}

impl ExactServoAxisRecurrence {
    /// Construct one exact source recurrence without approximation.
    pub const fn new(
        position: [Real; 4],
        velocity_feed_forward: [Real; 4],
        quadrature_current_feed_forward: [Real; 4],
    ) -> Self {
        Self {
            position,
            velocity_feed_forward,
            quadrature_current_feed_forward,
        }
    }

    /// Borrow the exact configured-axis position recurrence.
    pub const fn position(&self) -> &[Real; 4] {
        &self.position
    }

    /// Borrow the exact normalized velocity feed-forward recurrence.
    pub const fn velocity_feed_forward(&self) -> &[Real; 4] {
        &self.velocity_feed_forward
    }

    /// Borrow the exact normalized quadrature-current recurrence.
    pub const fn quadrature_current_feed_forward(&self) -> &[Real; 4] {
        &self.quadrature_current_feed_forward
    }
}

/// One exact, fixed-cadence cubic source span for every simultaneous axis.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactServoCubicSpan<const AXES: usize> {
    update_count: u32,
    axes: [ExactServoAxisRecurrence; AXES],
}

impl<const AXES: usize> ExactServoCubicSpan<AXES> {
    /// Construct a source span. Bounds and axis width are checked during lowering.
    pub const fn new(update_count: u32, axes: [ExactServoAxisRecurrence; AXES]) -> Self {
        Self { update_count, axes }
    }

    /// Exact number of half-open dense updates represented by this source span.
    pub const fn update_count(&self) -> u32 {
        self.update_count
    }

    /// Borrow every simultaneous exact axis recurrence.
    pub const fn axes(&self) -> &[ExactServoAxisRecurrence; AXES] {
        &self.axes
    }
}

/// Browser-owned allocation, refinement, and approximation policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServoFiniteDifferenceCompilePolicy {
    maximum_source_spans: usize,
    maximum_output_records: usize,
    maximum_examined_updates: u64,
    coefficient_precision_bits: u16,
    maximum_position_error: Rational,
    maximum_feed_forward_error: Rational,
}

impl ServoFiniteDifferenceCompilePolicy {
    /// Construct an explicit bounded compiler policy.
    pub fn try_new(
        maximum_source_spans: usize,
        maximum_output_records: usize,
        maximum_examined_updates: u64,
        coefficient_precision_bits: u16,
        maximum_position_error: Rational,
        maximum_feed_forward_error: Rational,
    ) -> ServoMotionResult<Self> {
        if maximum_source_spans == 0
            || maximum_output_records == 0
            || maximum_examined_updates == 0
            || !(MINIMUM_COEFFICIENT_PRECISION_BITS..=MAXIMUM_COEFFICIENT_PRECISION_BITS)
                .contains(&coefficient_precision_bits)
            || maximum_position_error <= Rational::zero()
            || maximum_feed_forward_error <= Rational::zero()
        {
            return Err(ServoMotionError::InvalidPolicy);
        }
        Ok(Self {
            maximum_source_spans,
            maximum_output_records,
            maximum_examined_updates,
            coefficient_precision_bits,
            maximum_position_error,
            maximum_feed_forward_error,
        })
    }

    /// A bounded interactive policy with 128-bit coefficient refinement.
    pub fn interactive(
        maximum_position_error: Rational,
        maximum_feed_forward_error: Rational,
    ) -> ServoMotionResult<Self> {
        Self::try_new(
            65_536,
            262_144,
            16_777_216,
            128,
            maximum_position_error,
            maximum_feed_forward_error,
        )
    }

    /// Maximum exact source spans retained by one compilation.
    pub const fn maximum_source_spans(&self) -> usize {
        self.maximum_source_spans
    }

    /// Maximum extrema- and horizon-split firmware records retained in memory.
    pub const fn maximum_output_records(&self) -> usize {
        self.maximum_output_records
    }

    /// Maximum dense updates examined while finding discrete extrema.
    pub const fn maximum_examined_updates(&self) -> u64 {
        self.maximum_examined_updates
    }

    /// Binary precision requested from Hyperreal for lattice projection.
    pub const fn coefficient_precision_bits(&self) -> u16 {
        self.coefficient_precision_bits
    }

    /// Maximum certified configured-axis error within one source span.
    pub const fn maximum_position_error(&self) -> &Rational {
        &self.maximum_position_error
    }

    /// Maximum certified normalized error for either feed-forward signal.
    pub const fn maximum_feed_forward_error(&self) -> &Rational {
        &self.maximum_feed_forward_error
    }
}

/// Certified projection of one exact coefficient onto a signed integer lattice.
#[derive(Clone, Debug, PartialEq)]
pub struct ServoCoefficientProjection {
    ideal: Real,
    scaled_interval: [Rational; 2],
    encoded_bits: i64,
    fractional_bits: u32,
    maximum_error: Rational,
    continuity_forced: bool,
}

impl ServoCoefficientProjection {
    /// Exact source coefficient before lattice projection.
    pub const fn ideal(&self) -> &Real {
        &self.ideal
    }

    /// Certified closed enclosure after multiplication by the lattice scale.
    pub const fn scaled_interval(&self) -> &[Rational; 2] {
        &self.scaled_interval
    }

    /// Canonical signed integer coefficient placed on the wire.
    pub const fn encoded_bits(&self) -> i64 {
        self.encoded_bits
    }

    /// Binary fractional width of the destination lattice.
    pub const fn fractional_bits(&self) -> u32 {
        self.fractional_bits
    }

    /// Conservative absolute error in unscaled configured-axis or normalized units.
    pub const fn maximum_error(&self) -> &Rational {
        &self.maximum_error
    }

    /// Whether the initial value was selected by exact stream continuity.
    pub const fn continuity_forced(&self) -> bool {
        self.continuity_forced
    }
}

/// Complete coefficient and recurrence-error evidence for one source axis.
#[derive(Clone, Debug, PartialEq)]
pub struct ServoAxisProjectionEvidence {
    position: [ServoCoefficientProjection; 4],
    velocity_feed_forward: [ServoCoefficientProjection; 4],
    quadrature_current_feed_forward: [ServoCoefficientProjection; 4],
    maximum_position_error: Rational,
    maximum_velocity_feed_forward_error: Rational,
    maximum_quadrature_current_feed_forward_error: Rational,
}

impl ServoAxisProjectionEvidence {
    /// Q31.32 position coefficient projections.
    pub const fn position(&self) -> &[ServoCoefficientProjection; 4] {
        &self.position
    }

    /// Q2.30 velocity feed-forward coefficient projections.
    pub const fn velocity_feed_forward(&self) -> &[ServoCoefficientProjection; 4] {
        &self.velocity_feed_forward
    }

    /// Q2.30 quadrature-current coefficient projections.
    pub const fn quadrature_current_feed_forward(&self) -> &[ServoCoefficientProjection; 4] {
        &self.quadrature_current_feed_forward
    }

    /// Conservative configured-axis error over the complete source span.
    pub const fn maximum_position_error(&self) -> &Rational {
        &self.maximum_position_error
    }

    /// Conservative normalized velocity error over the complete source span.
    pub const fn maximum_velocity_feed_forward_error(&self) -> &Rational {
        &self.maximum_velocity_feed_forward_error
    }

    /// Conservative normalized quadrature-current error over the complete span.
    pub const fn maximum_quadrature_current_feed_forward_error(&self) -> &Rational {
        &self.maximum_quadrature_current_feed_forward_error
    }
}

/// Projection and deterministic splitting evidence for one exact source span.
#[derive(Clone, Debug, PartialEq)]
pub struct ServoSpanProjectionEvidence<const AXES: usize> {
    source_span_index: usize,
    update_count: u32,
    first_output_record: usize,
    output_record_count: usize,
    axes: [ServoAxisProjectionEvidence; AXES],
}

impl<const AXES: usize> ServoSpanProjectionEvidence<AXES> {
    /// Zero-based source span identity.
    pub const fn source_span_index(&self) -> usize {
        self.source_span_index
    }

    /// Exact dense update count in the unsplit source recurrence.
    pub const fn update_count(&self) -> u32 {
        self.update_count
    }

    /// Index of the first canonical firmware record produced by this span.
    pub const fn first_output_record(&self) -> usize {
        self.first_output_record
    }

    /// Number of canonical records after extrema and horizon splitting.
    pub const fn output_record_count(&self) -> usize {
        self.output_record_count
    }

    /// Per-axis coefficient and error certificates.
    pub const fn axes(&self) -> &[ServoAxisProjectionEvidence; AXES] {
        &self.axes
    }
}

/// Authoritative exact browser/WASM servo machine program.
#[derive(Clone, Debug)]
pub struct CanonicalServoFiniteDifferenceProgram<const AXES: usize> {
    capability_digest: Digest,
    configuration_digest: Digest,
    timer_ticks_per_second: u64,
    admission: CachedServoAdmissionProfile<AXES>,
    initial_position: [i64; AXES],
    final_state: ServoFiniteDifferenceState<AXES>,
    records: Vec<ServoFiniteDifferenceSegment<AXES>>,
    total_update_count: u64,
    evidence: Vec<ServoSpanProjectionEvidence<AXES>>,
}

impl<const AXES: usize> CanonicalServoFiniteDifferenceProgram<AXES> {
    /// Immutable board capability identity used during compilation.
    pub const fn capability_digest(&self) -> Digest {
        self.capability_digest
    }

    /// Complete active FOC configuration identity.
    pub const fn configuration_digest(&self) -> Digest {
        self.configuration_digest
    }

    /// Exact device timer frequency used by stream-relative ticks.
    pub const fn timer_ticks_per_second(&self) -> u64 {
        self.timer_ticks_per_second
    }

    /// Configuration-derived admission and setpoint authority.
    pub const fn admission(&self) -> CachedServoAdmissionProfile<AXES> {
        self.admission
    }

    /// Absolute initial configured-axis position in Q31.32 bits.
    pub const fn initial_position(&self) -> [i64; AXES] {
        self.initial_position
    }

    /// Exact terminal continuation state; both feed-forward vectors are zero.
    pub const fn final_state(&self) -> ServoFiniteDifferenceState<AXES> {
        self.final_state
    }

    /// Canonical extrema- and hardware-horizon-split firmware records.
    pub fn records(&self) -> &[ServoFiniteDifferenceSegment<AXES>] {
        &self.records
    }

    /// Number of half-open physical setpoints before the terminal at-rest hold.
    pub const fn total_update_count(&self) -> u64 {
        self.total_update_count
    }

    /// Exact projection and splitting evidence in source order.
    pub fn evidence(&self) -> &[ServoSpanProjectionEvidence<AXES>] {
        &self.evidence
    }
}

/// Project, split, and independently validate exact servo recurrences.
pub fn lower_exact_servo_recurrences<const AXES: usize>(
    spans: &[ExactServoCubicSpan<AXES>],
    timer_ticks_per_second: u64,
    capability_digest: Digest,
    admission: CachedServoAdmissionProfile<AXES>,
    policy: ServoFiniteDifferenceCompilePolicy,
) -> ServoMotionResult<CanonicalServoFiniteDifferenceProgram<AXES>> {
    if AXES == 0 || AXES > MAX_SERVO_EXECUTION_AXES {
        return Err(ServoMotionError::AxisCount);
    }
    admission
        .setpoints
        .validate::<AXES>()
        .map_err(ServoMotionError::SetpointProfile)?;
    if spans.is_empty() {
        return Err(ServoMotionError::EmptyProgram);
    }
    if spans.len() > policy.maximum_source_spans {
        return Err(ServoMotionError::SourceSpanLimit);
    }
    if timer_ticks_per_second == 0 || capability_digest.is_zero() {
        return Err(ServoMotionError::MissingIdentity);
    }
    let limits = admission.limits;
    let period = limits.segment.required_update_period_ticks;
    if period != admission.setpoints.update_period_ticks
        || limits.maximum_block_ticks == 0
        || limits.segment.maximum_segment_ticks == 0
        || limits.segment.maximum_update_count == 0
    {
        return Err(ServoMotionError::AdmissionProfile);
    }
    let maximum_record_updates = limits
        .segment
        .maximum_update_count
        .min(
            u32::try_from(limits.segment.maximum_segment_ticks / u64::from(period))
                .unwrap_or(u32::MAX),
        )
        .min(u32::try_from(limits.maximum_block_ticks / u64::from(period)).unwrap_or(u32::MAX));
    if maximum_record_updates == 0 {
        return Err(ServoMotionError::AdmissionProfile);
    }

    let mut records = Vec::new();
    let mut evidence = Vec::new();
    let mut continuation = None;
    let mut expected_state = ServoFiniteDifferenceState::at_rest([0; AXES]);
    let mut initial_position = [0_i64; AXES];
    let mut next_tick = StreamTick(0);
    let mut total_updates = 0_u64;

    for (span_index, span) in spans.iter().enumerate() {
        if span.update_count == 0 {
            return Err(ServoMotionError::UpdateCount { span: span_index });
        }
        total_updates = total_updates
            .checked_add(u64::from(span.update_count))
            .ok_or(ServoMotionError::Arithmetic {
                domain: "total servo update count",
            })?;
        if total_updates > policy.maximum_examined_updates {
            return Err(ServoMotionError::ExaminedUpdateLimit);
        }
        // Command zero is reserved and the executor appends one terminal hold.
        if total_updates >= u64::from(u32::MAX) {
            return Err(ServoMotionError::CommandCountOverflow);
        }

        let mut encoded_axes = [ServoFiniteDifferenceAxis::default(); AXES];
        let mut axis_evidence = Vec::new();
        axis_evidence
            .try_reserve_exact(AXES)
            .map_err(|_| ServoMotionError::AllocationOverflow)?;
        for (axis, encoded_axis) in encoded_axes.iter_mut().enumerate() {
            let forced = continuation.map(|state: ServoFiniteDifferenceState<AXES>| {
                [
                    state.position[axis],
                    i64::from(state.velocity_feed_forward[axis]),
                    i64::from(state.quadrature_current_feed_forward[axis]),
                ]
            });
            let position = project_coefficients(
                &span.axes[axis].position,
                SERVO_FINITE_DIFFERENCE_POSITION_FRACTION_BITS,
                policy.coefficient_precision_bits,
                forced.map(|values| values[0]),
                span_index,
                axis,
                ServoSignal::Position,
            )?;
            let velocity = project_coefficients(
                &span.axes[axis].velocity_feed_forward,
                SERVO_FINITE_DIFFERENCE_Q30_FRACTION_BITS,
                policy.coefficient_precision_bits,
                forced.map(|values| values[1]),
                span_index,
                axis,
                ServoSignal::VelocityFeedForward,
            )?;
            let current = project_coefficients(
                &span.axes[axis].quadrature_current_feed_forward,
                SERVO_FINITE_DIFFERENCE_Q30_FRACTION_BITS,
                policy.coefficient_precision_bits,
                forced.map(|values| values[2]),
                span_index,
                axis,
                ServoSignal::QuadratureCurrentFeedForward,
            )?;
            let velocity_bits = projection_i32(
                &velocity,
                span_index,
                axis,
                ServoSignal::VelocityFeedForward,
            )?;
            let current_bits = projection_i32(
                &current,
                span_index,
                axis,
                ServoSignal::QuadratureCurrentFeedForward,
            )?;
            *encoded_axis = ServoFiniteDifferenceAxis {
                position: alumina_machine_ir::FiniteDifferenceAxis {
                    initial_position: position[0].encoded_bits,
                    first_difference: position[1].encoded_bits,
                    second_difference: position[2].encoded_bits,
                    third_difference: position[3].encoded_bits,
                },
                velocity_feed_forward: ServoQ30FiniteDifferenceAxis {
                    initial_value: velocity_bits[0],
                    first_difference: velocity_bits[1],
                    second_difference: velocity_bits[2],
                    third_difference: velocity_bits[3],
                },
                quadrature_current_feed_forward: ServoQ30FiniteDifferenceAxis {
                    initial_value: current_bits[0],
                    first_difference: current_bits[1],
                    second_difference: current_bits[2],
                    third_difference: current_bits[3],
                },
            };
            let maximum_position_error = recurrence_error_bound(&position, span.update_count)?;
            let maximum_velocity_feed_forward_error =
                recurrence_error_bound(&velocity, span.update_count)?;
            let maximum_quadrature_current_feed_forward_error =
                recurrence_error_bound(&current, span.update_count)?;
            if maximum_position_error > policy.maximum_position_error {
                return Err(ServoMotionError::ApproximationBudget {
                    span: span_index,
                    axis,
                    signal: ServoSignal::Position,
                });
            }
            if maximum_velocity_feed_forward_error > policy.maximum_feed_forward_error {
                return Err(ServoMotionError::ApproximationBudget {
                    span: span_index,
                    axis,
                    signal: ServoSignal::VelocityFeedForward,
                });
            }
            if maximum_quadrature_current_feed_forward_error > policy.maximum_feed_forward_error {
                return Err(ServoMotionError::ApproximationBudget {
                    span: span_index,
                    axis,
                    signal: ServoSignal::QuadratureCurrentFeedForward,
                });
            }
            axis_evidence.push(ServoAxisProjectionEvidence {
                position,
                velocity_feed_forward: velocity,
                quadrature_current_feed_forward: current,
                maximum_position_error,
                maximum_velocity_feed_forward_error,
                maximum_quadrature_current_feed_forward_error,
            });
        }
        let axis_evidence: [ServoAxisProjectionEvidence; AXES] =
            axis_evidence
                .try_into()
                .map_err(|_| ServoMotionError::Arithmetic {
                    domain: "servo axis evidence width",
                })?;
        let full = ServoFiniteDifferenceSegment {
            start_tick: StreamTick(0),
            end_tick: StreamTick(u64::from(period) * u64::from(span.update_count)),
            update_period_ticks: period,
            update_count: span.update_count,
            axes: encoded_axes,
            flags: 0,
        };
        let source_initial = full
            .state_at(0)
            .map_err(|error| ServoMotionError::Machine {
                span: span_index,
                error,
            })?;
        if span_index == 0 {
            if !source_initial.feed_forwards_are_zero() {
                return Err(ServoMotionError::InitialFeedForward);
            }
            initial_position = source_initial.position;
            expected_state = source_initial;
        } else if source_initial != expected_state {
            return Err(ServoMotionError::Continuity { span: span_index });
        }

        let first_output_record = records.len();
        let mut source_cursor = 0_u32;
        while source_cursor < span.update_count {
            if records.len() >= policy.maximum_output_records {
                return Err(ServoMotionError::OutputRecordLimit);
            }
            let source_end = find_record_end(
                &full,
                source_cursor,
                maximum_record_updates,
                limits.segment.maximum_position_delta_bits,
                limits
                    .segment
                    .maximum_absolute_position_first_difference_bits,
                limits.segment.maximum_absolute_velocity_feed_forward_bits,
                limits
                    .segment
                    .maximum_absolute_quadrature_current_feed_forward_bits,
                span_index,
            )?;
            let record_updates = source_end - source_cursor;
            let duration = u64::from(period)
                .checked_mul(u64::from(record_updates))
                .ok_or(ServoMotionError::Arithmetic {
                    domain: "servo record duration",
                })?;
            let end_tick = StreamTick(next_tick.0.checked_add(duration).ok_or(
                ServoMotionError::Arithmetic {
                    domain: "servo record end tick",
                },
            )?);
            let segment = ServoFiniteDifferenceSegment {
                start_tick: next_tick,
                end_tick,
                update_period_ticks: period,
                update_count: record_updates,
                axes: shift_axes(full.axes, source_cursor, span_index)?,
                flags: 0,
            };
            let summary = segment
                .validate(next_tick, expected_state, limits.segment)
                .map_err(|error| ServoMotionError::Machine {
                    span: span_index,
                    error,
                })?;
            expected_state = summary.terminal_state;
            next_tick = summary.end_tick;
            records.push(segment);
            source_cursor = source_end;
        }
        let source_terminal =
            full.state_at(span.update_count)
                .map_err(|error| ServoMotionError::Machine {
                    span: span_index,
                    error,
                })?;
        if expected_state != source_terminal {
            return Err(ServoMotionError::Continuity { span: span_index });
        }
        continuation = Some(source_terminal);
        evidence.push(ServoSpanProjectionEvidence {
            source_span_index: span_index,
            update_count: span.update_count,
            first_output_record,
            output_record_count: records.len() - first_output_record,
            axes: axis_evidence,
        });
    }

    if !expected_state.feed_forwards_are_zero() {
        return Err(ServoMotionError::TerminalFeedForward);
    }
    Ok(CanonicalServoFiniteDifferenceProgram {
        capability_digest,
        configuration_digest: admission.setpoints.configuration_digest,
        timer_ticks_per_second,
        admission,
        initial_position,
        final_state: expected_state,
        records,
        total_update_count: total_updates,
        evidence,
    })
}

fn project_coefficients(
    exact: &[Real; 4],
    fractional_bits: u32,
    precision_bits: u16,
    forced_initial: Option<i64>,
    span: usize,
    axis: usize,
    signal: ServoSignal,
) -> ServoMotionResult<[ServoCoefficientProjection; 4]> {
    Ok([
        project_coefficient(
            &exact[0],
            fractional_bits,
            precision_bits,
            forced_initial,
            span,
            axis,
            signal,
            0,
        )?,
        project_coefficient(
            &exact[1],
            fractional_bits,
            precision_bits,
            None,
            span,
            axis,
            signal,
            1,
        )?,
        project_coefficient(
            &exact[2],
            fractional_bits,
            precision_bits,
            None,
            span,
            axis,
            signal,
            2,
        )?,
        project_coefficient(
            &exact[3],
            fractional_bits,
            precision_bits,
            None,
            span,
            axis,
            signal,
            3,
        )?,
    ])
}

#[allow(
    clippy::too_many_arguments,
    reason = "projection evidence retains each exact source coordinate"
)]
fn project_coefficient(
    ideal: &Real,
    fractional_bits: u32,
    precision_bits: u16,
    forced: Option<i64>,
    span: usize,
    axis: usize,
    signal: ServoSignal,
    coefficient: usize,
) -> ServoMotionResult<ServoCoefficientProjection> {
    let scale = 1_i64
        .checked_shl(fractional_bits)
        .ok_or(ServoMotionError::Arithmetic {
            domain: "servo lattice scale",
        })?;
    let scaled = ideal * Real::from(scale);
    let interval = scaled
        .certified_dyadic_interval(-i32::from(precision_bits))
        .ok_or(ServoMotionError::ProjectionAborted {
            span,
            axis,
            signal,
            coefficient,
        })?;
    let encoded_bits = if let Some(forced) = forced {
        forced
    } else {
        let lower = round_rational_ties_even(&interval[0])?;
        let upper = round_rational_ties_even(&interval[1])?;
        if lower != upper {
            return Err(ServoMotionError::ProjectionUnresolved {
                span,
                axis,
                signal,
                coefficient,
                precision_bits,
            });
        }
        lower
    };
    let encoded = Rational::from(encoded_bits);
    let maximum_scaled_error = maximum_rational(
        absolute_rational(&interval[0] - &encoded),
        absolute_rational(&interval[1] - &encoded),
    );
    Ok(ServoCoefficientProjection {
        ideal: ideal.clone(),
        scaled_interval: interval,
        encoded_bits,
        fractional_bits,
        maximum_error: maximum_scaled_error / Rational::from(scale),
        continuity_forced: forced.is_some(),
    })
}

fn projection_i32(
    projected: &[ServoCoefficientProjection; 4],
    span: usize,
    axis: usize,
    signal: ServoSignal,
) -> ServoMotionResult<[i32; 4]> {
    Ok([
        i32::try_from(projected[0].encoded_bits).map_err(|_| {
            ServoMotionError::CoefficientRange {
                span,
                axis,
                signal,
                coefficient: 0,
            }
        })?,
        i32::try_from(projected[1].encoded_bits).map_err(|_| {
            ServoMotionError::CoefficientRange {
                span,
                axis,
                signal,
                coefficient: 1,
            }
        })?,
        i32::try_from(projected[2].encoded_bits).map_err(|_| {
            ServoMotionError::CoefficientRange {
                span,
                axis,
                signal,
                coefficient: 2,
            }
        })?,
        i32::try_from(projected[3].encoded_bits).map_err(|_| {
            ServoMotionError::CoefficientRange {
                span,
                axis,
                signal,
                coefficient: 3,
            }
        })?,
    ])
}

fn recurrence_error_bound(
    coefficients: &[ServoCoefficientProjection; 4],
    updates: u32,
) -> ServoMotionResult<Rational> {
    let n = Rational::from(u64::from(updates));
    let choose_two = &n * Rational::from(u64::from(updates.saturating_sub(1))) / Rational::from(2);
    let choose_three =
        &choose_two * Rational::from(u64::from(updates.saturating_sub(2))) / Rational::from(3);
    Ok(coefficients[0].maximum_error.clone()
        + &n * &coefficients[1].maximum_error
        + choose_two * &coefficients[2].maximum_error
        + choose_three * &coefficients[3].maximum_error)
}

#[allow(
    clippy::too_many_arguments,
    reason = "each array is a separate configuration-derived hardware bound"
)]
fn find_record_end<const AXES: usize>(
    source: &ServoFiniteDifferenceSegment<AXES>,
    start: u32,
    maximum_updates: u32,
    maximum_position_delta: [u64; AXES],
    maximum_position_rate: [u64; AXES],
    maximum_velocity: [u32; AXES],
    maximum_current: [u32; AXES],
    span: usize,
) -> ServoMotionResult<u32> {
    let cap = start
        .saturating_add(maximum_updates)
        .min(source.update_count);
    let base = source
        .state_at(start)
        .map_err(|error| ServoMotionError::Machine { span, error })?;
    let mut signs = [[0_i8; 3]; AXES];
    let mut update = start;
    while update < cap {
        let current = source
            .state_at(update)
            .map_err(|error| ServoMotionError::Machine { span, error })?;
        let next = source
            .state_at(update + 1)
            .map_err(|error| ServoMotionError::Machine { span, error })?;
        let mut conflict = false;
        let mut axis = 0;
        while axis < AXES {
            let position_delta =
                i128::from(next.position[axis]) - i128::from(current.position[axis]);
            let position_sign = sign_i128(position_delta);
            conflict |= sign_conflicts(signs[axis][0], position_sign);
            let velocity_delta = i64::from(next.velocity_feed_forward[axis])
                - i64::from(current.velocity_feed_forward[axis]);
            let velocity_sign = sign_i128(i128::from(velocity_delta));
            conflict |= sign_conflicts(signs[axis][1], velocity_sign);
            let current_delta = i64::from(next.quadrature_current_feed_forward[axis])
                - i64::from(current.quadrature_current_feed_forward[axis]);
            let current_sign = sign_i128(i128::from(current_delta));
            conflict |= sign_conflicts(signs[axis][2], current_sign);
            if conflict && update > start {
                break;
            }
            if position_delta.unsigned_abs() > u128::from(maximum_position_rate[axis])
                || (i128::from(next.position[axis]) - i128::from(base.position[axis]))
                    .unsigned_abs()
                    > u128::from(maximum_position_delta[axis])
                || next.velocity_feed_forward[axis].unsigned_abs() > maximum_velocity[axis]
                || current.velocity_feed_forward[axis].unsigned_abs() > maximum_velocity[axis]
                || next.quadrature_current_feed_forward[axis].unsigned_abs() > maximum_current[axis]
                || current.quadrature_current_feed_forward[axis].unsigned_abs()
                    > maximum_current[axis]
            {
                if update == start {
                    return Err(ServoMotionError::HardwareLimit { span, axis });
                }
                conflict = true;
                break;
            }
            axis += 1;
        }
        if conflict && update > start {
            break;
        }
        for (axis, retained_signs) in signs.iter_mut().enumerate() {
            let deltas = [
                i128::from(next.position[axis]) - i128::from(current.position[axis]),
                i128::from(
                    i64::from(next.velocity_feed_forward[axis])
                        - i64::from(current.velocity_feed_forward[axis]),
                ),
                i128::from(
                    i64::from(next.quadrature_current_feed_forward[axis])
                        - i64::from(current.quadrature_current_feed_forward[axis]),
                ),
            ];
            for (signal, delta) in deltas.into_iter().enumerate() {
                let sign = sign_i128(delta);
                if sign != 0 {
                    retained_signs[signal] = sign;
                }
            }
        }
        update += 1;
    }
    if update == start {
        return Err(ServoMotionError::HardwareLimit { span, axis: 0 });
    }
    Ok(update)
}

fn shift_axes<const AXES: usize>(
    axes: [ServoFiniteDifferenceAxis; AXES],
    update: u32,
    span: usize,
) -> ServoMotionResult<[ServoFiniteDifferenceAxis; AXES]> {
    let mut shifted = [ServoFiniteDifferenceAxis::default(); AXES];
    for axis in 0..AXES {
        shifted[axis] = ServoFiniteDifferenceAxis {
            position: shift_i64(axes[axis].position, update)
                .map_err(|_| ServoMotionError::CoefficientShift { span, axis })?,
            velocity_feed_forward: shift_i32(axes[axis].velocity_feed_forward, update)
                .map_err(|_| ServoMotionError::CoefficientShift { span, axis })?,
            quadrature_current_feed_forward: shift_i32(
                axes[axis].quadrature_current_feed_forward,
                update,
            )
            .map_err(|_| ServoMotionError::CoefficientShift { span, axis })?,
        };
    }
    Ok(shifted)
}

fn shift_i64(
    coefficients: alumina_machine_ir::FiniteDifferenceAxis,
    update: u32,
) -> Result<alumina_machine_ir::FiniteDifferenceAxis, ()> {
    let [
        initial_position,
        first_difference,
        second_difference,
        third_difference,
    ] = shift(
        [
            i128::from(coefficients.initial_position),
            i128::from(coefficients.first_difference),
            i128::from(coefficients.second_difference),
            i128::from(coefficients.third_difference),
        ],
        update,
    )?;
    Ok(alumina_machine_ir::FiniteDifferenceAxis {
        initial_position: i64::try_from(initial_position).map_err(|_| ())?,
        first_difference: i64::try_from(first_difference).map_err(|_| ())?,
        second_difference: i64::try_from(second_difference).map_err(|_| ())?,
        third_difference: i64::try_from(third_difference).map_err(|_| ())?,
    })
}

fn shift_i32(
    coefficients: ServoQ30FiniteDifferenceAxis,
    update: u32,
) -> Result<ServoQ30FiniteDifferenceAxis, ()> {
    let [
        initial_value,
        first_difference,
        second_difference,
        third_difference,
    ] = shift(
        [
            i128::from(coefficients.initial_value),
            i128::from(coefficients.first_difference),
            i128::from(coefficients.second_difference),
            i128::from(coefficients.third_difference),
        ],
        update,
    )?;
    Ok(ServoQ30FiniteDifferenceAxis {
        initial_value: i32::try_from(initial_value).map_err(|_| ())?,
        first_difference: i32::try_from(first_difference).map_err(|_| ())?,
        second_difference: i32::try_from(second_difference).map_err(|_| ())?,
        third_difference: i32::try_from(third_difference).map_err(|_| ())?,
    })
}

fn shift(coefficients: [i128; 4], update: u32) -> Result<[i128; 4], ()> {
    let k = i128::from(update);
    let choose_two = k
        .checked_mul(k - 1)
        .and_then(|value| value.checked_div(2))
        .ok_or(())?;
    let choose_three = choose_two
        .checked_mul(k - 2)
        .and_then(|value| value.checked_div(3))
        .ok_or(())?;
    let initial = coefficients[0]
        .checked_add(k.checked_mul(coefficients[1]).ok_or(())?)
        .and_then(|value| value.checked_add(choose_two.checked_mul(coefficients[2])?))
        .and_then(|value| value.checked_add(choose_three.checked_mul(coefficients[3])?))
        .ok_or(())?;
    let first = coefficients[1]
        .checked_add(k.checked_mul(coefficients[2]).ok_or(())?)
        .and_then(|value| value.checked_add(choose_two.checked_mul(coefficients[3])?))
        .ok_or(())?;
    let second = coefficients[2]
        .checked_add(k.checked_mul(coefficients[3]).ok_or(())?)
        .ok_or(())?;
    Ok([initial, first, second, coefficients[3]])
}

fn round_rational_ties_even(value: &Rational) -> ServoMotionResult<i64> {
    let truncated = i64::try_from(value.trunc()).map_err(|_| ServoMotionError::Arithmetic {
        domain: "servo coefficient projection",
    })?;
    let floor = if value.is_negative() && !value.fract().is_zero() {
        truncated
            .checked_sub(1)
            .ok_or(ServoMotionError::Arithmetic {
                domain: "servo coefficient projection floor",
            })?
    } else {
        truncated
    };
    let half = Rational::fraction(1, 2).map_err(|_| ServoMotionError::Arithmetic {
        domain: "servo ties-even threshold",
    })?;
    let remainder = value - Rational::from(floor);
    if remainder < half || (remainder == half && floor % 2 == 0) {
        Ok(floor)
    } else {
        floor.checked_add(1).ok_or(ServoMotionError::Arithmetic {
            domain: "servo coefficient ties-even increment",
        })
    }
}

fn absolute_rational(value: Rational) -> Rational {
    if value < Rational::zero() {
        -value
    } else {
        value
    }
}

fn maximum_rational(left: Rational, right: Rational) -> Rational {
    if left >= right { left } else { right }
}

const fn sign_i128(value: i128) -> i8 {
    if value < 0 {
        -1
    } else if value > 0 {
        1
    } else {
        0
    }
}

const fn sign_conflicts(retained: i8, candidate: i8) -> bool {
    retained != 0 && candidate != 0 && retained != candidate
}

/// Exact servo compilation rejection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServoMotionError {
    /// Allocation, precision, or approximation policy was zero or unsupported.
    InvalidPolicy,
    /// Compile-time axis width was zero or exceeded the firmware schema.
    AxisCount,
    /// No exact source span was supplied.
    EmptyProgram,
    /// Source span count exceeded the browser-owned allocation limit.
    SourceSpanLimit,
    /// An exact source span contained no update.
    UpdateCount {
        /// Zero-based rejected source span.
        span: usize,
    },
    /// Dense extrema inspection exceeded the caller-owned CPU bound.
    ExaminedUpdateLimit,
    /// Extrema/horizon splitting exceeded retained record capacity.
    OutputRecordLimit,
    /// The stream could not reserve nonzero command IDs plus its terminal hold.
    CommandCountOverflow,
    /// Device frequency or capability identity was absent.
    MissingIdentity,
    /// Configuration-derived limits and setpoint cadence disagreed.
    AdmissionProfile,
    /// The portable setpoint identity/cadence profile was invalid.
    SetpointProfile(CachedServoSetpointError),
    /// Hyperreal did not complete a certified projection.
    ProjectionAborted {
        /// Zero-based source span.
        span: usize,
        /// Zero-based configured axis.
        axis: usize,
        /// Rejected recurrence signal.
        signal: ServoSignal,
        /// Newton-forward coefficient index.
        coefficient: usize,
    },
    /// The selected refinement could not prove a unique ties-even integer.
    ProjectionUnresolved {
        /// Zero-based source span.
        span: usize,
        /// Zero-based configured axis.
        axis: usize,
        /// Rejected recurrence signal.
        signal: ServoSignal,
        /// Newton-forward coefficient index.
        coefficient: usize,
        /// Attempted binary refinement depth.
        precision_bits: u16,
    },
    /// A Q2.30 coefficient did not fit its signed wire field.
    CoefficientRange {
        /// Zero-based source span.
        span: usize,
        /// Zero-based configured axis.
        axis: usize,
        /// Rejected recurrence signal.
        signal: ServoSignal,
        /// Newton-forward coefficient index.
        coefficient: usize,
    },
    /// Certified recurrence error exceeded the explicit caller budget.
    ApproximationBudget {
        /// Zero-based source span.
        span: usize,
        /// Zero-based configured axis.
        axis: usize,
        /// Recurrence signal whose error bound was too wide.
        signal: ServoSignal,
    },
    /// One dense transition exceeded configuration-derived physical authority.
    HardwareLimit {
        /// Zero-based source span.
        span: usize,
        /// Zero-based configured axis.
        axis: usize,
    },
    /// A shifted integer recurrence exceeded its canonical coefficient field.
    CoefficientShift {
        /// Zero-based source span.
        span: usize,
        /// Zero-based configured axis.
        axis: usize,
    },
    /// The first physical setpoint was not at rest.
    InitialFeedForward,
    /// The complete stream did not return both feed-forward vectors to zero.
    TerminalFeedForward,
    /// Exact source-span continuation did not equal retained encoded state.
    Continuity {
        /// Zero-based source span.
        span: usize,
    },
    /// Firmware record validation rejected the canonical integer result.
    Machine {
        /// Zero-based source span.
        span: usize,
        /// Exact firmware-side validation rejection.
        error: ServoFiniteDifferenceError,
    },
    /// Checked integer or exact-rational arithmetic failed.
    Arithmetic {
        /// Stable exact-arithmetic operation label.
        domain: &'static str,
    },
    /// Browser allocation could not retain the requested evidence or records.
    AllocationOverflow,
}

impl fmt::Display for ServoMotionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "exact servo lowering failed: {self:?}")
    }
}

impl StdError for ServoMotionError {}

#[cfg(test)]
mod tests {
    use alumina_machine_ir::{
        BlockValidationLimits, ServoFiniteDifferenceBlockValidationLimits,
        ServoFiniteDifferenceValidationLimits, ValidationLimits,
    };
    use alumina_motion::CachedServoSetpointProfile;
    use alumina_storage::{CacheLimits, UploadId};

    use super::*;
    use crate::partition::{MachinePartitionPolicy2, package_canonical_servo_program};

    const POSITION_ONE: i64 = 1_i64 << SERVO_FINITE_DIFFERENCE_POSITION_FRACTION_BITS;

    fn rational(numerator: i64, denominator: u64) -> Rational {
        Rational::fraction(numerator, denominator).unwrap()
    }

    fn real(numerator: i64, denominator: u64) -> Real {
        Real::from(rational(numerator, denominator))
    }

    fn zero_axis() -> ExactServoAxisRecurrence {
        ExactServoAxisRecurrence::new(
            std::array::from_fn(|_| Real::zero()),
            std::array::from_fn(|_| Real::zero()),
            std::array::from_fn(|_| Real::zero()),
        )
    }

    fn admission(maximum_update_count: u32) -> CachedServoAdmissionProfile<2> {
        CachedServoAdmissionProfile {
            limits: ServoFiniteDifferenceBlockValidationLimits {
                maximum_block_ticks: 1_000,
                segment: ServoFiniteDifferenceValidationLimits {
                    maximum_segment_ticks: 1_000,
                    maximum_update_count,
                    required_update_period_ticks: 10,
                    maximum_position_delta_bits: [16 * POSITION_ONE as u64; 2],
                    maximum_absolute_position_first_difference_bits: [4 * POSITION_ONE as u64; 2],
                    maximum_absolute_velocity_feed_forward_bits: [1 << 29; 2],
                    maximum_absolute_quadrature_current_feed_forward_bits: [1 << 29; 2],
                },
            },
            setpoints: CachedServoSetpointProfile {
                configuration_digest: Digest([0x71; 32]),
                update_period_ticks: 10,
            },
        }
    }

    fn policy() -> ServoFiniteDifferenceCompilePolicy {
        ServoFiniteDifferenceCompilePolicy::try_new(
            16,
            64,
            1_000,
            128,
            rational(1, 1_000_000),
            rational(1, 1_000_000),
        )
        .unwrap()
    }

    #[test]
    fn exact_two_span_program_splits_horizons_and_returns_to_rest() {
        let first = ExactServoCubicSpan::new(
            4,
            [
                ExactServoAxisRecurrence::new(
                    [Real::zero(), real(1, 4), Real::zero(), Real::zero()],
                    [Real::zero(), real(1, 16), Real::zero(), Real::zero()],
                    [Real::zero(), Real::zero(), Real::zero(), Real::zero()],
                ),
                zero_axis(),
            ],
        );
        let second = ExactServoCubicSpan::new(
            4,
            [
                ExactServoAxisRecurrence::new(
                    [Real::one(), real(1, 4), Real::zero(), Real::zero()],
                    [real(1, 4), real(-1, 16), Real::zero(), Real::zero()],
                    [Real::zero(), Real::zero(), Real::zero(), Real::zero()],
                ),
                zero_axis(),
            ],
        );
        let servo_admission = admission(3);
        let program = lower_exact_servo_recurrences(
            &[first, second],
            80_000_000,
            Digest([0x72; 32]),
            servo_admission,
            policy(),
        )
        .unwrap();
        assert_eq!(program.records().len(), 4);
        assert_eq!(program.total_update_count(), 8);
        assert_eq!(program.initial_position(), [0, 0]);
        assert_eq!(program.final_state().position, [2 * POSITION_ONE, 0]);
        assert!(program.final_state().feed_forwards_are_zero());
        assert_eq!(program.records()[0].update_count, 3);
        assert_eq!(program.records()[1].update_count, 1);
        assert_eq!(program.records()[2].start_tick, StreamTick(40));
        assert!(program.evidence()[1].axes()[0].position()[0].continuity_forced());

        let partition_policy = MachinePartitionPolicy2::try_new(
            [0x73; 16],
            program.capability_digest(),
            program.configuration_digest(),
            BlockValidationLimits {
                maximum_block_ticks: servo_admission.limits.maximum_block_ticks,
                segment: ValidationLimits {
                    maximum_segment_ticks: servo_admission.limits.segment.maximum_segment_ticks,
                    maximum_steps_per_segment: servo_admission
                        .limits
                        .segment
                        .maximum_position_delta_bits
                        .into_iter()
                        .max()
                        .unwrap(),
                },
            },
            UploadId(0x7475_7677_7879_7a7b),
            700,
            CacheLimits {
                maximum_object_bytes: 16 * 512,
                maximum_chunk_bytes: 1_024,
                maximum_chunks: 16,
            },
        )
        .unwrap();
        let partition = package_canonical_servo_program(&program, partition_policy).unwrap();
        assert_eq!(partition.block_count(), 2);
        assert_eq!(partition.maximum_segments_per_block(), 2);
        assert_eq!(partition.total_update_count(), 8);
        assert_eq!(partition.terminal_state(), program.final_state());
        let descriptor = partition.job_descriptor(0x8182_8384).unwrap();
        assert_eq!(
            descriptor.execution_kind,
            alumina_machine_ir::ExecutionKind::ServoFiniteDifference
        );
        assert_eq!(descriptor.maximum_dense_updates, 3);
        assert_eq!(descriptor.dense_update_period_ticks, 10);
        assert_eq!(
            alumina_job::JobDescriptor::decode::<2>(
                &partition.job_prepare_body(0x8182_8384).unwrap()
            )
            .unwrap(),
            descriptor
        );
    }

    #[test]
    fn discrete_direction_reversal_is_split_without_sampling_floats() {
        let span = ExactServoCubicSpan::new(
            4,
            [
                ExactServoAxisRecurrence::new(
                    [Real::zero(), Real::one(), Real::from(-1), Real::zero()],
                    [Real::zero(), Real::zero(), Real::zero(), Real::zero()],
                    [Real::zero(), Real::zero(), Real::zero(), Real::zero()],
                ),
                zero_axis(),
            ],
        );
        let program = lower_exact_servo_recurrences(
            &[span],
            80_000_000,
            Digest([0x72; 32]),
            admission(10),
            policy(),
        )
        .unwrap();
        assert_eq!(program.records().len(), 2);
        assert_eq!(program.records()[0].update_count, 2);
        assert_eq!(program.records()[1].update_count, 2);
        assert_eq!(program.final_state().position[0], -2 * POSITION_ONE);
    }

    #[test]
    fn terminal_feed_forward_is_a_typed_rejection() {
        let span = ExactServoCubicSpan::new(
            2,
            [
                ExactServoAxisRecurrence::new(
                    [Real::zero(), Real::zero(), Real::zero(), Real::zero()],
                    [Real::zero(), real(1, 16), Real::zero(), Real::zero()],
                    [Real::zero(), Real::zero(), Real::zero(), Real::zero()],
                ),
                zero_axis(),
            ],
        );
        assert!(matches!(
            lower_exact_servo_recurrences(
                &[span],
                80_000_000,
                Digest([0x72; 32]),
                admission(10),
                policy(),
            ),
            Err(ServoMotionError::TerminalFeedForward)
        ));
    }
}
