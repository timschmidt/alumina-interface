//! Exact stop-to-stop Hyperpath schedules lowered to direct finite differences.
//!
//! This first direct-motion compiler deliberately accepts only affine metric
//! spans whose certified Hyperpath schedule comes to rest at every element
//! boundary. It rebuilds each symmetric four-phase profile on the configured
//! output grid, independently re-certifies the resulting slower schedule, and
//! projects exact cubic step coordinates to the firmware Q31.32 lattice with a
//! caller-owned refinement and error budget. Curved spans and positive-feed
//! joins remain typed failures rather than silently returning to chord/DDA
//! execution.

use std::cmp::Ordering;
use std::error::Error as StdError;
use std::fmt;

use alumina_machine_ir::{
    FINITE_DIFFERENCE_ONE_STEP, FiniteDifferenceAxis, FiniteDifferenceError,
    FiniteDifferenceSegment, FiniteDifferenceValidationLimits, StreamTick,
};
use alumina_motion::{
    FiniteDifferenceExecutionLimits, FiniteDifferencePreflightError,
    FiniteDifferencePreflightSummary, preflight_finite_difference_segments,
};
use alumina_protocol::Digest;
use hypercurve::CurvePath2;
use hyperlimit::{PredicatePolicy, compare_reals};
use hyperpath::{
    FeedPathElement, JerkRampPhaseProposal, JerkRampSpanProposal,
    MultiPhaseJerkRampFeedScheduleReport, RouteCertificationError,
    certify_multi_phase_jerk_ramp_feed_schedule,
};
use hyperreal::{Problem, Rational, Real};

use crate::compiler::{MachineCompileError, half_lattice_unit, quantize_axis};
use crate::machine_profile::{MachineDynamicsProfile2, MachineResolutionBudget2};
use crate::motion_schedule::{CertifiedJerkSchedule2, TravelBoundary};
use crate::toolpath::CertifiedMetricPath2;

/// Result type for direct finite-difference lowering.
pub type DirectMotionResult<T> = Result<T, DirectMotionError>;

const MINIMUM_COEFFICIENT_PRECISION_BITS: u16 = 40;
const MAXIMUM_COEFFICIENT_PRECISION_BITS: u16 = 512;
const MAXIMUM_POLICY_UPDATES_PER_RECORD: u32 = 1_000_000;

/// Caller-owned browser/WASM allocation and approximation policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectFiniteDifferencePolicy2 {
    maximum_records: usize,
    maximum_updates_per_record: u32,
    maximum_steps_per_record: u64,
    coefficient_precision_bits: u16,
    maximum_position_error_mm: Rational,
}

impl DirectFiniteDifferencePolicy2 {
    /// Construct a bounded exact direct-lowering policy.
    pub fn try_new(
        maximum_records: usize,
        maximum_updates_per_record: u32,
        maximum_steps_per_record: u64,
        coefficient_precision_bits: u16,
        maximum_position_error_mm: Rational,
    ) -> DirectMotionResult<Self> {
        if maximum_records == 0
            || !(2..=MAXIMUM_POLICY_UPDATES_PER_RECORD).contains(&maximum_updates_per_record)
            || maximum_steps_per_record == 0
            || !(MINIMUM_COEFFICIENT_PRECISION_BITS..=MAXIMUM_COEFFICIENT_PRECISION_BITS)
                .contains(&coefficient_precision_bits)
            || maximum_position_error_mm <= Rational::zero()
        {
            return Err(DirectMotionError::InvalidPolicy);
        }
        Ok(Self {
            maximum_records,
            maximum_updates_per_record,
            maximum_steps_per_record,
            coefficient_precision_bits,
            maximum_position_error_mm,
        })
    }

    /// Practical browser policy with caller-selected positional allocation.
    pub fn interactive(maximum_position_error_mm: Rational) -> DirectMotionResult<Self> {
        Self::try_new(65_536, 256, 10_000, 128, maximum_position_error_mm)
    }

    /// Maximum number of canonical records retained in browser memory.
    pub const fn maximum_records(&self) -> usize {
        self.maximum_records
    }

    /// Maximum dense updates represented by one record.
    pub const fn maximum_updates_per_record(&self) -> u32 {
        self.maximum_updates_per_record
    }

    /// Maximum rounded displacement on one axis in one record.
    pub const fn maximum_steps_per_record(&self) -> u64 {
        self.maximum_steps_per_record
    }

    /// Binary precision requested from Hyperreal for each coefficient enclosure.
    pub const fn coefficient_precision_bits(&self) -> u16 {
        self.coefficient_precision_bits
    }

    /// Caller-owned maximum coefficient-induced position error in millimetres.
    pub const fn maximum_position_error_mm(&self) -> &Rational {
        &self.maximum_position_error_mm
    }
}

/// Certified projection of one exact coefficient onto signed Q31.32.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectCoefficientProjection2 {
    ideal_steps: Real,
    scaled_interval: [Rational; 2],
    encoded_q31_32: i64,
    maximum_error_steps: Rational,
}

impl DirectCoefficientProjection2 {
    /// Exact ideal coefficient in command steps.
    pub const fn ideal_steps(&self) -> &Real {
        &self.ideal_steps
    }

    /// Closed certified interval for `ideal_steps * 2^32`.
    pub const fn scaled_interval(&self) -> &[Rational; 2] {
        &self.scaled_interval
    }

    /// Selected signed Q31.32 integer.
    pub const fn encoded_q31_32(&self) -> i64 {
        self.encoded_q31_32
    }

    /// Exact conservative error bound in command steps.
    pub const fn maximum_error_steps(&self) -> &Rational {
        &self.maximum_error_steps
    }
}

/// Coefficient and propagated-error certificate for one axis record.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectAxisRecordEvidence2 {
    first_difference: DirectCoefficientProjection2,
    second_difference: DirectCoefficientProjection2,
    third_difference: DirectCoefficientProjection2,
    incoming_position_error_steps: Rational,
    terminal_position_error_steps: Rational,
}

impl DirectAxisRecordEvidence2 {
    /// Projection of the first forward difference.
    pub const fn first_difference(&self) -> &DirectCoefficientProjection2 {
        &self.first_difference
    }

    /// Projection of the second forward difference.
    pub const fn second_difference(&self) -> &DirectCoefficientProjection2 {
        &self.second_difference
    }

