//! Path-wide exact-stop lookahead and certified jerk-limited feed scheduling.
//!
//! The first schedule policy retains the exact Hypercurve source and admits a
//! separate metric line/arc path only through lossless promotion or a bounded
//! pointwise certificate over exact Hypercurve de Casteljau spans. Every metric join has radius zero
//! and therefore an exact zero-feed node, including every certified cubic
//! chord boundary. Each metric element is traversed by a four-phase symmetric
//! constant-jerk profile. Hyperpath and Hypersolve replay every lookahead,
//! phase, length, continuity, feed, acceleration, and jerk condition before a
//! schedule is exposed. No sampled display chord is used as path geometry.

use std::cmp::Ordering;
use std::error::Error as StdError;
use std::fmt;

use alumina_machine_ir::ExecutionSegment;
use alumina_motion::{MotionError, StepperPreflightSummary, preflight_stepper_segments};
use alumina_protocol::Digest;
use hypercurve::{
    Classification, CurveContext, CurveError, CurveGeometry2, CurvePath2, ExactCurveError,
    Point2 as CurvePoint2,
};
use hyperlimit::{PredicatePolicy, Sign, classify_real_sign, compare_reals};
use hyperpath::{
    FeedPathElement, JerkRampPhaseProposal, JerkRampSpanProposal, LookaheadFeedSchedule,
    LookaheadFeedScheduleReport, MultiPhaseJerkRampFeedScheduleReport, RouteCertificationError,
    TangentSpan, certify_lookahead_feed_schedule, certify_multi_phase_jerk_ramp_feed_schedule,
};
use hyperreal::{Problem, Rational, Real};

use crate::boundary::{BoundaryError, CanonicalCycle, CanonicalStep, canonical_motion_segment};
use crate::compiler::{MachineCompileError, certified_u64_round, half_lattice_unit, quantize_axis};
use crate::machine_profile::{MachineDynamicsProfile2, MachineResolutionBudget2};
use crate::toolpath::{
    CertifiedMetricPath2, MetricPathApproximationLimits2, ToolpathError, certify_metric_path,
    promote_metric_path,
};

/// Result type for exact feed scheduling.
pub type MotionScheduleResult<T> = Result<T, MotionScheduleError>;

/// Which side of a configured travel interval rejected retained source geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TravelBoundary {
    /// The exact source envelope fell below the conservative usable minimum.
    Minimum,
    /// The exact source envelope exceeded the conservative usable maximum.
    Maximum,
}

/// Exact proof that the complete retained source envelope fits usable machine travel.
///
/// This certificate is derived from Hypercurve's native line/arc extrema, not
/// from interpolation samples. It therefore also covers an arc extremum that
/// lies between later V1 command points.
#[derive(Clone, Debug, PartialEq)]
pub struct CertifiedTravelEnvelope2 {
    source_minimum_mm: [Real; 2],
    source_maximum_mm: [Real; 2],
    usable_minimum_mm: [Real; 2],
    usable_maximum_mm: [Real; 2],
}

impl CertifiedTravelEnvelope2 {
    fn certify(
        source: &CurvePath2,
        profile: &MachineDynamicsProfile2,
    ) -> MotionScheduleResult<Self> {
        let bounds = source.bounds()?;
        let source_minimum_mm = [bounds.min_x().clone(), bounds.min_y().clone()];
        let source_maximum_mm = [bounds.max_x().clone(), bounds.max_y().clone()];
        let usable_minimum_mm = [
            Real::from(profile.axes()[0].usable_position_minimum_metres() * Rational::from(1_000)),
            Real::from(profile.axes()[1].usable_position_minimum_metres() * Rational::from(1_000)),
        ];
        let usable_maximum_mm = [
            Real::from(profile.axes()[0].usable_position_maximum_metres() * Rational::from(1_000)),
            Real::from(profile.axes()[1].usable_position_maximum_metres() * Rational::from(1_000)),
        ];

        for axis in 0..2 {
            match compare_reals(
                &source_minimum_mm[axis],
                &usable_minimum_mm[axis],
                PredicatePolicy::STRICT,
            )
            .value()
            {
                Some(Ordering::Less) => {
                    return Err(MotionScheduleError::TravelEnvelopeExceeded {
                        axis,
                        boundary: TravelBoundary::Minimum,
                    });
                }
                Some(Ordering::Equal | Ordering::Greater) => {}
                None => {
                    return Err(MotionScheduleError::TravelEnvelopePredicateUnresolved {
                        axis,
                        boundary: TravelBoundary::Minimum,
                    });
                }
            }
            match compare_reals(
                &source_maximum_mm[axis],
                &usable_maximum_mm[axis],
                PredicatePolicy::STRICT,
            )
            .value()
            {
                Some(Ordering::Greater) => {
                    return Err(MotionScheduleError::TravelEnvelopeExceeded {
                        axis,
                        boundary: TravelBoundary::Maximum,
                    });
                }
                Some(Ordering::Less | Ordering::Equal) => {}
                None => {
                    return Err(MotionScheduleError::TravelEnvelopePredicateUnresolved {
                        axis,
                        boundary: TravelBoundary::Maximum,
                    });
                }
            }
        }

        Ok(Self {
            source_minimum_mm,
            source_maximum_mm,
            usable_minimum_mm,
            usable_maximum_mm,
        })
    }

    /// Exact minimum source coordinate in millimetres for X and Y.
    pub const fn source_minimum_mm(&self) -> &[Real; 2] {
        &self.source_minimum_mm
    }

    /// Exact maximum source coordinate in millimetres for X and Y.
    pub const fn source_maximum_mm(&self) -> &[Real; 2] {
        &self.source_maximum_mm
    }

    /// Conservative usable machine minimum in millimetres for X and Y.
    pub const fn usable_minimum_mm(&self) -> &[Real; 2] {
        &self.usable_minimum_mm
    }

    /// Conservative usable machine maximum in millimetres for X and Y.
    pub const fn usable_maximum_mm(&self) -> &[Real; 2] {
        &self.usable_maximum_mm
    }
}

/// Conservative scalar path limits valid for every two-axis tangent direction.
#[derive(Clone, Debug, PartialEq)]
pub struct ScalarMotionLimits2 {
    maximum_feed_mm_per_second: Real,
    maximum_acceleration_mm_per_second_squared: Real,
    maximum_jerk_mm_per_second_cubed: Real,
    maximum_spatial_acceleration_mm_per_second_squared: Real,
}

