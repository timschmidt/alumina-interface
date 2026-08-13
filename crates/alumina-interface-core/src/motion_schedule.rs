//! Path-wide exact lookahead and certified jerk-limited feed scheduling.
//!
//! The first schedule policy retains the exact Hypercurve source and admits a
//! separate metric line/arc path only through lossless promotion or a bounded
//! pointwise certificate over exact Hypercurve de Casteljau spans. Entry, exit,
//! every true corner, every reversal, and every certified cubic chord boundary
//! remain exact stops. Lossless line-to-line G1 joins may retain positive feed
//! after Hyperpath's acceleration lookahead and bounded component-local jerk
//! refinement. A zero/zero element uses the four-phase symmetric rest-to-rest
//! profile; every nonzero element uses Hyperpath's exact two-phase monotonic transition.
//! Hyperpath and Hypersolve replay every lookahead, phase, length, continuity,
//! feed, acceleration, and jerk condition before a schedule is exposed. No
//! sampled display chord is used as path geometry.

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
    AffineSpanAxisProjection, AxisMotionLimits, FeedPathElement, JerkRampPhaseProposal,
    JerkRampSpanProposal, LookaheadFeedPlanningLimits, LookaheadFeedSchedule,
    LookaheadFeedScheduleReport, MultiPhaseJerkRampFeedScheduleReport,
    PlannedAxisProjectedMotionLimits, PlannedJerkFeasibleLookaheadSchedule,
    PlannedLookaheadFeedSchedule, RouteCertificationError, TangentSpan,
    certify_multi_phase_jerk_ramp_feed_schedule, plan_axis_projected_motion_limits,
    plan_jerk_feasible_lookahead_schedule,
};
use hyperreal::{Problem, Rational, Real};

use crate::boundary::{BoundaryError, CanonicalCycle, CanonicalStep, canonical_motion_segment};
use crate::compiler::{MachineCompileError, half_lattice_unit, quantize_axis};
use crate::machine_profile::{MachineDynamicsProfile2, MachineResolutionBudget2};
use crate::toolpath::{
    CertifiedMetricPath2, MetricPathApproximationLimits2, ToolpathError, certify_metric_path,
    promote_metric_path,
};

/// Result type for exact feed scheduling.
pub type MotionScheduleResult<T> = Result<T, MotionScheduleError>;

const MAXIMUM_JERK_COMPONENT_HALVINGS: u32 = 64;

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
    affine_axis_projection: Option<PlannedAxisProjectedMotionLimits>,
}