    /// Projection of the third forward difference.
    pub const fn third_difference(&self) -> &DirectCoefficientProjection2 {
        &self.third_difference
    }

    /// Position-error bound carried into this record.
    pub const fn incoming_position_error_steps(&self) -> &Rational {
        &self.incoming_position_error_steps
    }

    /// Position-error bound after every update in this record.
    pub const fn terminal_position_error_steps(&self) -> &Rational {
        &self.terminal_position_error_steps
    }
}

/// Exact provenance and approximation proof for one canonical direct record.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectRecordEvidence2 {
    element_index: usize,
    phase_index: usize,
    first_phase_update: u64,
    segment: FiniteDifferenceSegment<2>,
    axes: [DirectAxisRecordEvidence2; 2],
}

impl DirectRecordEvidence2 {
    /// Retained affine metric element.
    pub const fn element_index(&self) -> usize {
        self.element_index
    }

    /// Re-certified constant-jerk phase in that element.
    pub const fn phase_index(&self) -> usize {
        self.phase_index
    }

    /// Zero-based dense update offset inside the phase.
    pub const fn first_phase_update(&self) -> u64 {
        self.first_phase_update
    }

    /// Canonical firmware record reconstructed by this certificate.
    pub const fn segment(&self) -> FiniteDifferenceSegment<2> {
        self.segment
    }

    /// Per-axis coefficient and propagated-error certificates.
    pub const fn axes(&self) -> &[DirectAxisRecordEvidence2; 2] {
        &self.axes
    }
}

/// Exact output-grid replacement of one certified phase duration.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectGridPhaseEvidence2 {
    element_index: usize,
    phase_index: usize,
    original_duration_seconds: Real,
    grid_duration_seconds: Rational,
    nonnegative_padding_seconds: Real,
    update_count: u64,
}

impl DirectGridPhaseEvidence2 {
    /// Retained affine metric element.
    pub const fn element_index(&self) -> usize {
        self.element_index
    }

    /// Reconstructed phase index.
    pub const fn phase_index(&self) -> usize {
        self.phase_index
    }

    /// Exact duration from the input Hyperpath certificate.
    pub const fn original_duration_seconds(&self) -> &Real {
        &self.original_duration_seconds
    }

    /// Exact output-grid duration selected for direct execution.
    pub const fn grid_duration_seconds(&self) -> &Rational {
        &self.grid_duration_seconds
    }

    /// Certified nonnegative grid extension.
    pub const fn nonnegative_padding_seconds(&self) -> &Real {
        &self.nonnegative_padding_seconds
    }

    /// Dense output-grid updates in the reconstructed phase.
    pub const fn update_count(&self) -> u64 {
        self.update_count
    }
}

/// Complete numerical and timing evidence retained beside direct machine IR.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectFiniteDifferenceEvidence2 {
    policy: DirectFiniteDifferencePolicy2,
    phase_evidence: Vec<DirectGridPhaseEvidence2>,
    record_evidence: Vec<DirectRecordEvidence2>,
    maximum_axis_position_error_steps: [Rational; 2],
    maximum_position_error_mm: Rational,
    total_update_count: u64,
}

impl DirectFiniteDifferenceEvidence2 {
    /// Caller-owned policy used for every projection and allocation.
    pub const fn policy(&self) -> &DirectFiniteDifferencePolicy2 {
        &self.policy
    }

    /// Exact before/after duration certificate for every jerk phase.
    pub fn phase_evidence(&self) -> &[DirectGridPhaseEvidence2] {
        &self.phase_evidence
    }

    /// Every canonical record and its coefficient proof.
    pub fn record_evidence(&self) -> &[DirectRecordEvidence2] {
        &self.record_evidence
    }

    /// Largest propagated command-step error bound for each axis.
    pub const fn maximum_axis_position_error_steps(&self) -> &[Rational; 2] {
        &self.maximum_axis_position_error_steps
    }

    /// Conservative exact L1 positional bound in millimetres.
    pub const fn maximum_position_error_mm(&self) -> &Rational {
        &self.maximum_position_error_mm
    }

    /// Total dense recurrence updates represented by all records.
    pub const fn total_update_count(&self) -> u64 {
        self.total_update_count
    }
}

/// Authoritative two-axis direct finite-difference machine program.
#[derive(Clone, Debug)]
pub struct CanonicalDirectFiniteDifferenceProgram2 {
    configuration_digest: Digest,
    capability_digest: Digest,
    source: CurvePath2,
    metric_path: CertifiedMetricPath2,
    timer_ticks_per_second: u64,
    output_quantum_cycles: u32,
    resolution_budget: MachineResolutionBudget2,
    grid_phases: Vec<Vec<JerkRampPhaseProposal>>,
    grid_jerk_report: MultiPhaseJerkRampFeedScheduleReport,
    initial_position: [i64; 2],
    records: Vec<FiniteDifferenceSegment<2>>,
    executor_preflight: FiniteDifferencePreflightSummary<2>,
    evidence: DirectFiniteDifferenceEvidence2,
}

impl CanonicalDirectFiniteDifferenceProgram2 {
    /// Canonical machine-configuration identity.
    pub const fn configuration_digest(&self) -> Digest {
        self.configuration_digest
    }

    /// Immutable board-capability identity.
    pub const fn capability_digest(&self) -> Digest {
        self.capability_digest
    }

    /// Exact retained Hypercurve source.
    pub const fn source(&self) -> &CurvePath2 {
        &self.source
    }

    /// Certified affine metric path actually executed.
    pub const fn metric_path(&self) -> &CertifiedMetricPath2 {
        &self.metric_path
    }

    /// Exact device-cycle frequency.
    pub const fn timer_ticks_per_second(&self) -> u64 {
        self.timer_ticks_per_second
    }

    /// Exact recurrence/output quantum.
    pub const fn output_quantum_cycles(&self) -> u32 {
        self.output_quantum_cycles
    }

    /// Full machine-resolution budget under which this program was admitted.
    pub const fn resolution_budget(&self) -> &MachineResolutionBudget2 {
        &self.resolution_budget
    }

    /// Output-grid-aligned phases independently certified by Hyperpath.
    pub fn grid_phases(&self) -> &[Vec<JerkRampPhaseProposal>] {
        &self.grid_phases
    }