impl ScalarMotionLimits2 {
    fn from_machine(
        profile: &MachineDynamicsProfile2,
        route: &[FeedPathElement],
    ) -> MotionScheduleResult<Self> {
        let maximum_feed = minimum_rational(
            profile.axes()[0]
                .effective_velocity_limit_metres_per_second()
                .clone(),
            profile.axes()[1]
                .effective_velocity_limit_metres_per_second()
                .clone(),
        ) * Rational::from(1_000);
        let maximum_acceleration = minimum_rational(
            profile.axes()[0]
                .effective_acceleration_limit_metres_per_second_squared()
                .clone(),
            profile.axes()[1]
                .effective_acceleration_limit_metres_per_second_squared()
                .clone(),
        ) * Rational::from(1_000);
        let maximum_jerk = minimum_rational(
            profile.axes()[0]
                .effective_jerk_limit_metres_per_second_cubed()
                .clone(),
            profile.axes()[1]
                .effective_jerk_limit_metres_per_second_cubed()
                .clone(),
        ) * Rational::from(1_000);
        let mut maximum_feed = Real::from(maximum_feed);
        let maximum_spatial_acceleration = Real::from(maximum_acceleration);
        let maximum_spatial_jerk = Real::from(maximum_jerk);
        let has_arc = route
            .iter()
            .any(|element| matches!(element, FeedPathElement::ExplicitArc(_)));
        let maximum_acceleration = if has_arc {
            (&maximum_spatial_acceleration / Real::from(2))?
        } else {
            maximum_spatial_acceleration.clone()
        };
        let maximum_jerk = if has_arc {
            (&maximum_spatial_jerk / Real::from(3))?
        } else {
            maximum_spatial_jerk.clone()
        };
        for element in route {
            let FeedPathElement::ExplicitArc(arc) = element else {
                continue;
            };
            let radius = arc.radius();
            let acceleration_feed = (&maximum_acceleration * radius).sqrt()?;
            let mixed_jerk_feed = (Real::from(2) * &maximum_spatial_jerk * radius
                / (Real::from(9) * &maximum_spatial_acceleration))?;
            let curvature_jerk_feed =
                ((&maximum_spatial_jerk * radius * radius / Real::from(3))?).cbrt()?;
            maximum_feed = minimum_real(
                maximum_feed,
                minimum_real(
                    acceleration_feed,
                    minimum_real(mixed_jerk_feed, curvature_jerk_feed)?,
                )?,
            )?;
        }
        Ok(Self {
            maximum_feed_mm_per_second: maximum_feed,
            maximum_acceleration_mm_per_second_squared: maximum_acceleration,
            maximum_jerk_mm_per_second_cubed: maximum_jerk,
            maximum_spatial_acceleration_mm_per_second_squared: maximum_spatial_acceleration,
        })
    }

    /// Conservative scalar path-feed ceiling in millimetres per second.
    pub const fn maximum_feed_mm_per_second(&self) -> &Real {
        &self.maximum_feed_mm_per_second
    }

    /// Conservative scalar path-acceleration ceiling in millimetres per second squared.
    pub const fn maximum_acceleration_mm_per_second_squared(&self) -> &Real {
        &self.maximum_acceleration_mm_per_second_squared
    }

    /// Conservative scalar path-jerk ceiling in millimetres per second cubed.
    pub const fn maximum_jerk_mm_per_second_cubed(&self) -> &Real {
        &self.maximum_jerk_mm_per_second_cubed
    }

    /// Full spatial acceleration envelope used for interpolation bounds.
    ///
    /// On curved routes, the scalar tangential limit is reduced so this bound
    /// also covers centripetal acceleration.
    pub const fn maximum_spatial_acceleration_mm_per_second_squared(&self) -> &Real {
        &self.maximum_spatial_acceleration_mm_per_second_squared
    }
}

/// Certified V1 schedule that stops at every retained metric-path join.
#[derive(Clone, Debug)]
pub struct CertifiedExactStopSchedule2 {
    configuration_digest: Digest,
    capability_digest: Digest,
    source: CurvePath2,
    metric_path: CertifiedMetricPath2,
    travel_envelope: CertifiedTravelEnvelope2,
    route: Vec<FeedPathElement>,
    tangent_spans: Vec<TangentSpan>,
    limits: ScalarMotionLimits2,
    lookahead: LookaheadFeedSchedule,
    lookahead_report: LookaheadFeedScheduleReport,
    phases: Vec<Vec<JerkRampPhaseProposal>>,
    jerk_report: MultiPhaseJerkRampFeedScheduleReport,
    total_path_length_mm: Real,
    total_traversal_time_seconds: Real,
}

/// One exact scheduled sample and its canonical machine coordinate/tick.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledMachinePoint2 {
    source_element: usize,
    motion_element: usize,
    phase_index: usize,
    subdivision_index: usize,
    exact_point_mm: CurvePoint2,
    ideal_time_seconds: Real,
    steps: [CanonicalStep; 2],
    tick: CanonicalCycle,
}

impl ScheduledMachinePoint2 {
    /// Zero-based retained source element.
    pub const fn source_element(&self) -> usize {
        self.source_element
    }

    /// Zero-based certified metric element used for actual motion.
    pub const fn motion_element(&self) -> usize {
        self.motion_element
    }

    /// Zero-based constant-jerk phase within that metric element.
    pub const fn phase_index(&self) -> usize {
        self.phase_index
    }

    /// Zero-based certified interpolation subdivision within the phase.
    pub const fn subdivision_index(&self) -> usize {
        self.subdivision_index
    }

    /// Exact certified metric-path point before command-lattice quantization.
    pub const fn exact_point_mm(&self) -> &CurvePoint2 {
        &self.exact_point_mm
    }

    /// Exact cumulative ideal schedule time.
    pub const fn ideal_time_seconds(&self) -> &Real {
        &self.ideal_time_seconds
    }

    /// Canonical absolute X/Y step coordinate.
    pub const fn steps(&self) -> [CanonicalStep; 2] {
        self.steps
    }

    /// Canonical cumulative device tick.
    pub const fn tick(&self) -> CanonicalCycle {
        self.tick
    }
}

/// Conservative schedule-to-firmware approximation evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledLoweringEvidence2 {
    maximum_source_to_motion_error_mm_exact: Rational,
    maximum_source_to_motion_error_mm: Real,
    requested_interpolation_error_mm_exact: Rational,
    requested_interpolation_error_mm: Real,
    maximum_chord_interpolation_error_mm: Real,
    maximum_axis_quantization_error_mm: [Real; 2],
    maximum_position_quantization_error_mm: Real,
    maximum_step_event_tracking_error_mm: Real,
    maximum_curve_to_canonical_error_mm: Real,
    maximum_timer_boundary_error_seconds: Real,
    maximum_segment_duration_error_seconds: Real,
}