impl ScalarMotionLimits2 {
    fn from_machine(
        profile: &MachineDynamicsProfile2,
        route: &[FeedPathElement],
    ) -> MotionScheduleResult<Self> {
        let conservative_maximum_feed = minimum_rational(
            profile.axes()[0]
                .effective_velocity_limit_metres_per_second()
                .clone(),
            profile.axes()[1]
                .effective_velocity_limit_metres_per_second()
                .clone(),
        ) * Rational::from(1_000);
        let conservative_maximum_acceleration = minimum_rational(
            profile.axes()[0]
                .effective_acceleration_limit_metres_per_second_squared()
                .clone(),
            profile.axes()[1]
                .effective_acceleration_limit_metres_per_second_squared()
                .clone(),
        ) * Rational::from(1_000);
        let conservative_maximum_jerk = minimum_rational(
            profile.axes()[0]
                .effective_jerk_limit_metres_per_second_cubed()
                .clone(),
            profile.axes()[1]
                .effective_jerk_limit_metres_per_second_cubed()
                .clone(),
        ) * Rational::from(1_000);
        let axis_limits = profile
            .axes()
            .iter()
            .map(|axis| AxisMotionLimits {
                maximum_velocity: Real::from(
                    axis.effective_velocity_limit_metres_per_second() * Rational::from(1_000),
                ),
                maximum_acceleration: Real::from(
                    axis.effective_acceleration_limit_metres_per_second_squared()
                        * Rational::from(1_000),
                ),
                maximum_jerk: Real::from(
                    axis.effective_jerk_limit_metres_per_second_cubed() * Rational::from(1_000),
                ),
            })
            .collect::<Vec<_>>();
        let affine_axis_projection = affine_line_axis_projection(route, &axis_limits)?;
        let (mut maximum_feed, maximum_spatial_acceleration, maximum_spatial_jerk) =
            if let Some(projection) = &affine_axis_projection {
                (
                    projection.maximum_path_feed.clone(),
                    projection.maximum_path_acceleration.clone(),
                    projection.maximum_path_jerk.clone(),
                )
            } else {
                (
                    Real::from(conservative_maximum_feed),
                    Real::from(conservative_maximum_acceleration),
                    Real::from(conservative_maximum_jerk),
                )
            };
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
            affine_axis_projection,
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

    /// Exact dense-axis projection when every retained metric element is affine.
    ///
    /// Mixed or curved routes return `None` and retain the conservative
    /// direction-independent machine-wide minima until their higher-derivative
    /// terms have a separate certificate.
    pub fn affine_axis_projection(&self) -> Option<&PlannedAxisProjectedMotionLimits> {
        self.affine_axis_projection.as_ref()
    }
}

fn affine_line_axis_projection(
    route: &[FeedPathElement],
    axis_limits: &[AxisMotionLimits],
) -> MotionScheduleResult<Option<PlannedAxisProjectedMotionLimits>> {
    if !route
        .iter()
        .all(|element| matches!(element, FeedPathElement::Line(_)))
    {
        return Ok(None);
    }
    let projections = route
        .iter()
        .map(|element| {
            let FeedPathElement::Line(line) = element else {
                return Err(MotionScheduleError::UnsupportedMetricElement);
            };
            let direction = line.direction_vector();
            let length = line.euclidean_length()?;
            let absolute_x = absolute_real(
                &direction.x,
                "affine line X derivative sign for axis projection",
            )?;
            let absolute_y = absolute_real(
                &direction.y,
                "affine line Y derivative sign for axis projection",
            )?;
            Ok(AffineSpanAxisProjection {
                absolute_axis_derivatives: vec![(&absolute_x / &length)?, (&absolute_y / &length)?],
            })
        })
        .collect::<MotionScheduleResult<Vec<_>>>()?;
    Ok(Some(plan_axis_projected_motion_limits(
        &projections,
        axis_limits,
        PredicatePolicy::STRICT,
    )?))
}

fn absolute_real(value: &Real, domain: &'static str) -> MotionScheduleResult<Real> {
    match classify_real_sign(value, PredicatePolicy::STRICT).value() {
        Some(Sign::Negative) => Ok(-value.clone()),
        Some(Sign::Zero | Sign::Positive) => Ok(value.clone()),
        None => Err(MotionScheduleError::PredicateUnresolved { domain }),
    }
}

/// Certified V1 schedule with positive feed only across lossless line-to-line G1 joins.
#[derive(Clone, Debug)]
pub struct CertifiedJerkSchedule2 {
    configuration_digest: Digest,
    capability_digest: Digest,
    source: CurvePath2,
    metric_path: CertifiedMetricPath2,
    travel_envelope: CertifiedTravelEnvelope2,
    route: Vec<FeedPathElement>,
    tangent_spans: Vec<TangentSpan>,
    limits: ScalarMotionLimits2,
    lookahead_plan: PlannedJerkFeasibleLookaheadSchedule,
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
    timer_lattice_schedule: TimerLatticeScheduleReport2,
}

/// Exact report for one-sided timer quantization and bounded time dilation.
///
/// The selected factor is the smallest value on the caller's rational factor
/// lattice whose complete canonical stream passes the unchanged production
/// stepper preflight. Every individual segment duration is rounded upward to
/// the configured output quantum after applying that factor, so no segment is
/// shorter than its retained ideal schedule interval.
#[derive(Clone, Debug, PartialEq)]
pub struct TimerLatticeScheduleReport2 {
    selected_factor_numerator: u32,
    selected_factor_denominator: u32,
    maximum_factor_numerator: u32,
    candidate_replays: u32,
    unit_factor_rejection: Option<MotionError>,
    predecessor_rejection: Option<MotionError>,
    ideal_total_time_seconds: Real,
    scheduled_total_time_seconds: Real,
    maximum_cumulative_delay_seconds: Real,
    maximum_segment_extension_seconds: Real,
    maximum_output_grid_padding_seconds: Real,
}

impl TimerLatticeScheduleReport2 {
    /// Exact selected time-dilation factor.
    pub fn selected_factor(&self) -> Rational {
        Rational::fraction(
            i64::from(self.selected_factor_numerator),
            u64::from(self.selected_factor_denominator),
        )
        .expect("validated timer-dilation factor remains a positive rational")
    }