    /// Hyperpath/Hypersolve report for the reconstructed exact phases.
    pub const fn grid_jerk_report(&self) -> &MultiPhaseJerkRampFeedScheduleReport {
        &self.grid_jerk_report
    }

    /// Absolute integer command position at stream tick zero.
    pub const fn initial_position(&self) -> [i64; 2] {
        self.initial_position
    }

    /// Absolute integer terminal command position after direct replay.
    pub const fn final_position(&self) -> [i64; 2] {
        self.executor_preflight.position
    }

    /// Canonical direct records in stream order.
    pub fn records(&self) -> &[FiniteDifferenceSegment<2>] {
        &self.records
    }

    /// Complete production electrical preflight.
    pub const fn executor_preflight(&self) -> FiniteDifferencePreflightSummary<2> {
        self.executor_preflight
    }

    /// Exact grid, coefficient, and propagated-error evidence.
    pub const fn evidence(&self) -> &DirectFiniteDifferenceEvidence2 {
        &self.evidence
    }
}

/// Lower one exact stop-separated affine Hyperpath schedule into firmware V2
/// direct finite-difference records.
pub fn lower_certified_schedule_to_direct_finite_difference(
    schedule: &CertifiedJerkSchedule2,
    profile: &MachineDynamicsProfile2,
    resolution_budget: &MachineResolutionBudget2,
    policy: DirectFiniteDifferencePolicy2,
) -> DirectMotionResult<CanonicalDirectFiniteDifferenceProgram2> {
    validate_identities(schedule, profile, resolution_budget, &policy)?;
    let (grid_phases, phase_evidence) = build_grid_phases(schedule, profile)?;
    let grid_jerk_report = certify_multi_phase_jerk_ramp_feed_schedule(
        schedule.route(),
        &grid_phases,
        schedule.limits().maximum_feed_mm_per_second().clone(),
        schedule
            .limits()
            .maximum_acceleration_mm_per_second_squared()
            .clone(),
        schedule.limits().maximum_jerk_mm_per_second_cubed().clone(),
        PredicatePolicy::STRICT,
    )?;
    if !grid_jerk_report.all_satisfied() {
        return Err(DirectMotionError::GridJerkScheduleUncertified {
            element: grid_jerk_report.first_unsatisfied_element(),
        });
    }

    let metric_start = schedule.metric_path().path().start().clone();
    let maximum_axis_quantization_error_mm = [
        half_lattice_unit(
            profile.axes()[0]
                .command_density_steps_per_millimetre()
                .nominal(),
        )?,
        half_lattice_unit(
            profile.axes()[1]
                .command_density_steps_per_millimetre()
                .nominal(),
        )?,
    ];
    let initial_position = [
        quantize_axis(
            metric_start.x(),
            profile.axes()[0]
                .command_density_steps_per_millimetre()
                .nominal(),
            &maximum_axis_quantization_error_mm[0],
            0,
            0,
        )?
        .0
        .get(),
        quantize_axis(
            metric_start.y(),
            profile.axes()[1]
                .command_density_steps_per_millimetre()
                .nominal(),
            &maximum_axis_quantization_error_mm[1],
            0,
            1,
        )?
        .0
        .get(),
    ];

    let timing = profile.stepper_timing(0);
    let build = build_records(schedule, profile, &grid_phases, &policy)?;
    if build.maximum_position_error_mm > policy.maximum_position_error_mm {
        return Err(DirectMotionError::PositionErrorBudgetExceeded {
            required_mm: build.maximum_position_error_mm,
            maximum_mm: policy.maximum_position_error_mm,
        });
    }
    let limits = direct_execution_limits(profile, &policy)?;
    let executor_preflight =
        preflight_finite_difference_segments(timing, limits, initial_position, &build.records)?;
    if executor_preflight.terminal_finite_position != build.terminal_finite_position
        || usize::try_from(executor_preflight.segment_count).ok() != Some(build.records.len())
        || executor_preflight.update_count != build.total_update_count
    {
        return Err(DirectMotionError::TerminalMismatch);
    }
    certify_position_inside_travel(executor_preflight.position, profile)?;

    Ok(CanonicalDirectFiniteDifferenceProgram2 {
        configuration_digest: profile.configuration_digest(),
        capability_digest: profile.capability_digest(),
        source: schedule.source().clone(),
        metric_path: schedule.metric_path().clone(),
        timer_ticks_per_second: profile.timer_ticks_per_second(),
        output_quantum_cycles: profile.output_quantum_cycles(),
        resolution_budget: resolution_budget.clone(),
        grid_phases,
        grid_jerk_report,
        initial_position,
        records: build.records,
        executor_preflight,
        evidence: DirectFiniteDifferenceEvidence2 {
            policy,
            phase_evidence,
            record_evidence: build.record_evidence,
            maximum_axis_position_error_steps: build.maximum_axis_position_error_steps,
            maximum_position_error_mm: build.maximum_position_error_mm,
            total_update_count: build.total_update_count,
        },
    })
}

fn validate_identities(
    schedule: &CertifiedJerkSchedule2,
    profile: &MachineDynamicsProfile2,
    resolution_budget: &MachineResolutionBudget2,
    policy: &DirectFiniteDifferencePolicy2,
) -> DirectMotionResult<()> {
    if schedule.configuration_digest() != profile.configuration_digest()
        || schedule.capability_digest() != profile.capability_digest()
        || resolution_budget.configuration_digest() != profile.configuration_digest()
        || resolution_budget.capability_digest() != profile.capability_digest()
    {
        return Err(DirectMotionError::MachineIdentityMismatch);
    }
    if schedule.metric_path().maximum_source_error_mm_exact()
        > resolution_budget.source_curve_allocation_mm_exact()
        || policy.maximum_position_error_mm
            > *resolution_budget.controller_interpolation_allocation_mm_exact()
    {
        return Err(DirectMotionError::PositionErrorAllocationExceeded);
    }
    Ok(())
}