/// Caller-owned memory bound for V1 schedule interpolation.
///
/// The limit counts retained scheduled points, including the initial point.
/// It is checked before reserving or appending each phase's interpolation
/// samples so an otherwise valid but pathological machine profile fails
/// closed instead of requesting unbounded browser memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledLoweringLimits {
    maximum_points: usize,
}

impl ScheduledLoweringLimits {
    /// Interactive browser policy for one lowered schedule.
    pub const INTERACTIVE: Self = Self {
        maximum_points: 131_072,
    };

    /// Construct a caller-owned scheduled-point limit.
    pub const fn try_new(maximum_points: usize) -> MotionScheduleResult<Self> {
        if maximum_points < 2 {
            return Err(MotionScheduleError::InvalidLoweringLimits);
        }
        Ok(Self { maximum_points })
    }

    /// Maximum retained scheduled points, including the initial point.
    pub const fn maximum_points(self) -> usize {
        self.maximum_points
    }
}

impl ScheduledLoweringEvidence2 {
    /// Certified source-curve to metric-motion bound as an exact rational.
    pub const fn maximum_source_to_motion_error_mm_exact(&self) -> &Rational {
        &self.maximum_source_to_motion_error_mm_exact
    }

    /// Certified source-curve to metric-motion positional bound.
    pub const fn maximum_source_to_motion_error_mm(&self) -> &Real {
        &self.maximum_source_to_motion_error_mm
    }

    /// Caller-owned path interpolation allocation as an exact rational.
    pub const fn requested_interpolation_error_mm_exact(&self) -> &Rational {
        &self.requested_interpolation_error_mm_exact
    }

    /// Caller-owned path interpolation allocation.
    pub const fn requested_interpolation_error_mm(&self) -> &Real {
        &self.requested_interpolation_error_mm
    }

    /// Certified acceleration/chord error bound before coordinate quantization.
    pub const fn maximum_chord_interpolation_error_mm(&self) -> &Real {
        &self.maximum_chord_interpolation_error_mm
    }

    /// Per-axis half-step command-lattice bounds.
    pub const fn maximum_axis_quantization_error_mm(&self) -> &[Real; 2] {
        &self.maximum_axis_quantization_error_mm
    }

    /// Euclidean command-lattice position bound.
    pub const fn maximum_position_quantization_error_mm(&self) -> &Real {
        &self.maximum_position_quantization_error_mm
    }

    /// Conservative within-segment DDA step-event tracking error.
    pub const fn maximum_step_event_tracking_error_mm(&self) -> &Real {
        &self.maximum_step_event_tracking_error_mm
    }

    /// Sum of source reduction, interpolation, and command-lattice bounds.
    pub const fn maximum_curve_to_canonical_error_mm(&self) -> &Real {
        &self.maximum_curve_to_canonical_error_mm
    }

    /// Half-tick cumulative boundary error.
    pub const fn maximum_timer_boundary_error_seconds(&self) -> &Real {
        &self.maximum_timer_boundary_error_seconds
    }

    /// One-tick segment-duration error.
    pub const fn maximum_segment_duration_error_seconds(&self) -> &Real {
        &self.maximum_segment_duration_error_seconds
    }
}

/// Canonical V1 constant-velocity IR approximation of a certified jerk schedule.
#[derive(Clone, Debug)]
pub struct CanonicalScheduledProgram2 {
    configuration_digest: Digest,
    capability_digest: Digest,
    source: CurvePath2,
    metric_path: CertifiedMetricPath2,
    timer_ticks_per_second: u64,
    output_quantum_cycles: u32,
    resolution_budget: MachineResolutionBudget2,
    points: Vec<ScheduledMachinePoint2>,
    segments: Vec<ExecutionSegment<2>>,
    executor_preflight: StepperPreflightSummary<2>,
    evidence: ScheduledLoweringEvidence2,
}

impl CanonicalScheduledProgram2 {
    /// Canonical machine configuration bound into this lowering.
    pub const fn configuration_digest(&self) -> Digest {
        self.configuration_digest
    }

    /// Immutable board capability bound into this lowering.
    pub const fn capability_digest(&self) -> Digest {
        self.capability_digest
    }

    /// Authoritative retained exact source path used to derive the schedule.
    pub const fn source(&self) -> &CurvePath2 {
        &self.source
    }

    /// Certified line/arc path actually scheduled against Hyperpath.
    pub const fn metric_path(&self) -> &CertifiedMetricPath2 {
        &self.metric_path
    }

    /// Exact integer device tick frequency.
    pub const fn timer_ticks_per_second(&self) -> u64 {
        self.timer_ticks_per_second
    }

    /// Exact backend output lattice against which executor preflight ran.
    pub const fn output_quantum_cycles(&self) -> u32 {
        self.output_quantum_cycles
    }

    /// Full machine-wide error budget under which lowering was admitted.
    pub const fn resolution_budget(&self) -> &MachineResolutionBudget2 {
        &self.resolution_budget
    }

    /// Exact scheduled samples and canonical coordinates.
    pub fn points(&self) -> &[ScheduledMachinePoint2] {
        &self.points
    }

    /// Firmware V1 constant-velocity segments.
    pub fn segments(&self) -> &[ExecutionSegment<2>] {
        &self.segments
    }

    /// Allocation-free replay through the production stepper executor's
    /// electrical timing and state-transition validator.
    pub const fn executor_preflight(&self) -> StepperPreflightSummary<2> {
        self.executor_preflight
    }

    /// Conservative interpolation, lattice, and timer evidence.
    pub const fn evidence(&self) -> &ScheduledLoweringEvidence2 {
        &self.evidence
    }
}

impl CertifiedExactStopSchedule2 {
    /// Canonical machine configuration for which the schedule was certified.
    pub const fn configuration_digest(&self) -> Digest {
        self.configuration_digest
    }

    /// Immutable board capability for which the schedule was certified.
    pub const fn capability_digest(&self) -> Digest {
        self.capability_digest
    }

    /// Exact Hypercurve source retained independently of display geometry.
    pub const fn source(&self) -> &CurvePath2 {
        &self.source
    }

    /// Certified line/arc path actually scheduled against Hyperpath.
    pub const fn metric_path(&self) -> &CertifiedMetricPath2 {
        &self.metric_path
    }

    /// Exact retained-path envelope certified inside conservative usable travel.
    pub const fn travel_envelope(&self) -> &CertifiedTravelEnvelope2 {
        &self.travel_envelope
    }

    /// Hyperpath elements promoted from the certified metric path.
    pub fn route(&self) -> &[FeedPathElement] {
        &self.route
    }

    /// Exact endpoints and tangent vectors used for lookahead classification.
    pub fn tangent_spans(&self) -> &[TangentSpan] {
        &self.tangent_spans
    }