    /// Numerator on the caller-selected factor lattice.
    pub const fn selected_factor_numerator(&self) -> u32 {
        self.selected_factor_numerator
    }

    /// Denominator defining the caller-selected factor resolution.
    pub const fn selected_factor_denominator(&self) -> u32 {
        self.selected_factor_denominator
    }

    /// Inclusive caller-owned search ceiling numerator on the same lattice.
    pub const fn maximum_factor_numerator(&self) -> u32 {
        self.maximum_factor_numerator
    }

    /// Number of complete production-preflight candidate replays.
    pub const fn candidate_replays(&self) -> u32 {
        self.candidate_replays
    }

    /// Exact production failure at factor one, if dilation was required.
    pub const fn unit_factor_rejection(&self) -> Option<MotionError> {
        self.unit_factor_rejection
    }

    /// Exact failure at the immediately smaller factor-grid value.
    ///
    /// This is `None` only when factor one already passed.
    pub const fn predecessor_rejection(&self) -> Option<MotionError> {
        self.predecessor_rejection
    }

    /// Retained exact traversal time before output-grid lowering.
    pub const fn ideal_total_time_seconds(&self) -> &Real {
        &self.ideal_total_time_seconds
    }

    /// Exact canonical end tick converted back to seconds.
    pub const fn scheduled_total_time_seconds(&self) -> &Real {
        &self.scheduled_total_time_seconds
    }

    /// Largest nonnegative scheduled-minus-ideal cumulative delay.
    pub const fn maximum_cumulative_delay_seconds(&self) -> &Real {
        &self.maximum_cumulative_delay_seconds
    }

    /// Largest nonnegative extension of one retained ideal interval.
    pub const fn maximum_segment_extension_seconds(&self) -> &Real {
        &self.maximum_segment_extension_seconds
    }

    /// Largest exact per-segment padding introduced solely by ceiling to the
    /// output quantum after applying the selected factor.
    pub const fn maximum_output_grid_padding_seconds(&self) -> &Real {
        &self.maximum_output_grid_padding_seconds
    }
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
    timer_dilation_policy: TimerDilationPolicy,
}

/// Caller-owned rational lattice and search ceiling for exact timer dilation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerDilationPolicy {
    factor_denominator: u32,
    maximum_factor_numerator: u32,
}

impl TimerDilationPolicy {
    /// Browser policy: factors in increments of `1/4096`, up to exactly 16.
    pub const INTERACTIVE: Self = Self {
        factor_denominator: 4_096,
        maximum_factor_numerator: 65_536,
    };

    /// Constructs a bounded exact factor lattice. Factor one is always the
    /// first candidate, so the maximum numerator must be at least the
    /// denominator.
    pub const fn try_new(
        factor_denominator: u32,
        maximum_factor_numerator: u32,
    ) -> MotionScheduleResult<Self> {
        if factor_denominator == 0 || maximum_factor_numerator < factor_denominator {
            return Err(MotionScheduleError::InvalidTimerDilationPolicy);
        }
        Ok(Self {
            factor_denominator,
            maximum_factor_numerator,
        })
    }

    /// Denominator of every candidate factor.
    pub const fn factor_denominator(self) -> u32 {
        self.factor_denominator
    }

    /// Inclusive largest candidate numerator.
    pub const fn maximum_factor_numerator(self) -> u32 {
        self.maximum_factor_numerator
    }
}

impl ScheduledLoweringLimits {
    /// Interactive browser policy for one lowered schedule.
    pub const INTERACTIVE: Self = Self {
        maximum_points: 131_072,
        timer_dilation_policy: TimerDilationPolicy::INTERACTIVE,
    };

    /// Construct a caller-owned scheduled-point limit.
    pub const fn try_new(maximum_points: usize) -> MotionScheduleResult<Self> {
        if maximum_points < 2 {
            return Err(MotionScheduleError::InvalidLoweringLimits);
        }
        Ok(Self {
            maximum_points,
            timer_dilation_policy: TimerDilationPolicy::INTERACTIVE,
        })
    }

    /// Construct caller-owned point and timer-dilation bounds.
    pub const fn try_new_with_timer_dilation(
        maximum_points: usize,
        timer_dilation_policy: TimerDilationPolicy,
    ) -> MotionScheduleResult<Self> {
        if maximum_points < 2 {
            return Err(MotionScheduleError::InvalidLoweringLimits);
        }
        Ok(Self {
            maximum_points,
            timer_dilation_policy,
        })
    }