fn build_grid_phases(
    schedule: &CertifiedJerkSchedule2,
    profile: &MachineDynamicsProfile2,
) -> DirectMotionResult<(
    Vec<Vec<JerkRampPhaseProposal>>,
    Vec<DirectGridPhaseEvidence2>,
)> {
    let mut phases = Vec::new();
    let mut evidence = Vec::new();
    phases
        .try_reserve_exact(schedule.route().len())
        .map_err(|_| DirectMotionError::AllocationOverflow)?;
    evidence
        .try_reserve_exact(schedule.route().len().saturating_mul(4))
        .map_err(|_| DirectMotionError::AllocationOverflow)?;

    for (element_index, (element, original)) in
        schedule.route().iter().zip(schedule.phases()).enumerate()
    {
        let FeedPathElement::Line(line) = element else {
            return Err(DirectMotionError::UnsupportedRouteElement { element_index });
        };
        if original.len() != 4
            || !is_exact_zero(&original[0].ramp.start_feed)
            || !is_exact_zero(&original[0].ramp.start_acceleration)
            || !is_exact_zero(&original[3].ramp.end_feed)
            || !is_exact_zero(&original[3].ramp.end_acceleration)
            || original.iter().skip(1).any(|phase| {
                compare_reals(
                    &phase.ramp.traversal_time,
                    &original[0].ramp.traversal_time,
                    PredicatePolicy::STRICT,
                )
                .value()
                    != Some(Ordering::Equal)
            })
        {
            return Err(DirectMotionError::UnsupportedNonstopSchedule { element_index });
        }
        let exact_frames = ((&original[0].ramp.traversal_time
            * Real::from(profile.timer_ticks_per_second()))
            / Real::from(profile.output_quantum_cycles()))?;
        let frames = u64::try_from(exact_frames.ceil_certified()?).map_err(|_| {
            DirectMotionError::IntegerOverflow {
                domain: "grid phase update count",
            }
        })?;
        if frames == 0 {
            return Err(DirectMotionError::IntegerOverflow {
                domain: "grid phase update count",
            });
        }
        let phase_cycles = frames
            .checked_mul(u64::from(profile.output_quantum_cycles()))
            .ok_or(DirectMotionError::IntegerOverflow {
                domain: "grid phase cycles",
            })?;
        let phase_duration =
            Rational::from(phase_cycles) / Rational::from(profile.timer_ticks_per_second());
        let padding = Real::from(phase_duration.clone()) - &original[0].ramp.traversal_time;
        match compare_reals(&padding, &Real::zero(), PredicatePolicy::STRICT).value() {
            Some(Ordering::Equal | Ordering::Greater) => {}
            Some(Ordering::Less) | None => return Err(DirectMotionError::GridTimeUncertified),
        }
        let rebuilt = symmetric_phases_at_time(
            &line.euclidean_length()?,
            Real::from(phase_duration.clone()),
        )?;
        for (phase_index, original_phase) in original.iter().enumerate() {
            evidence.push(DirectGridPhaseEvidence2 {
                element_index,
                phase_index,
                original_duration_seconds: original_phase.ramp.traversal_time.clone(),
                grid_duration_seconds: phase_duration.clone(),
                nonnegative_padding_seconds: Real::from(phase_duration.clone())
                    - &original_phase.ramp.traversal_time,
                update_count: frames,
            });
        }
        phases.push(rebuilt);
    }
    Ok((phases, evidence))
}

fn symmetric_phases_at_time(
    length: &Real,
    phase_time: Real,
) -> DirectMotionResult<Vec<JerkRampPhaseProposal>> {
    let peak_feed = (length / (Real::from(2) * &phase_time))?;
    let half_peak_feed = (&peak_feed / Real::from(2))?;
    let peak_acceleration = (&peak_feed / &phase_time)?;
    let unit_length = (length / Real::from(12))?;
    let five_units = Real::from(5) * &unit_length;
    Ok(vec![
        JerkRampPhaseProposal {
            path_length: unit_length.clone(),
            ramp: JerkRampSpanProposal {
                start_feed: Real::zero(),
                end_feed: half_peak_feed.clone(),
                start_acceleration: Real::zero(),
                end_acceleration: peak_acceleration.clone(),
                traversal_time: phase_time.clone(),
            },
        },
        JerkRampPhaseProposal {
            path_length: five_units.clone(),
            ramp: JerkRampSpanProposal {
                start_feed: half_peak_feed.clone(),
                end_feed: peak_feed.clone(),
                start_acceleration: peak_acceleration.clone(),
                end_acceleration: Real::zero(),
                traversal_time: phase_time.clone(),
            },
        },
        JerkRampPhaseProposal {
            path_length: five_units,
            ramp: JerkRampSpanProposal {
                start_feed: peak_feed,
                end_feed: half_peak_feed.clone(),
                start_acceleration: Real::zero(),
                end_acceleration: -&peak_acceleration,
                traversal_time: phase_time.clone(),
            },
        },
        JerkRampPhaseProposal {
            path_length: unit_length,
            ramp: JerkRampSpanProposal {
                start_feed: half_peak_feed,
                end_feed: Real::zero(),
                start_acceleration: -peak_acceleration,
                end_acceleration: Real::zero(),
                traversal_time: phase_time,
            },
        },
    ])
}

struct RecordBuild {
    records: Vec<FiniteDifferenceSegment<2>>,
    record_evidence: Vec<DirectRecordEvidence2>,
    terminal_finite_position: [i64; 2],
    maximum_axis_position_error_steps: [Rational; 2],
    maximum_position_error_mm: Rational,
    total_update_count: u64,
}