    /// Conservative path-wide machine limits.
    pub const fn limits(&self) -> &ScalarMotionLimits2 {
        &self.limits
    }

    /// Exact zero-radius/zero-feed join proposal.
    pub const fn lookahead(&self) -> &LookaheadFeedSchedule {
        &self.lookahead
    }

    /// Hyperpath/Hypersolve replay of every join and span speed node.
    pub const fn lookahead_report(&self) -> &LookaheadFeedScheduleReport {
        &self.lookahead_report
    }

    /// Four exact constant-jerk phases for every retained metric element.
    pub fn phases(&self) -> &[Vec<JerkRampPhaseProposal>] {
        &self.phases
    }

    /// Hyperpath/Hypersolve replay of phase kinematics, limits, sums, and continuity.
    pub const fn jerk_report(&self) -> &MultiPhaseJerkRampFeedScheduleReport {
        &self.jerk_report
    }

    /// Exact retained metric path length in millimetres.
    pub const fn total_path_length_mm(&self) -> &Real {
        &self.total_path_length_mm
    }

    /// Exact sum of all certified phase durations in seconds.
    pub const fn total_traversal_time_seconds(&self) -> &Real {
        &self.total_traversal_time_seconds
    }
}

/// Generate and certify the conservative first-release jerk schedule.
///
/// Lines and arcs are preserved losslessly. A general cubic is reduced only by
/// [`certify_metric_path`]'s exact pointwise certificate under the machine's
/// source-curve allocation and caller-owned element/depth limits. Every
/// resulting metric join uses zero retained radius and zero feed. This
/// deliberately stops at certified
/// cubic chord boundaries: it is slower than future native curved feed but
/// does not carry an instantaneous direction change at nonzero velocity.
pub fn certify_exact_stop_jerk_schedule(
    source: &CurvePath2,
    profile: &MachineDynamicsProfile2,
    resolution_budget: &MachineResolutionBudget2,
    approximation_limits: MetricPathApproximationLimits2,
) -> MotionScheduleResult<CertifiedExactStopSchedule2> {
    if resolution_budget.configuration_digest() != profile.configuration_digest()
        || resolution_budget.capability_digest() != profile.capability_digest()
    {
        return Err(MotionScheduleError::MachineIdentityMismatch);
    }
    let metric_path = certify_metric_path(
        source,
        resolution_budget.source_curve_allocation_mm_exact().clone(),
        approximation_limits,
    )?;
    let route = promote_metric_path(metric_path.path())?;
    let travel_envelope = CertifiedTravelEnvelope2::certify(source, profile)?;
    let tangent_spans = route
        .iter()
        .map(tangent_span)
        .collect::<MotionScheduleResult<Vec<_>>>()?;
    let limits = ScalarMotionLimits2::from_machine(profile, &route)?;
    let corner_count = route.len().saturating_sub(1);
    let lookahead = LookaheadFeedSchedule {
        entry_feed: Real::zero(),
        corner_feeds: vec![Real::zero(); corner_count],
        corner_radii: vec![Real::zero(); corner_count],
        exit_feed: Real::zero(),
    };
    let lookahead_report = certify_lookahead_feed_schedule(
        &route,
        &tangent_spans,
        &lookahead,
        limits.maximum_feed_mm_per_second.clone(),
        limits.maximum_acceleration_mm_per_second_squared.clone(),
        PredicatePolicy::STRICT,
    )?;
    if !lookahead_report.all_satisfied() {
        return Err(MotionScheduleError::LookaheadUncertified {
            join: lookahead_report.corners.first_unsatisfied_join(),
            span: lookahead_report.first_unsatisfied_span(),
        });
    }

    let mut phases = Vec::with_capacity(route.len());
    let mut total_path_length_mm = Real::zero();
    let mut total_traversal_time_seconds = Real::zero();
    for element in &route {
        let length = element_length(element)?;
        total_path_length_mm += &length;
        let element_phases =
            symmetric_rest_to_rest_phases(&length, &limits, profile.timer_ticks_per_second())?;
        for phase in &element_phases {
            total_traversal_time_seconds += &phase.ramp.traversal_time;
        }
        phases.push(element_phases);
    }
    let jerk_report = certify_multi_phase_jerk_ramp_feed_schedule(
        &route,
        &phases,
        limits.maximum_feed_mm_per_second.clone(),
        limits.maximum_acceleration_mm_per_second_squared.clone(),
        limits.maximum_jerk_mm_per_second_cubed.clone(),
        PredicatePolicy::STRICT,
    )?;
    if !jerk_report.all_satisfied() {
        return Err(MotionScheduleError::JerkScheduleUncertified {
            element: jerk_report.first_unsatisfied_element(),
        });
    }

    Ok(CertifiedExactStopSchedule2 {
        configuration_digest: profile.configuration_digest(),
        capability_digest: profile.capability_digest(),
        source: source.clone(),
        metric_path,
        travel_envelope,
        route,
        tangent_spans,
        limits,
        lookahead,
        lookahead_report,
        phases,
        jerk_report,
        total_path_length_mm,
        total_traversal_time_seconds,
    })
}