    /// Maximum retained scheduled points, including the initial point.
    pub const fn maximum_points(self) -> usize {
        self.maximum_points
    }

    /// Exact factor lattice and maximum accepted dilation.
    pub const fn timer_dilation_policy(self) -> TimerDilationPolicy {
        self.timer_dilation_policy
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

    /// Exact timer/output-lattice selection and production replay report.
    pub const fn timer_lattice_schedule(&self) -> &TimerLatticeScheduleReport2 {
        &self.timer_lattice_schedule
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

impl CertifiedJerkSchedule2 {
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

    /// Exact zero-radius schedule selected by lookahead and jerk refinement.
    pub const fn lookahead(&self) -> &LookaheadFeedSchedule {
        &self.lookahead_plan.schedule
    }

    /// Exact acceleration proposal, jerk refinement, final nodes, and replay.
    pub const fn lookahead_plan(&self) -> &PlannedJerkFeasibleLookaheadSchedule {
        &self.lookahead_plan
    }

    /// Original exact node ceilings, forward pass, and reverse-pass proposal.
    pub const fn acceleration_lookahead_plan(&self) -> &PlannedLookaheadFeedSchedule {
        &self.lookahead_plan.acceleration_plan
    }

    /// Hyperpath/Hypersolve replay of every join and span speed node.
    pub const fn lookahead_report(&self) -> &LookaheadFeedScheduleReport {
        &self.lookahead_plan.lookahead_certification
    }

    /// Exact constant-jerk phases for every retained metric element.
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
/// source-curve allocation and caller-owned element/depth limits. Every join
/// uses zero retained radius. Lossless exact line-to-line G1 continuations may
/// retain positive feed; curvature-bearing joins, true corners, reversals, and
/// certified cubic chord boundaries stop. This is slower than future retained
/// blends or native curved feed but never carries an instantaneous direction
/// change or uncertified curvature discontinuity at nonzero velocity.
pub fn certify_jerk_schedule(
    source: &CurvePath2,
    profile: &MachineDynamicsProfile2,
    resolution_budget: &MachineResolutionBudget2,
    approximation_limits: MetricPathApproximationLimits2,
) -> MotionScheduleResult<CertifiedJerkSchedule2> {
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
    let lookahead_limits = LookaheadFeedPlanningLimits {
        maximum_entry_feed: Real::zero(),
        maximum_corner_feeds: (0..corner_count)
            .map(|join_index| {
                if lossless_line_join_can_move(&metric_path, &route, join_index) {
                    limits.maximum_feed_mm_per_second.clone()
                } else {
                    Real::zero()
                }
            })
            .collect(),
        corner_radii: vec![Real::zero(); corner_count],
        maximum_exit_feed: Real::zero(),
    };
    let lookahead_plan = plan_jerk_feasible_lookahead_schedule(
        &route,
        &tangent_spans,
        &lookahead_limits,
        limits.maximum_feed_mm_per_second.clone(),
        limits.maximum_acceleration_mm_per_second_squared.clone(),
        limits.maximum_jerk_mm_per_second_cubed.clone(),
        MAXIMUM_JERK_COMPONENT_HALVINGS,
        PredicatePolicy::STRICT,
    )?;

    let mut phases = Vec::with_capacity(route.len());
    let mut total_path_length_mm = Real::zero();
    let mut total_traversal_time_seconds = Real::zero();
    for (element_index, element) in route.iter().enumerate() {
        let length = element_length(element)?;
        total_path_length_mm += &length;
        let element_phases = match &lookahead_plan.span_transitions[element_index] {
            Some(transition) => transition.phases.clone(),
            None => {
                symmetric_rest_to_rest_phases(&length, &limits, profile.timer_ticks_per_second())?
            }
        };
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

    Ok(CertifiedJerkSchedule2 {
        configuration_digest: profile.configuration_digest(),
        capability_digest: profile.capability_digest(),
        source: source.clone(),
        metric_path,
        travel_envelope,
        route,
        tangent_spans,
        limits,
        lookahead_plan,
        phases,
        jerk_report,
        total_path_length_mm,
        total_traversal_time_seconds,
    })
}

fn lossless_line_join_can_move(
    metric_path: &CertifiedMetricPath2,
    route: &[FeedPathElement],
    join_index: usize,
) -> bool {
    if !matches!(route.get(join_index), Some(FeedPathElement::Line(_)))
        || !matches!(route.get(join_index + 1), Some(FeedPathElement::Line(_)))
    {
        return false;
    }
    [join_index, join_index + 1]
        .into_iter()
        .all(|motion_index| {
            metric_path
                .source_element_for_motion(motion_index)
                .and_then(|source_index| {
                    metric_path
                        .spans()
                        .iter()
                        .find(|span| span.source_element() == source_index)
                })
                .is_some_and(|span| !span.is_approximated())
        })
}

/// Lower a certified schedule to V1 constant-velocity firmware segments.
///
/// A phase is divided into an exact integer number of equal time intervals.
/// The count is the smallest integer whose second-derivative chord bound
/// `A*dt²/8` is no greater than `maximum_interpolation_error_mm`, where `A`
/// is the conservative full spatial acceleration envelope. Source points are
/// evaluated exactly from the certified metric line/arc path, then coordinates
/// are rounded to the configured step lattice. Each exact ideal interval is
/// dilated by the smallest admitted factor on the caller's rational search
/// lattice and rounded upward to the output quantum. The complete candidate and
/// its immediate predecessor are replayed through the production executor. The
/// retained source-to-motion bound remains additive evidence.
pub fn lower_certified_schedule_to_v1(
    schedule: &CertifiedJerkSchedule2,
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

    let initial_position = points
        .first()
        .map(|point| [point.steps[0].get(), point.steps[1].get()])
        .ok_or(MotionScheduleError::MetricPathMismatch)?;
    let selected_timer = select_timer_lattice_schedule(
        &points,
        profile,
        initial_position,
        limits.timer_dilation_policy(),
    )?;
    for (point, tick) in points.iter_mut().zip(&selected_timer.ticks) {
        point.tick = *tick;
    }

    Ok(CanonicalScheduledProgram2 {
        configuration_digest: profile.configuration_digest(),
        capability_digest: profile.capability_digest(),
        source: schedule.source.clone(),
        metric_path: schedule.metric_path.clone(),
        timer_ticks_per_second: profile.timer_ticks_per_second(),
        output_quantum_cycles: profile.output_quantum_cycles(),
        resolution_budget: resolution_budget.clone(),
        points,
        segments: selected_timer.segments,
        executor_preflight: selected_timer.executor_preflight,
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
            timer_lattice_schedule: selected_timer.report,
        },
    })
}

struct TimerCandidate2 {
    ticks: Vec<CanonicalCycle>,
    segments: Vec<ExecutionSegment<2>>,
    executor_preflight: Result<StepperPreflightSummary<2>, MotionError>,
    ideal_total_time_seconds: Real,
    scheduled_total_time_seconds: Real,
    maximum_cumulative_delay_seconds: Real,
    maximum_segment_extension_seconds: Real,
    maximum_output_grid_padding_seconds: Real,
}

struct SelectedTimerLatticeSchedule2 {
    ticks: Vec<CanonicalCycle>,
    segments: Vec<ExecutionSegment<2>>,
    executor_preflight: StepperPreflightSummary<2>,
    report: TimerLatticeScheduleReport2,
}

struct TimerSelectionEvidence {
    selected_factor_numerator: u32,
    selected_factor_denominator: u32,
    maximum_factor_numerator: u32,
    candidate_replays: u32,
    unit_factor_rejection: Option<MotionError>,
    predecessor_rejection: Option<MotionError>,
}

fn select_timer_lattice_schedule(
    points: &[ScheduledMachinePoint2],
    profile: &MachineDynamicsProfile2,
    initial_position: [i64; 2],
    policy: TimerDilationPolicy,
) -> MotionScheduleResult<SelectedTimerLatticeSchedule2> {
    let denominator = policy.factor_denominator();
    let maximum_numerator = policy.maximum_factor_numerator();
    let mut candidate_replays = 0_u32;

    let unit_candidate =
        build_timer_candidate(points, profile, initial_position, denominator, denominator)?;
    candidate_replays =
        candidate_replays
            .checked_add(1)
            .ok_or(MotionScheduleError::IntegerOverflow {
                domain: "timer-dilation candidate replay count",
            })?;
    match unit_candidate.executor_preflight {
        Ok(executor_preflight) => Ok(finish_timer_selection(
            unit_candidate,
            executor_preflight,
            TimerSelectionEvidence {
                selected_factor_numerator: denominator,
                selected_factor_denominator: denominator,
                maximum_factor_numerator: maximum_numerator,
                candidate_replays,
                unit_factor_rejection: None,
                predecessor_rejection: None,
            },
        )),
        Err(error) if !error.is_time_dilation_candidate() => {
            Err(MotionScheduleError::ExecutorPreflight(error))
        }
        Err(unit_rejection) => {
            if maximum_numerator == denominator {
                return Err(MotionScheduleError::TimerDilationBudgetExceeded {
                    maximum_factor_numerator: maximum_numerator,
                    factor_denominator: denominator,
                    rejection: unit_rejection,
                });
            }

            let maximum_candidate = build_timer_candidate(
                points,
                profile,
                initial_position,
                maximum_numerator,
                denominator,
            )?;
            candidate_replays =
                candidate_replays
                    .checked_add(1)
                    .ok_or(MotionScheduleError::IntegerOverflow {
                        domain: "timer-dilation candidate replay count",
                    })?;
            match maximum_candidate.executor_preflight {
                Ok(_) => {}
                Err(error) if !error.is_time_dilation_candidate() => {
                    return Err(MotionScheduleError::ExecutorPreflight(error));
                }
                Err(rejection) => {
                    return Err(MotionScheduleError::TimerDilationBudgetExceeded {
                        maximum_factor_numerator: maximum_numerator,
                        factor_denominator: denominator,
                        rejection,
                    });
                }
            }

            // Each candidate duration is q*ceil(factor*ideal/q), hence is
            // monotone in the factor. Centered first-edge offsets and terminal
            // gaps are also monotone in duration, as are the production rate,
            // pulse-low, setup, and hold inequalities. A passing candidate
            // therefore makes every larger numerator pass unless a structural
            // or arithmetic failure occurs; those are never treated as timing
            // pressure above.
            let mut rejected_numerator = denominator;
            let mut admitted_numerator = maximum_numerator;
            while admitted_numerator - rejected_numerator > 1 {
                let candidate_numerator =
                    rejected_numerator + (admitted_numerator - rejected_numerator) / 2;
                let candidate = build_timer_candidate(
                    points,
                    profile,
                    initial_position,
                    candidate_numerator,
                    denominator,
                )?;
                candidate_replays = candidate_replays.checked_add(1).ok_or(
                    MotionScheduleError::IntegerOverflow {
                        domain: "timer-dilation candidate replay count",
                    },
                )?;
                match candidate.executor_preflight {
                    Ok(_) => admitted_numerator = candidate_numerator,
                    Err(error) if error.is_time_dilation_candidate() => {
                        rejected_numerator = candidate_numerator;
                    }
                    Err(error) => return Err(MotionScheduleError::ExecutorPreflight(error)),
                }
            }

            let selected = build_timer_candidate(
                points,
                profile,
                initial_position,
                admitted_numerator,
                denominator,
            )?;
            candidate_replays =
                candidate_replays
                    .checked_add(1)
                    .ok_or(MotionScheduleError::IntegerOverflow {
                        domain: "timer-dilation candidate replay count",
                    })?;
            let executor_preflight = selected
                .executor_preflight
                .map_err(MotionScheduleError::ExecutorPreflight)?;

            let predecessor = build_timer_candidate(
                points,
                profile,
                initial_position,
                admitted_numerator - 1,
                denominator,
            )?;
            candidate_replays =
                candidate_replays
                    .checked_add(1)
                    .ok_or(MotionScheduleError::IntegerOverflow {
                        domain: "timer-dilation candidate replay count",
                    })?;
            let predecessor_rejection = match predecessor.executor_preflight {
                Err(error) if error.is_time_dilation_candidate() => error,
                Err(error) => return Err(MotionScheduleError::ExecutorPreflight(error)),
                Ok(_) => {
                    return Err(MotionScheduleError::TimerDilationMinimalityUncertified {
                        selected_factor_numerator: admitted_numerator,
                        factor_denominator: denominator,
                    });
                }
            };

            Ok(finish_timer_selection(
                selected,
                executor_preflight,
                TimerSelectionEvidence {
                    selected_factor_numerator: admitted_numerator,
                    selected_factor_denominator: denominator,
                    maximum_factor_numerator: maximum_numerator,
                    candidate_replays,
                    unit_factor_rejection: Some(unit_rejection),
                    predecessor_rejection: Some(predecessor_rejection),
                },
            ))
        }
    }
}

fn finish_timer_selection(
    candidate: TimerCandidate2,
    executor_preflight: StepperPreflightSummary<2>,
    selection: TimerSelectionEvidence,
) -> SelectedTimerLatticeSchedule2 {
    SelectedTimerLatticeSchedule2 {
        ticks: candidate.ticks,
        segments: candidate.segments,
        executor_preflight,
        report: TimerLatticeScheduleReport2 {
            selected_factor_numerator: selection.selected_factor_numerator,
            selected_factor_denominator: selection.selected_factor_denominator,
            maximum_factor_numerator: selection.maximum_factor_numerator,
            candidate_replays: selection.candidate_replays,
            unit_factor_rejection: selection.unit_factor_rejection,
            predecessor_rejection: selection.predecessor_rejection,
            ideal_total_time_seconds: candidate.ideal_total_time_seconds,
            scheduled_total_time_seconds: candidate.scheduled_total_time_seconds,
            maximum_cumulative_delay_seconds: candidate.maximum_cumulative_delay_seconds,
            maximum_segment_extension_seconds: candidate.maximum_segment_extension_seconds,
            maximum_output_grid_padding_seconds: candidate.maximum_output_grid_padding_seconds,
        },
    }
}

fn build_timer_candidate(
    points: &[ScheduledMachinePoint2],
    profile: &MachineDynamicsProfile2,
    initial_position: [i64; 2],
    factor_numerator: u32,
    factor_denominator: u32,
) -> MotionScheduleResult<TimerCandidate2> {
    if points.len() < 2 || factor_denominator == 0 || factor_numerator < factor_denominator {
        return Err(MotionScheduleError::InvalidTimerDilationPolicy);
    }
    let factor = Real::from(Rational::fraction(
        i64::from(factor_numerator),
        u64::from(factor_denominator),
    )?);
    let timer_frequency = Real::from(profile.timer_ticks_per_second());
    let output_quantum = u64::from(profile.output_quantum_cycles());
    let output_quantum_real = Real::from(output_quantum);
    let one_quantum_seconds = (&output_quantum_real / &timer_frequency)?;

    let mut ticks = Vec::new();
    ticks
        .try_reserve_exact(points.len())
        .map_err(|_| MotionScheduleError::AllocationOverflow {
            domain: "timer-lattice candidate ticks",
        })?;
    ticks.push(CanonicalCycle::new(0));
    let mut segments = Vec::new();
    segments.try_reserve_exact(points.len() - 1).map_err(|_| {
        MotionScheduleError::AllocationOverflow {
            domain: "timer-lattice candidate segments",
        }
    })?;

    let mut cumulative_tick = 0_u64;
    let mut scheduled_time = Real::zero();
    let mut maximum_cumulative_delay = Real::zero();
    let mut maximum_segment_extension = Real::zero();
    let mut maximum_output_grid_padding = Real::zero();
    for (segment_index, pair) in points.windows(2).enumerate() {
        let ideal_duration = &pair[1].ideal_time_seconds - &pair[0].ideal_time_seconds;
        require_positive(&ideal_duration, "retained ideal segment duration")?;
        let scaled_ideal_duration = &ideal_duration * &factor;
        let required_output_frames = ((&scaled_ideal_duration * &timer_frequency)
            / &output_quantum_real)?
            .ceil_certified()?;
        let output_frames = u64::try_from(required_output_frames).map_err(|_| {
            MotionScheduleError::IntegerOverflow {
                domain: "timer-lattice segment frame count",
            }
        })?;
        let duration = output_frames.checked_mul(output_quantum).ok_or(
            MotionScheduleError::IntegerOverflow {
                domain: "timer-lattice segment duration",
            },
        )?;
        if duration == 0 {
            return Err(MotionScheduleError::TickBoundaryCollapsed { segment_index });
        }
        let next_tick =
            cumulative_tick
                .checked_add(duration)
                .ok_or(MotionScheduleError::IntegerOverflow {
                    domain: "timer-lattice cumulative tick",
                })?;
        let actual_duration = (Real::from(duration) / &timer_frequency)?;
        let grid_padding = &actual_duration - &scaled_ideal_duration;
        certify_nonnegative_timer_value(&grid_padding)?;
        match compare_reals(&grid_padding, &one_quantum_seconds, PredicatePolicy::STRICT).value() {
            Some(Ordering::Less) => {}
            Some(Ordering::Equal | Ordering::Greater) | None => {
                return Err(MotionScheduleError::TimerQuantizationUncertified);
            }
        }
        let segment_extension = &actual_duration - &ideal_duration;
        certify_nonnegative_timer_value(&segment_extension)?;
        scheduled_time += &actual_duration;
        let cumulative_delay = &scheduled_time - &pair[1].ideal_time_seconds;
        certify_nonnegative_timer_value(&cumulative_delay)?;
        maximum_output_grid_padding = maximum_real(maximum_output_grid_padding, grid_padding)?;
        maximum_segment_extension = maximum_real(maximum_segment_extension, segment_extension)?;
        maximum_cumulative_delay = maximum_real(maximum_cumulative_delay, cumulative_delay)?;

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
            CanonicalCycle::new(cumulative_tick),
            CanonicalCycle::new(next_tick),
            [CanonicalStep::new(delta[0]), CanonicalStep::new(delta[1])],
        )?);
        ticks.push(CanonicalCycle::new(next_tick));
        cumulative_tick = next_tick;
    }

    let ideal_total_time_seconds = points
        .last()
        .map(|point| point.ideal_time_seconds.clone())
        .ok_or(MotionScheduleError::MetricPathMismatch)?;
    let executor_preflight =
        preflight_stepper_segments(profile.stepper_timing(0), initial_position, &segments);
    Ok(TimerCandidate2 {
        ticks,
        segments,
        executor_preflight,
        ideal_total_time_seconds,
        scheduled_total_time_seconds: scheduled_time,
        maximum_cumulative_delay_seconds: maximum_cumulative_delay,
        maximum_segment_extension_seconds: maximum_segment_extension,
        maximum_output_grid_padding_seconds: maximum_output_grid_padding,
    })
}

fn certify_nonnegative_timer_value(value: &Real) -> MotionScheduleResult<()> {
    match classify_real_sign(value, PredicatePolicy::STRICT).value() {
        Some(Sign::Zero | Sign::Positive) => Ok(()),
        Some(Sign::Negative) | None => Err(MotionScheduleError::TimerQuantizationUncertified),
    }
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
    points.push(ScheduledMachinePoint2 {
        source_element: provenance.source_element,
        motion_element: provenance.motion_element,
        phase_index: provenance.phase_index,
        subdivision_index: provenance.subdivision_index,
        exact_point_mm,
        ideal_time_seconds,
        steps: [x, y],
        // Timer/output-lattice selection occurs transactionally after every
        // spatial point is retained. No partially timed program is exposed.
        tick: CanonicalCycle::new(0),
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
    /// Timer-dilation factor lattice was empty or excluded factor one.
    InvalidTimerDilationPolicy,
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
    /// One-sided output-grid construction failed an exact sign or quantum bound.
    TimerQuantizationUncertified,
    /// No candidate through the caller-owned factor ceiling passed production preflight.
    TimerDilationBudgetExceeded {
        /// Inclusive largest candidate numerator.
        maximum_factor_numerator: u32,
        /// Shared candidate denominator.
        factor_denominator: u32,
        /// Exact production rejection at the ceiling.
        rejection: MotionError,
    },
    /// The candidate immediately below the selected factor unexpectedly passed.
    TimerDilationMinimalityUncertified {
        /// Selected factor numerator.
        selected_factor_numerator: u32,
        /// Shared candidate denominator.
        factor_denominator: u32,
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
            Self::InvalidTimerDilationPolicy => formatter.write_str(
                "timer dilation requires a positive denominator and a ceiling of at least one",
            ),
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
            Self::TimerQuantizationUncertified => formatter.write_str(
                "one-sided timer/output-grid quantization did not certify its exact bounds",
            ),
            Self::TimerDilationBudgetExceeded {
                maximum_factor_numerator,
                factor_denominator,
                rejection,
            } => write!(
                formatter,
                "timer dilation through {maximum_factor_numerator}/{factor_denominator} remained infeasible: {rejection:?}"
            ),
            Self::TimerDilationMinimalityUncertified {
                selected_factor_numerator,
                factor_denominator,
            } => write!(
                formatter,
                "timer dilation {selected_factor_numerator}/{factor_denominator} passed but its immediate predecessor also passed"
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