fn build_records(
    schedule: &CertifiedJerkSchedule2,
    profile: &MachineDynamicsProfile2,
    grid_phases: &[Vec<JerkRampPhaseProposal>],
    policy: &DirectFiniteDifferencePolicy2,
) -> DirectMotionResult<RecordBuild> {
    let mut records = Vec::new();
    let mut record_evidence = Vec::new();
    records
        .try_reserve(policy.maximum_records.min(4_096))
        .map_err(|_| DirectMotionError::AllocationOverflow)?;
    record_evidence
        .try_reserve(policy.maximum_records.min(4_096))
        .map_err(|_| DirectMotionError::AllocationOverflow)?;
    let metric_start = schedule.metric_path().path().start();
    let update_period = profile.output_quantum_cycles();
    let update_seconds =
        (Real::from(update_period) / Real::from(profile.timer_ticks_per_second()))?;
    let structural_limits = direct_validation_limits(profile, policy)?;
    let mut next_tick = 0_u64;
    let mut finite_position = [0_i64; 2];
    let mut propagated_error = [Rational::zero(), Rational::zero()];
    let mut maximum_axis_error = [Rational::zero(), Rational::zero()];
    let mut total_update_count = 0_u64;

    for (element_index, (element, element_phases)) in
        schedule.route().iter().zip(grid_phases).enumerate()
    {
        let FeedPathElement::Line(line) = element else {
            return Err(DirectMotionError::UnsupportedRouteElement { element_index });
        };
        let length = line.euclidean_length()?;
        let direction = line.direction_vector();
        let line_base = [
            (&line.start().x - metric_start.x())
                * Real::from(
                    profile.axes()[0]
                        .command_density_steps_per_millimetre()
                        .nominal()
                        .clone(),
                ),
            (&line.start().y - metric_start.y())
                * Real::from(
                    profile.axes()[1]
                        .command_density_steps_per_millimetre()
                        .nominal()
                        .clone(),
                ),
        ];
        let axis_scale = [
            ((&direction.x / &length)?
                * Real::from(
                    profile.axes()[0]
                        .command_density_steps_per_millimetre()
                        .nominal()
                        .clone(),
                )),
            ((&direction.y / &length)?
                * Real::from(
                    profile.axes()[1]
                        .command_density_steps_per_millimetre()
                        .nominal()
                        .clone(),
                )),
        ];
        let mut element_distance = Real::zero();

        for (phase_index, phase) in element_phases.iter().enumerate() {
            let phase_updates_real = (&phase.ramp.traversal_time / &update_seconds)?;
            let phase_updates =
                u64::try_from(phase_updates_real.floor_certified()?).map_err(|_| {
                    DirectMotionError::IntegerOverflow {
                        domain: "phase update count",
                    }
                })?;
            if phase_updates == 0
                || compare_reals(
                    &phase_updates_real,
                    &Real::from(phase_updates),
                    PredicatePolicy::STRICT,
                )
                .value()
                    != Some(Ordering::Equal)
            {
                return Err(DirectMotionError::GridTimeUncertified);
            }

            let mut phase_offset = 0_u64;
            while phase_offset < phase_updates {
                if records.len() >= policy.maximum_records {
                    return Err(DirectMotionError::RecordBudgetExceeded {
                        maximum: policy.maximum_records,
                    });
                }
                let remaining = phase_updates - phase_offset;
                let tentative_count =
                    u32::try_from(remaining.min(u64::from(policy.maximum_updates_per_record)))
                        .map_err(|_| DirectMotionError::IntegerOverflow {
                            domain: "record update count",
                        })?;
                if tentative_count == 0 {
                    return Err(DirectMotionError::InvalidPolicy);
                }
                let ideal_differences = ideal_axis_differences(
                    &line_base,
                    &axis_scale,
                    &element_distance,
                    &phase.ramp,
                    &update_seconds,
                    phase_offset,
                )?;
                let projections = [
                    project_axis_differences(
                        &ideal_differences[0],
                        policy.coefficient_precision_bits,
                    )?,
                    project_axis_differences(
                        &ideal_differences[1],
                        policy.coefficient_precision_bits,
                    )?,
                ];
                let axes = [
                    FiniteDifferenceAxis {
                        initial_position: finite_position[0],
                        first_difference: projections[0].0.encoded_q31_32,
                        second_difference: projections[0].1.encoded_q31_32,
                        third_difference: projections[0].2.encoded_q31_32,
                    },
                    FiniteDifferenceAxis {
                        initial_position: finite_position[1],
                        first_difference: projections[1].0.encoded_q31_32,
                        second_difference: projections[1].1.encoded_q31_32,
                        third_difference: projections[1].2.encoded_q31_32,
                    },
                ];
                let selected = select_structurally_valid_record(
                    next_tick,
                    update_period,
                    tentative_count,
                    axes,
                    finite_position,
                    structural_limits,
                )
                .map_err(|source| DirectMotionError::RecordValidation {
                    element_index,
                    phase_index,
                    first_phase_update: phase_offset,
                    source,
                })?;
                let count = selected.update_count;
                let terminal_error = [
                    propagate_error(&propagated_error[0], &projections[0], count),
                    propagate_error(&propagated_error[1], &projections[1], count),
                ];
                maximum_axis_error[0] =
                    maximum_rational(maximum_axis_error[0].clone(), terminal_error[0].clone());
                maximum_axis_error[1] =
                    maximum_rational(maximum_axis_error[1].clone(), terminal_error[1].clone());
                record_evidence.push(DirectRecordEvidence2 {
                    element_index,
                    phase_index,
                    first_phase_update: phase_offset,
                    segment: selected,
                    axes: [
                        DirectAxisRecordEvidence2 {
                            first_difference: projections[0].0.clone(),
                            second_difference: projections[0].1.clone(),
                            third_difference: projections[0].2.clone(),
                            incoming_position_error_steps: propagated_error[0].clone(),
                            terminal_position_error_steps: terminal_error[0].clone(),
                        },
                        DirectAxisRecordEvidence2 {
                            first_difference: projections[1].0.clone(),
                            second_difference: projections[1].1.clone(),
                            third_difference: projections[1].2.clone(),
                            incoming_position_error_steps: propagated_error[1].clone(),
                            terminal_position_error_steps: terminal_error[1].clone(),
                        },
                    ],
                });
                propagated_error = terminal_error;
                finite_position = [
                    selected.position_at(0, count)?,
                    selected.position_at(1, count)?,
                ];
                next_tick = selected.end_tick.0;
                phase_offset = phase_offset.checked_add(u64::from(count)).ok_or(
                    DirectMotionError::IntegerOverflow {
                        domain: "phase update cursor",
                    },
                )?;
                total_update_count = total_update_count.checked_add(u64::from(count)).ok_or(
                    DirectMotionError::IntegerOverflow {
                        domain: "total direct updates",
                    },
                )?;
                records.push(selected);
            }
            element_distance += &phase.path_length;
        }
    }
    let maximum_position_error_mm = &maximum_axis_error[0]
        / profile.axes()[0]
            .command_density_steps_per_millimetre()
            .lower()
        + &maximum_axis_error[1]
            / profile.axes()[1]
                .command_density_steps_per_millimetre()
                .lower();
    if records.is_empty() {
        return Err(DirectMotionError::EmptyProgram);
    }
    Ok(RecordBuild {
        records,
        record_evidence,
        terminal_finite_position: finite_position,
        maximum_axis_position_error_steps: maximum_axis_error,
        maximum_position_error_mm,
        total_update_count,
    })
}