/// Lower a certified schedule to V1 constant-velocity firmware segments.
///
/// A phase is divided into an exact integer number of equal time intervals.
/// The count is the smallest integer whose second-derivative chord bound
/// `A*dt²/8` is no greater than `maximum_interpolation_error_mm`, where `A`
/// is the conservative full spatial acceleration envelope. Source points are
/// evaluated exactly from the certified metric line/arc path, then coordinates
/// and cumulative times are independently rounded to the configured step and
/// tick lattices. The retained source-to-motion bound remains additive evidence.
pub fn lower_certified_schedule_to_v1(
    schedule: &CertifiedExactStopSchedule2,
    profile: &MachineDynamicsProfile2,
    resolution_budget: &MachineResolutionBudget2,
    maximum_interpolation_error_mm: Rational,
    limits: ScheduledLoweringLimits,
) -> MotionScheduleResult<CanonicalScheduledProgram2> {
    if schedule.configuration_digest != profile.configuration_digest()
        || schedule.capability_digest != profile.capability_digest()
        || resolution_budget.configuration_digest() != profile.configuration_digest()
        || resolution_budget.capability_digest() != profile.capability_digest()
    {
        return Err(MotionScheduleError::MachineIdentityMismatch);
    }
    if schedule.metric_path.maximum_source_error_mm_exact()
        > resolution_budget.source_curve_allocation_mm_exact()
    {
        return Err(MotionScheduleError::SourceApproximationAllocationExceeded);
    }
    if maximum_interpolation_error_mm <= Rational::zero() {
        return Err(MotionScheduleError::InvalidInterpolationError);
    }
    if maximum_interpolation_error_mm
        > resolution_budget
            .controller_interpolation_allocation_mm_exact()
            .clone()
    {
        return Err(MotionScheduleError::InterpolationAllocationExceeded);
    }
    let maximum_interpolation_error_mm_exact = maximum_interpolation_error_mm;
    let maximum_interpolation_error_mm = Real::from(maximum_interpolation_error_mm_exact.clone());
    let maximum_source_to_motion_error_mm_exact =
        schedule.metric_path.maximum_source_error_mm_exact().clone();
    let maximum_source_to_motion_error_mm =
        Real::from(maximum_source_to_motion_error_mm_exact.clone());
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
    let maximum_position_quantization_error_mm = (maximum_axis_quantization_error_mm[0].clone()
        * maximum_axis_quantization_error_mm[0].clone()
        + maximum_axis_quantization_error_mm[1].clone()
            * maximum_axis_quantization_error_mm[1].clone())
    .sqrt()?;
    let maximum_step_event_tracking_error_mm =
        resolution_budget.step_event_tracking_error_mm().clone();
    let maximum_curve_to_canonical_error_mm = maximum_source_to_motion_error_mm.clone()
        + maximum_interpolation_error_mm.clone()
        + maximum_position_quantization_error_mm.clone()
        + maximum_step_event_tracking_error_mm.clone();
    let timer_frequency = Real::from(profile.timer_ticks_per_second());
    let maximum_timer_boundary_error_seconds = (Real::one() / (Real::from(2) * &timer_frequency))?;
    let maximum_segment_duration_error_seconds = (Real::one() / &timer_frequency)?;

    let mut points = Vec::new();
    let mut cumulative_time = Real::zero();
    let mut maximum_chord_interpolation_error_mm = Real::zero();
    let start = schedule.metric_path.path().start().clone();
    let initial_source_element = schedule
        .metric_path
        .source_element_for_motion(0)
        .ok_or(MotionScheduleError::MetricPathMismatch)?;
    push_scheduled_point(
        &mut points,
        start,
        cumulative_time.clone(),
        ScheduledPointProvenance {
            source_element: initial_source_element,
            motion_element: 0,
            phase_index: 0,
            subdivision_index: 0,
        },
        profile,
        &maximum_axis_quantization_error_mm,
    )?;

    for (element_index, element_phases) in schedule.phases.iter().enumerate() {
        let source_element = schedule
            .metric_path
            .source_element_for_motion(element_index)
            .ok_or(MotionScheduleError::MetricPathMismatch)?;
        let element_length = element_length(&schedule.route[element_index])?;
        let mut element_length_cursor = Real::zero();
        for (phase_index, phase) in element_phases.iter().enumerate() {
            let phase_time = &phase.ramp.traversal_time;
            let required_subdivisions = ((schedule
                .limits
                .maximum_spatial_acceleration_mm_per_second_squared
                .clone()
                * phase_time
                * phase_time
                / (Real::from(8) * &maximum_interpolation_error_mm))?)
                .sqrt()?;
            let subdivisions_integer = required_subdivisions.ceil_certified()?;
            let subdivisions = usize::try_from(subdivisions_integer).map_err(|_| {
                MotionScheduleError::IntegerOverflow {
                    domain: "phase interpolation subdivision count",
                }
            })?;
            let subdivisions = subdivisions.max(1);
            let required_points = points.len().checked_add(subdivisions).ok_or(
                MotionScheduleError::IntegerOverflow {
                    domain: "scheduled point count",
                },
            )?;
            if required_points > limits.maximum_points {
                return Err(MotionScheduleError::PointBudgetExceeded {
                    required: required_points,
                    maximum: limits.maximum_points,
                });
            }
            points.try_reserve(subdivisions).map_err(|_| {
                MotionScheduleError::AllocationOverflow {
                    domain: "scheduled points",
                }
            })?;
            let subdivisions_real = Real::from(u64::try_from(subdivisions).map_err(|_| {
                MotionScheduleError::IntegerOverflow {
                    domain: "phase interpolation subdivision count",
                }
            })?);
            let interval_time = (phase_time / &subdivisions_real)?;
            let interval_error = (schedule
                .limits
                .maximum_spatial_acceleration_mm_per_second_squared
                .clone()
                * &interval_time
                * &interval_time
                / Real::from(8))?;
            match compare_reals(
                &interval_error,
                &maximum_interpolation_error_mm,
                PredicatePolicy::STRICT,
            )
            .value()
            {
                Some(Ordering::Less | Ordering::Equal) => {}
                Some(Ordering::Greater) | None => {
                    return Err(MotionScheduleError::InterpolationBoundUncertified);
                }
            }
            maximum_chord_interpolation_error_mm =
                maximum_real(maximum_chord_interpolation_error_mm, interval_error)?;

            for subdivision_index in 1..=subdivisions {
                let local_time = &interval_time
                    * Real::from(u64::try_from(subdivision_index).map_err(|_| {
                        MotionScheduleError::IntegerOverflow {
                            domain: "phase interpolation sample index",
                        }
                    })?);
                let phase_distance = jerk_phase_distance(&phase.ramp, &local_time)?;
                let path_distance = &element_length_cursor + phase_distance;
                let element_fraction = (path_distance / &element_length)?;
                let exact_point = metric_point_at_fraction(
                    schedule.metric_path.path(),
                    element_index,
                    &element_fraction,
                )?;
                let ideal_time = &cumulative_time + local_time;
                push_scheduled_point(
                    &mut points,
                    exact_point,
                    ideal_time,
                    ScheduledPointProvenance {
                        source_element,
                        motion_element: element_index,
                        phase_index,
                        subdivision_index,
                    },
                    profile,
                    &maximum_axis_quantization_error_mm,
                )?;
            }
            cumulative_time += phase_time;
            element_length_cursor += &phase.path_length;
        }
    }

    let mut segments = Vec::new();
    segments
        .try_reserve_exact(points.len().saturating_sub(1))
        .map_err(|_| MotionScheduleError::AllocationOverflow {
            domain: "canonical schedule segments",
        })?;
    for (segment_index, pair) in points.windows(2).enumerate() {
        if pair[1].tick <= pair[0].tick {
            return Err(MotionScheduleError::TickBoundaryCollapsed { segment_index });
        }
        let delta = [
            pair[1].steps[0]
                .get()
                .checked_sub(pair[0].steps[0].get())
                .ok_or(MotionScheduleError::IntegerOverflow {
                    domain: "axis step delta",
                })?,
            pair[1].steps[1]
                .get()
                .checked_sub(pair[0].steps[1].get())
                .ok_or(MotionScheduleError::IntegerOverflow {
                    domain: "axis step delta",
                })?,
        ];
        segments.push(canonical_motion_segment(
            pair[0].tick,
            pair[1].tick,
            [CanonicalStep::new(delta[0]), CanonicalStep::new(delta[1])],
        )?);
    }

    let initial_position = points
        .first()
        .map(|point| [point.steps[0].get(), point.steps[1].get()])
        .ok_or(MotionScheduleError::MetricPathMismatch)?;
    let executor_preflight =
        preflight_stepper_segments(profile.stepper_timing(0), initial_position, &segments)?;

    Ok(CanonicalScheduledProgram2 {
        configuration_digest: profile.configuration_digest(),
        capability_digest: profile.capability_digest(),
        source: schedule.source.clone(),
        metric_path: schedule.metric_path.clone(),
        timer_ticks_per_second: profile.timer_ticks_per_second(),
        output_quantum_cycles: profile.output_quantum_cycles(),
        resolution_budget: resolution_budget.clone(),
        points,
        segments,
        executor_preflight,
        evidence: ScheduledLoweringEvidence2 {
            maximum_source_to_motion_error_mm_exact,
            maximum_source_to_motion_error_mm,
            requested_interpolation_error_mm_exact: maximum_interpolation_error_mm_exact,
            requested_interpolation_error_mm: maximum_interpolation_error_mm,
            maximum_chord_interpolation_error_mm,
            maximum_axis_quantization_error_mm,
            maximum_position_quantization_error_mm,
            maximum_step_event_tracking_error_mm,
            maximum_curve_to_canonical_error_mm,
            maximum_timer_boundary_error_seconds,
            maximum_segment_duration_error_seconds,
        },
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScheduledPointProvenance {
    source_element: usize,
    motion_element: usize,
    phase_index: usize,
    subdivision_index: usize,
}

fn push_scheduled_point(
    points: &mut Vec<ScheduledMachinePoint2>,
    exact_point_mm: CurvePoint2,
    ideal_time_seconds: Real,
    provenance: ScheduledPointProvenance,
    profile: &MachineDynamicsProfile2,
    maximum_axis_quantization_error_mm: &[Real; 2],
) -> MotionScheduleResult<()> {
    let point_index = points.len();
    let (x, _) = quantize_axis(
        exact_point_mm.x(),
        profile.axes()[0]
            .command_density_steps_per_millimetre()
            .nominal(),
        &maximum_axis_quantization_error_mm[0],
        point_index,
        0,
    )?;
    let (y, _) = quantize_axis(
        exact_point_mm.y(),
        profile.axes()[1]
            .command_density_steps_per_millimetre()
            .nominal(),
        &maximum_axis_quantization_error_mm[1],
        point_index,
        1,
    )?;
    certify_canonical_position_inside_travel([x, y], profile, point_index)?;
    let tick = certified_u64_round(
        &(ideal_time_seconds.clone() * Real::from(profile.timer_ticks_per_second())),
        "scheduled timer boundary",
        point_index,
    )?;
    points.push(ScheduledMachinePoint2 {
        source_element: provenance.source_element,
        motion_element: provenance.motion_element,
        phase_index: provenance.phase_index,
        subdivision_index: provenance.subdivision_index,
        exact_point_mm,
        ideal_time_seconds,
        steps: [x, y],
        tick: CanonicalCycle::new(tick),
    });
    Ok(())
}

fn certify_canonical_position_inside_travel(
    steps: [CanonicalStep; 2],
    profile: &MachineDynamicsProfile2,
    point_index: usize,
) -> MotionScheduleResult<()> {
    for (axis, step) in steps.into_iter().enumerate() {
        let commanded_mm = (Real::from(step.get())
            / Real::from(
                profile.axes()[axis]
                    .command_density_steps_per_millimetre()
                    .nominal()
                    .clone(),
            ))?;
        let usable_minimum_mm = Real::from(
            profile.axes()[axis].usable_position_minimum_metres() * Rational::from(1_000),
        );
        let usable_maximum_mm = Real::from(
            profile.axes()[axis].usable_position_maximum_metres() * Rational::from(1_000),
        );
        for (boundary, limit, outside) in [
            (TravelBoundary::Minimum, usable_minimum_mm, Ordering::Less),
            (
                TravelBoundary::Maximum,
                usable_maximum_mm,
                Ordering::Greater,
            ),
        ] {
            match compare_reals(&commanded_mm, &limit, PredicatePolicy::STRICT).value() {
                Some(ordering) if ordering == outside => {
                    return Err(MotionScheduleError::CanonicalTravelExceeded {
                        point_index,
                        axis,
                        boundary,
                    });
                }
                Some(_) => {}
                None => {
                    return Err(MotionScheduleError::CanonicalTravelPredicateUnresolved {
                        point_index,
                        axis,
                        boundary,
                    });
                }
            }
        }
    }
    Ok(())
}

fn jerk_phase_distance(
    phase: &JerkRampSpanProposal,
    local_time: &Real,
) -> MotionScheduleResult<Real> {
    let duration = &phase.traversal_time;
    let jerk = ((&phase.end_acceleration - &phase.start_acceleration) / duration)?;
    let time_squared = local_time * local_time;
    let time_cubed = &time_squared * local_time;
    Ok(&phase.start_feed * local_time
        + (&phase.start_acceleration * time_squared / Real::from(2))?
        + (jerk * time_cubed / Real::from(6))?)
}

fn metric_point_at_fraction(
    metric_path: &CurvePath2,
    element_index: usize,
    fraction: &Real,
) -> MotionScheduleResult<CurvePoint2> {
    let curve = metric_path
        .curves()
        .get(element_index)
        .ok_or(MotionScheduleError::MetricPathMismatch)?;
    match curve.geometry() {
        CurveGeometry2::Line(line) => Ok(line.point_at(fraction.clone())),
        CurveGeometry2::CircularArc(arc) => {
            match arc.point_at_sweep_fraction(fraction, &CurveContext::STRICT)? {
                Classification::Decided(point) => Ok(point),
                Classification::Uncertain(_) => Err(MotionScheduleError::MetricEvaluationUncertain),
            }
        }
        _ => Err(MotionScheduleError::UnsupportedMetricElement),
    }
}

fn symmetric_rest_to_rest_phases(
    length: &Real,
    limits: &ScalarMotionLimits2,
    timer_ticks_per_second: u64,
) -> MotionScheduleResult<Vec<JerkRampPhaseProposal>> {
    require_positive(length, "retained element length")?;
    let feed_time = (length / (Real::from(2) * limits.maximum_feed_mm_per_second.clone()))?;
    let acceleration_time = ((length
        / (Real::from(2) * limits.maximum_acceleration_mm_per_second_squared.clone()))?)
    .sqrt()?;
    let jerk_time =
        ((length / (Real::from(2) * limits.maximum_jerk_mm_per_second_cubed.clone()))?).cbrt()?;
    let required_phase_time = maximum_real(feed_time, maximum_real(acceleration_time, jerk_time)?)?;
    let phase_ticks =
        (required_phase_time * Real::from(timer_ticks_per_second)).ceil_certified()?;
    let phase_time = (Real::integer(phase_ticks) / Real::from(timer_ticks_per_second))?;
    require_positive(&phase_time, "selected jerk phase time")?;

    // Defining states from retained length makes the source-length identity
    // primary. Hyperpath independently reconstructs each phase's constant
    // jerk from the acceleration delta and traversal time.
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

fn tangent_span(element: &FeedPathElement) -> MotionScheduleResult<TangentSpan> {
    match element {
        FeedPathElement::Line(line) => Ok(TangentSpan::from_line_segment(line)),
        FeedPathElement::ExplicitArc(arc) => Ok(TangentSpan::from_explicit_arc(arc)),
        FeedPathElement::CubicPh(_) | FeedPathElement::QuinticPh(_) => {
            Err(MotionScheduleError::UnsupportedTangentCarrier)
        }
    }
}

fn element_length(element: &FeedPathElement) -> MotionScheduleResult<Real> {
    match element {
        FeedPathElement::Line(line) => Ok(line.euclidean_length()?),
        FeedPathElement::ExplicitArc(arc) => arc
            .certified_sweep_length()
            .ok_or(MotionScheduleError::UnsupportedMetricElement),
        FeedPathElement::CubicPh(curve) => Ok(curve.exact_length()),
        FeedPathElement::QuinticPh(curve) => Ok(curve.exact_length()),
    }
}

fn require_positive(value: &Real, domain: &'static str) -> MotionScheduleResult<()> {
    match classify_real_sign(value, PredicatePolicy::STRICT).value() {
        Some(Sign::Positive) => Ok(()),
        Some(Sign::Negative | Sign::Zero) => Err(MotionScheduleError::NonPositiveValue { domain }),
        None => Err(MotionScheduleError::PredicateUnresolved { domain }),
    }
}

fn maximum_real(left: Real, right: Real) -> MotionScheduleResult<Real> {
    match compare_reals(&left, &right, PredicatePolicy::STRICT).value() {
        Some(Ordering::Less) => Ok(right),
        Some(Ordering::Equal | Ordering::Greater) => Ok(left),
        None => Err(MotionScheduleError::PredicateUnresolved {
            domain: "jerk phase time maximum",
        }),
    }
}

fn minimum_real(left: Real, right: Real) -> MotionScheduleResult<Real> {
    match compare_reals(&left, &right, PredicatePolicy::STRICT).value() {
        Some(Ordering::Less | Ordering::Equal) => Ok(left),
        Some(Ordering::Greater) => Ok(right),
        None => Err(MotionScheduleError::PredicateUnresolved {
            domain: "curved-path scalar limit minimum",
        }),
    }
}

fn minimum_rational(left: Rational, right: Rational) -> Rational {
    if left <= right { left } else { right }
}

/// Failure to construct or certify an exact motion schedule.
#[derive(Debug)]
pub enum MotionScheduleError {
    /// Exact Hypercurve-to-Hyperpath promotion failed.
    Toolpath(ToolpathError),
    /// Hyperpath rejected the proposed route or schedule shape.
    Route(RouteCertificationError),
    /// Hyper exact arithmetic rejected a root or division.
    Arithmetic(Problem),
    /// Existing exact-to-machine compiler boundary rejected a value.
    MachineCompile(MachineCompileError),
    /// Hypercurve rejected exact source evaluation.
    CurveEvaluation(CurveError),
    /// Hypercurve could not certify an exact source envelope.
    SourceBounds(ExactCurveError),
    /// Canonical firmware segment construction rejected a boundary.
    CanonicalBoundary(BoundaryError),
    /// The production stepper executor rejected electrical timing or state transitions.
    ExecutorPreflight(MotionError),
    /// The current route carried a PH tangent form not yet connected here.
    UnsupportedTangentCarrier,
    /// A retained element lacked an exact supported metric length.
    UnsupportedMetricElement,
    /// Certified metric path, route, phases, or provenance diverged.
    MetricPathMismatch,
    /// Exact metric-path evaluation remained undecided.
    MetricEvaluationUncertain,
    /// The complete exact source envelope lies outside conservative usable travel.
    TravelEnvelopeExceeded {
        /// Dense machine axis index.
        axis: usize,
        /// Rejected side of the configured interval.
        boundary: TravelBoundary,
    },
    /// Exact comparison between source envelope and usable travel remained undecided.
    TravelEnvelopePredicateUnresolved {
        /// Dense machine axis index.
        axis: usize,
        /// Undecided side of the configured interval.
        boundary: TravelBoundary,
    },
    /// A rounded canonical command coordinate lies outside conservative usable travel.
    CanonicalTravelExceeded {
        /// Zero-based scheduled point index.
        point_index: usize,
        /// Dense machine axis index.
        axis: usize,
        /// Rejected side of the configured interval.
        boundary: TravelBoundary,
    },
    /// A rounded command/travel comparison remained undecided.
    CanonicalTravelPredicateUnresolved {
        /// Zero-based scheduled point index.
        point_index: usize,
        /// Dense machine axis index.
        axis: usize,
        /// Undecided side of the configured interval.
        boundary: TravelBoundary,
    },
    /// A required physical or phase value was not positive.
    NonPositiveValue {
        /// Value domain.
        domain: &'static str,
    },
    /// An exact sign/order predicate remained unresolved.
    PredicateUnresolved {
        /// Predicate domain.
        domain: &'static str,
    },
    /// Schedule and machine configuration/capability identities differed.
    MachineIdentityMismatch,
    /// Interpolation error must be strictly positive.
    InvalidInterpolationError,
    /// Requested V1 interpolation exceeded its machine-wide allocation.
    InterpolationAllocationExceeded,
    /// The schedule's certified source reduction exceeded the lowering budget.
    SourceApproximationAllocationExceeded,
    /// A lowering policy must retain at least an initial and final point.
    InvalidLoweringLimits,
    /// The proposed interpolation exceeds the caller-owned point budget.
    PointBudgetExceeded {
        /// Number of points required after the current phase.
        required: usize,
        /// Maximum number of retained points allowed by the caller.
        maximum: usize,
    },
    /// A bounded allocation could not be represented or reserved.
    AllocationOverflow {
        /// Allocation domain.
        domain: &'static str,
    },
    /// An integer lattice boundary exceeded its storage representation.
    IntegerOverflow {
        /// Boundary domain.
        domain: &'static str,
    },
    /// The acceleration-based interpolation predicate failed.
    InterpolationBoundUncertified,
    /// A positive scheduled interval collapsed on the device tick lattice.
    TickBoundaryCollapsed {
        /// Zero-based V1 segment index.
        segment_index: usize,
    },
    /// At least one lookahead constraint did not certify.
    LookaheadUncertified {
        /// First failed join, if any.
        join: Option<usize>,
        /// First failed retained span, if any.
        span: Option<usize>,
    },
    /// At least one jerk phase, sum, or continuity condition did not certify.
    JerkScheduleUncertified {
        /// First failed retained element, if any.
        element: Option<usize>,
    },
}

impl fmt::Display for MotionScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toolpath(source) => write!(formatter, "metric path promotion failed: {source}"),
            Self::Route(source) => {
                write!(formatter, "Hyperpath schedule replay failed: {source:?}")
            }
            Self::Arithmetic(source) => {
                write!(formatter, "exact schedule arithmetic failed: {source}")
            }
            Self::MachineCompile(source) => {
                write!(formatter, "exact machine lowering failed: {source}")
            }
            Self::CurveEvaluation(source) => {
                write!(formatter, "exact source evaluation failed: {source}")
            }
            Self::SourceBounds(source) => {
                write!(
                    formatter,
                    "exact source envelope certification failed: {source}"
                )
            }
            Self::CanonicalBoundary(source) => {
                write!(formatter, "canonical firmware boundary failed: {source}")
            }
            Self::ExecutorPreflight(source) => {
                write!(formatter, "stepper executor preflight failed: {source:?}")
            }
            Self::UnsupportedTangentCarrier => formatter
                .write_str("the retained metric element has no connected exact tangent carrier"),
            Self::UnsupportedMetricElement => {
                formatter.write_str("the retained path element has no exact supported length")
            }
            Self::MetricPathMismatch => formatter
                .write_str("certified metric path, route, phases, and provenance do not match"),
            Self::MetricEvaluationUncertain => {
                formatter.write_str("exact metric-path evaluation remained uncertain")
            }
            Self::TravelEnvelopeExceeded { axis, boundary } => write!(
                formatter,
                "exact source envelope exceeds the usable axis {axis} {boundary:?} travel boundary"
            ),
            Self::TravelEnvelopePredicateUnresolved { axis, boundary } => write!(
                formatter,
                "exact source envelope comparison remained unresolved at axis {axis} {boundary:?} travel boundary"
            ),
            Self::CanonicalTravelExceeded {
                point_index,
                axis,
                boundary,
            } => write!(
                formatter,
                "canonical point {point_index} exceeds the usable axis {axis} {boundary:?} travel boundary after step rounding"
            ),
            Self::CanonicalTravelPredicateUnresolved {
                point_index,
                axis,
                boundary,
            } => write!(
                formatter,
                "canonical point {point_index} comparison remained unresolved at axis {axis} {boundary:?} travel boundary"
            ),
            Self::NonPositiveValue { domain } => {
                write!(formatter, "{domain} is not strictly positive")
            }
            Self::PredicateUnresolved { domain } => {
                write!(
                    formatter,
                    "exact schedule predicate remained unresolved for {domain}"
                )
            }
            Self::MachineIdentityMismatch => formatter
                .write_str("certified schedule and machine profile identities do not match"),
            Self::InvalidInterpolationError => {
                formatter.write_str("interpolation error must be strictly positive")
            }
            Self::InterpolationAllocationExceeded => formatter
                .write_str("V1 interpolation error exceeds its certified machine-wide allocation"),
            Self::SourceApproximationAllocationExceeded => formatter.write_str(
                "certified source-to-motion error exceeds the machine-wide source allocation",
            ),
            Self::InvalidLoweringLimits => {
                formatter.write_str("scheduled lowering requires a point budget of at least two")
            }
            Self::PointBudgetExceeded { required, maximum } => write!(
                formatter,
                "scheduled lowering requires {required} points but the caller permits {maximum}"
            ),
            Self::AllocationOverflow { domain } => {
                write!(formatter, "bounded allocation failed for {domain}")
            }
            Self::IntegerOverflow { domain } => {
                write!(formatter, "{domain} exceeded canonical integer storage")
            }
            Self::InterpolationBoundUncertified => {
                formatter.write_str("the acceleration-based V1 interpolation bound did not certify")
            }
            Self::TickBoundaryCollapsed { segment_index } => write!(
                formatter,
                "scheduled interval {segment_index} collapsed on the timer lattice"
            ),
            Self::LookaheadUncertified { join, span } => write!(
                formatter,
                "lookahead proposal did not certify (join {join:?}, span {span:?})"
            ),
            Self::JerkScheduleUncertified { element } => write!(
                formatter,
                "jerk schedule did not certify at retained element {element:?}"
            ),
        }
    }
}

impl StdError for MotionScheduleError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Toolpath(source) => Some(source),
            Self::Arithmetic(source) => Some(source),
            Self::MachineCompile(source) => Some(source),
            Self::CurveEvaluation(source) => Some(source),
            Self::SourceBounds(source) => Some(source),
            Self::CanonicalBoundary(source) => Some(source),
            _ => None,
        }
    }
}

impl From<ToolpathError> for MotionScheduleError {
    fn from(value: ToolpathError) -> Self {
        Self::Toolpath(value)
    }
}

impl From<RouteCertificationError> for MotionScheduleError {
    fn from(value: RouteCertificationError) -> Self {
        Self::Route(value)
    }
}

impl From<Problem> for MotionScheduleError {
    fn from(value: Problem) -> Self {
        Self::Arithmetic(value)
    }
}

impl From<MachineCompileError> for MotionScheduleError {
    fn from(value: MachineCompileError) -> Self {
        Self::MachineCompile(value)
    }
}

impl From<CurveError> for MotionScheduleError {
    fn from(value: CurveError) -> Self {
        Self::CurveEvaluation(value)
    }
}

impl From<ExactCurveError> for MotionScheduleError {
    fn from(value: ExactCurveError) -> Self {
        Self::SourceBounds(value)
    }
}

impl From<BoundaryError> for MotionScheduleError {
    fn from(value: BoundaryError) -> Self {
        Self::CanonicalBoundary(value)
    }
}

impl From<MotionError> for MotionScheduleError {
    fn from(value: MotionError) -> Self {
        Self::ExecutorPreflight(value)
    }
}