fn select_structurally_valid_record(
    start_tick: u64,
    update_period: u32,
    tentative_count: u32,
    axes: [FiniteDifferenceAxis; 2],
    expected_position: [i64; 2],
    limits: FiniteDifferenceValidationLimits<2>,
) -> Result<FiniteDifferenceSegment<2>, FiniteDifferenceError> {
    let candidate = finite_difference_record(start_tick, update_period, tentative_count, axes)?;
    match candidate.validate(StreamTick(start_tick), expected_position, limits) {
        Ok(_) => return Ok(candidate),
        Err(error) if record_length_refinement_candidate(error) => {}
        Err(error) => return Err(error),
    }

    let first = finite_difference_record(start_tick, update_period, 1, axes)?;
    first.validate(StreamTick(start_tick), expected_position, limits)?;
    let mut valid = 1_u32;
    let mut invalid = tentative_count;
    while valid + 1 < invalid {
        let middle = valid + (invalid - valid) / 2;
        let candidate = finite_difference_record(start_tick, update_period, middle, axes)?;
        match candidate.validate(StreamTick(start_tick), expected_position, limits) {
            Ok(_) => valid = middle,
            Err(error) if record_length_refinement_candidate(error) => invalid = middle,
            Err(error) => return Err(error),
        }
    }
    finite_difference_record(start_tick, update_period, valid, axes)
}

fn record_length_refinement_candidate(error: FiniteDifferenceError) -> bool {
    matches!(
        error,
        FiniteDifferenceError::DirectionReversal { .. }
            | FiniteDifferenceError::TooManySteps { .. }
    )
}

fn finite_difference_record(
    start_tick: u64,
    update_period: u32,
    update_count: u32,
    axes: [FiniteDifferenceAxis; 2],
) -> Result<FiniteDifferenceSegment<2>, FiniteDifferenceError> {
    let duration = u64::from(update_period)
        .checked_mul(u64::from(update_count))
        .ok_or(FiniteDifferenceError::Arithmetic)?;
    let end_tick = start_tick
        .checked_add(duration)
        .ok_or(FiniteDifferenceError::Arithmetic)?;
    Ok(FiniteDifferenceSegment {
        start_tick: StreamTick(start_tick),
        end_tick: StreamTick(end_tick),
        update_period_ticks: update_period,
        update_count,
        axes,
        flags: 0,
    })
}

fn ideal_axis_differences(
    line_base: &[Real; 2],
    axis_scale: &[Real; 2],
    element_distance: &Real,
    phase: &JerkRampSpanProposal,
    update_seconds: &Real,
    phase_offset: u64,
) -> DirectMotionResult<[[Real; 3]; 2]> {
    let p0 = ideal_axis_position(
        line_base,
        axis_scale,
        element_distance,
        phase,
        update_seconds,
        phase_offset,
        0,
    )?;
    let p1 = ideal_axis_position(
        line_base,
        axis_scale,
        element_distance,
        phase,
        update_seconds,
        phase_offset,
        1,
    )?;
    let p2 = ideal_axis_position(
        line_base,
        axis_scale,
        element_distance,
        phase,
        update_seconds,
        phase_offset,
        2,
    )?;
    let p3 = ideal_axis_position(
        line_base,
        axis_scale,
        element_distance,
        phase,
        update_seconds,
        phase_offset,
        3,
    )?;
    Ok(std::array::from_fn(|axis| {
        let first = &p1[axis] - &p0[axis];
        let second = &p2[axis] - Real::from(2) * &p1[axis] + &p0[axis];
        let third = &p3[axis] - Real::from(3) * &p2[axis] + Real::from(3) * &p1[axis] - &p0[axis];
        [first, second, third]
    }))
}

#[allow(
    clippy::too_many_arguments,
    reason = "one exact polynomial sample keeps all provenance inputs explicit"
)]
fn ideal_axis_position(
    line_base: &[Real; 2],
    axis_scale: &[Real; 2],
    element_distance: &Real,
    phase: &JerkRampSpanProposal,
    update_seconds: &Real,
    phase_offset: u64,
    delta: u64,
) -> DirectMotionResult<[Real; 2]> {
    let update = phase_offset
        .checked_add(delta)
        .ok_or(DirectMotionError::IntegerOverflow {
            domain: "coefficient sample update",
        })?;
    let local_time = update_seconds * Real::from(update);
    let distance = element_distance + phase_distance(phase, &local_time)?;
    Ok([
        &line_base[0] + &axis_scale[0] * &distance,
        &line_base[1] + &axis_scale[1] * &distance,
    ])
}

fn project_axis_differences(
    ideal: &[Real; 3],
    precision_bits: u16,
) -> DirectMotionResult<(
    DirectCoefficientProjection2,
    DirectCoefficientProjection2,
    DirectCoefficientProjection2,
)> {
    Ok((
        project_coefficient(&ideal[0], precision_bits)?,
        project_coefficient(&ideal[1], precision_bits)?,
        project_coefficient(&ideal[2], precision_bits)?,
    ))
}

fn project_coefficient(
    ideal_steps: &Real,
    precision_bits: u16,
) -> DirectMotionResult<DirectCoefficientProjection2> {
    let scaled = ideal_steps * Real::from(FINITE_DIFFERENCE_ONE_STEP);
    let precision = -i32::from(precision_bits);
    let interval = scaled
        .certified_dyadic_interval(precision)
        .ok_or(DirectMotionError::CoefficientApproximationAborted)?;
    let lower = round_rational_ties_even(&interval[0])?;
    let upper = round_rational_ties_even(&interval[1])?;
    if lower != upper {
        return Err(DirectMotionError::CoefficientProjectionUnresolved { precision_bits });
    }
    let encoded = Rational::from(lower);
    let lower_error = absolute_rational(&interval[0] - &encoded);
    let upper_error = absolute_rational(&interval[1] - &encoded);
    let maximum_scaled_error = maximum_rational(lower_error, upper_error);
    Ok(DirectCoefficientProjection2 {
        ideal_steps: ideal_steps.clone(),
        scaled_interval: interval,
        encoded_q31_32: lower,
        maximum_error_steps: maximum_scaled_error / Rational::from(FINITE_DIFFERENCE_ONE_STEP),
    })
}

fn round_rational_ties_even(value: &Rational) -> DirectMotionResult<i64> {
    let truncated =
        i64::try_from(value.trunc()).map_err(|_| DirectMotionError::IntegerOverflow {
            domain: "Q31.32 coefficient projection",
        })?;
    let floor = if value.is_negative() && !value.fract().is_zero() {
        truncated
            .checked_sub(1)
            .ok_or(DirectMotionError::IntegerOverflow {
                domain: "Q31.32 coefficient projection",
            })?
    } else {
        truncated
    };
    let remainder = value - Rational::from(floor);
    let half = Rational::fraction(1, 2)?;
    if remainder < half || (remainder == half && floor % 2 == 0) {
        Ok(floor)
    } else {
        floor
            .checked_add(1)
            .ok_or(DirectMotionError::IntegerOverflow {
                domain: "Q31.32 coefficient projection",
            })
    }
}

fn propagate_error(
    incoming: &Rational,
    projections: &(
        DirectCoefficientProjection2,
        DirectCoefficientProjection2,
        DirectCoefficientProjection2,
    ),
    updates: u32,
) -> Rational {
    let n = Rational::from(u64::from(updates));
    let choose_two = &n * Rational::from(u64::from(updates.saturating_sub(1))) / Rational::from(2);
    let choose_three =
        &choose_two * Rational::from(u64::from(updates.saturating_sub(2))) / Rational::from(3);
    incoming
        + &n * &projections.0.maximum_error_steps
        + choose_two * &projections.1.maximum_error_steps
        + choose_three * &projections.2.maximum_error_steps
}

fn direct_validation_limits(
    profile: &MachineDynamicsProfile2,
    policy: &DirectFiniteDifferencePolicy2,
) -> DirectMotionResult<FiniteDifferenceValidationLimits<2>> {
    Ok(FiniteDifferenceValidationLimits {
        maximum_segment_ticks: u64::from(profile.output_quantum_cycles())
            .checked_mul(u64::from(policy.maximum_updates_per_record))
            .ok_or(DirectMotionError::IntegerOverflow {
                domain: "maximum direct record ticks",
            })?,
        maximum_update_count: policy.maximum_updates_per_record,
        maximum_steps_per_segment: policy.maximum_steps_per_record,
        maximum_absolute_first_difference: [FINITE_DIFFERENCE_ONE_STEP.unsigned_abs() - 1; 2],
    })
}

fn direct_execution_limits(
    profile: &MachineDynamicsProfile2,
    policy: &DirectFiniteDifferencePolicy2,
) -> DirectMotionResult<FiniteDifferenceExecutionLimits> {
    Ok(FiniteDifferenceExecutionLimits {
        maximum_segment_ticks: u64::from(profile.output_quantum_cycles())
            .checked_mul(u64::from(policy.maximum_updates_per_record))
            .ok_or(DirectMotionError::IntegerOverflow {
                domain: "maximum direct record ticks",
            })?,
        maximum_update_count: policy.maximum_updates_per_record,
        maximum_steps_per_segment: policy.maximum_steps_per_record,
    })
}

fn phase_distance(phase: &JerkRampSpanProposal, time: &Real) -> DirectMotionResult<Real> {
    let jerk = ((&phase.end_acceleration - &phase.start_acceleration) / &phase.traversal_time)?;
    let time_squared = time * time;
    let time_cubed = &time_squared * time;
    Ok(&phase.start_feed * time
        + (&phase.start_acceleration * time_squared / Real::from(2))?
        + (jerk * time_cubed / Real::from(6))?)
}

fn is_exact_zero(value: &Real) -> bool {
    compare_reals(value, &Real::zero(), PredicatePolicy::STRICT).value() == Some(Ordering::Equal)
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

fn certify_position_inside_travel(
    position: [i64; 2],
    profile: &MachineDynamicsProfile2,
) -> DirectMotionResult<()> {
    for (axis, position) in position.into_iter().enumerate() {
        let coordinate_mm = (Real::from(position)
            / Real::from(
                profile.axes()[axis]
                    .command_density_steps_per_millimetre()
                    .nominal()
                    .clone(),
            ))?;
        let minimum = Real::from(
            profile.axes()[axis].usable_position_minimum_metres() * Rational::from(1_000),
        );
        let maximum = Real::from(
            profile.axes()[axis].usable_position_maximum_metres() * Rational::from(1_000),
        );
        for (boundary, limit, outside) in [
            (TravelBoundary::Minimum, minimum, Ordering::Less),
            (TravelBoundary::Maximum, maximum, Ordering::Greater),
        ] {
            match compare_reals(&coordinate_mm, &limit, PredicatePolicy::STRICT).value() {
                Some(ordering) if ordering == outside => {
                    return Err(DirectMotionError::CanonicalTravelExceeded { axis, boundary });
                }
                Some(_) => {}
                None => {
                    return Err(DirectMotionError::CanonicalTravelPredicateUnresolved {
                        axis,
                        boundary,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Failure to construct or certify a direct finite-difference program.
#[derive(Debug)]
pub enum DirectMotionError {
    /// Caller allocation, update, precision, or error policy was invalid.
    InvalidPolicy,
    /// Schedule, machine profile, and resolution-budget identities differed.
    MachineIdentityMismatch,
    /// Source reduction or direct error allocation exceeded the machine budget.
    PositionErrorAllocationExceeded,
    /// Curved and non-affine metric elements are not yet accepted.
    UnsupportedRouteElement {
        /// Zero-based metric element.
        element_index: usize,
    },
    /// The first direct slice requires a complete stop at this element.
    UnsupportedNonstopSchedule {
        /// Zero-based metric element.
        element_index: usize,
    },
    /// Output-grid phase time was not an exact nonnegative extension.
    GridTimeUncertified,
    /// Reconstructed exact phases did not pass Hyperpath/Hypersolve replay.
    GridJerkScheduleUncertified {
        /// First failed element, when reported by Hyperpath.
        element: Option<usize>,
    },
    /// Hyper exact arithmetic failed.
    Arithmetic(Problem),
    /// Existing exact coordinate quantization failed.
    MachineCompile(MachineCompileError),
    /// Hyperpath rejected the reconstructed phase schedule.
    Route(RouteCertificationError),
    /// Hyperreal aborted a requested coefficient enclosure.
    CoefficientApproximationAborted,
    /// The selected precision did not isolate one ties-to-even coefficient.
    CoefficientProjectionUnresolved {
        /// Caller-selected binary precision.
        precision_bits: u16,
    },
    /// Canonical fixed-width integer storage overflowed.
    IntegerOverflow {
        /// Failed value domain.
        domain: &'static str,
    },
    /// Browser record allocation exceeded its caller-owned bound.
    RecordBudgetExceeded {
        /// Maximum retained record count.
        maximum: usize,
    },
    /// Canonical record arithmetic or monotonic validation failed.
    FiniteDifference(FiniteDifferenceError),
    /// A constructed record failed with retained schedule provenance.
    RecordValidation {
        /// Zero-based metric element.
        element_index: usize,
        /// Zero-based constant-jerk phase.
        phase_index: usize,
        /// First dense update inside that phase.
        first_phase_update: u64,
        /// Canonical structural failure.
        source: FiniteDifferenceError,
    },
    /// Production electrical preflight rejected the complete stream.
    ExecutorPreflight(FiniteDifferencePreflightError),
    /// Proven coefficient-position error exceeded the caller allocation.
    PositionErrorBudgetExceeded {
        /// Required exact conservative bound.
        required_mm: Rational,
        /// Caller-owned maximum.
        maximum_mm: Rational,
    },
    /// Final independent numerical/executor facts diverged.
    TerminalMismatch,
    /// Canonical terminal position exceeded conservative usable travel.
    CanonicalTravelExceeded {
        /// Dense axis index.
        axis: usize,
        /// Rejected travel side.
        boundary: TravelBoundary,
    },
    /// Terminal travel comparison remained unresolved.
    CanonicalTravelPredicateUnresolved {
        /// Dense axis index.
        axis: usize,
        /// Undecided travel side.
        boundary: TravelBoundary,
    },
    /// A bounded vector allocation failed.
    AllocationOverflow,
    /// No direct records were constructed.
    EmptyProgram,
}

impl fmt::Display for DirectMotionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy => formatter.write_str("invalid direct finite-difference policy"),
            Self::MachineIdentityMismatch => {
                formatter.write_str("direct schedule and machine identities differ")
            }
            Self::PositionErrorAllocationExceeded => formatter
                .write_str("direct or source approximation allocation exceeds machine budget"),
            Self::UnsupportedRouteElement { element_index } => write!(
                formatter,
                "direct lowering does not yet support metric element {element_index}"
            ),
            Self::UnsupportedNonstopSchedule { element_index } => write!(
                formatter,
                "direct lowering requires a stop-separated four-phase schedule at element {element_index}"
            ),
            Self::GridTimeUncertified => {
                formatter.write_str("direct phase duration did not certify on the output grid")
            }
            Self::GridJerkScheduleUncertified { element } => write!(
                formatter,
                "output-grid jerk schedule failed certification at element {element:?}"
            ),
            Self::Arithmetic(source) => {
                write!(formatter, "exact direct arithmetic failed: {source}")
            }
            Self::MachineCompile(source) => {
                write!(formatter, "direct coordinate lowering failed: {source}")
            }
            Self::Route(source) => write!(formatter, "direct Hyperpath replay failed: {source:?}"),
            Self::CoefficientApproximationAborted => {
                formatter.write_str("Hyperreal aborted a coefficient enclosure")
            }
            Self::CoefficientProjectionUnresolved { precision_bits } => write!(
                formatter,
                "{precision_bits}-bit coefficient refinement did not isolate one Q31.32 value"
            ),
            Self::IntegerOverflow { domain } => {
                write!(formatter, "{domain} exceeded fixed-width storage")
            }
            Self::RecordBudgetExceeded { maximum } => write!(
                formatter,
                "direct lowering exceeded the {maximum}-record browser bound"
            ),
            Self::FiniteDifference(source) => {
                write!(formatter, "canonical direct record failed: {source:?}")
            }
            Self::RecordValidation {
                element_index,
                phase_index,
                first_phase_update,
                source,
            } => write!(
                formatter,
                "direct element {element_index} phase {phase_index} record at update {first_phase_update} failed: {source:?}"
            ),
            Self::ExecutorPreflight(source) => {
                write!(formatter, "direct production preflight failed: {source:?}")
            }
            Self::PositionErrorBudgetExceeded {
                required_mm,
                maximum_mm,
            } => write!(
                formatter,
                "direct coefficient bound {required_mm} mm exceeds {maximum_mm} mm"
            ),
            Self::TerminalMismatch => {
                formatter.write_str("direct terminal recurrence and preflight facts diverged")
            }
            Self::CanonicalTravelExceeded { axis, boundary } => write!(
                formatter,
                "direct terminal axis {axis} exceeds {boundary:?} usable travel"
            ),
            Self::CanonicalTravelPredicateUnresolved { axis, boundary } => write!(
                formatter,
                "direct terminal axis {axis} {boundary:?} travel predicate remained unresolved"
            ),
            Self::AllocationOverflow => {
                formatter.write_str("bounded direct compiler allocation failed")
            }
            Self::EmptyProgram => formatter.write_str("direct program contains no records"),
        }
    }
}

impl StdError for DirectMotionError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Arithmetic(source) => Some(source),
            Self::MachineCompile(source) => Some(source),
            _ => None,
        }
    }
}

impl From<Problem> for DirectMotionError {
    fn from(value: Problem) -> Self {
        Self::Arithmetic(value)
    }
}

impl From<MachineCompileError> for DirectMotionError {
    fn from(value: MachineCompileError) -> Self {
        Self::MachineCompile(value)
    }
}

impl From<RouteCertificationError> for DirectMotionError {
    fn from(value: RouteCertificationError) -> Self {
        Self::Route(value)
    }
}

impl From<FiniteDifferenceError> for DirectMotionError {
    fn from(value: FiniteDifferenceError) -> Self {
        Self::FiniteDifference(value)
    }
}

impl From<FiniteDifferencePreflightError> for DirectMotionError {
    fn from(value: FiniteDifferencePreflightError) -> Self {
        Self::ExecutorPreflight(value)
    }
}
