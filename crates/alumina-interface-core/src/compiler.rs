//! Certified source-curve approximation and deterministic machine-lattice quantization.
//!
//! This is the first window-free compiler slice. Hypercurve creates a
//! motion-specific exact chord path under a caller-owned geometric error
//! budget. The compiler then quantizes coordinates and cumulative time with
//! certified nearest-integer operations. Renderer meshes never enter this
//! module.

use std::cmp::Ordering;
use std::error::Error as StdError;
use std::fmt;

use alumina_machine_ir::ExecutionSegment;
use alumina_protocol::Digest;
use hypercurve::{
    BezierFlatteningOptions, Classification, CurveContext, CurveError, CurvePath2, ExactCurveError,
    Point2 as CurvePoint2, UncertaintyReason,
};
use hyperlimit::{PredicatePolicy, compare_reals};
use hyperreal::{Problem, Rational, Real};

use crate::boundary::{BoundaryError, CanonicalCycle, CanonicalStep, canonical_motion_segment};
use crate::machine_profile::{MachineDynamicsProfile2, MachineResolutionBudget2};
use crate::toolpath::{ToolpathError, representative_curve_path};

/// Result type for exact-to-canonical path compilation.
pub type MachineCompileResult<T> = Result<T, MachineCompileError>;

/// Exact machine and compiler facts selected for a two-axis path compilation.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionCompilePolicy2 {
    machine_configuration_digest: Digest,
    capability_digest: Digest,
    resolution_budget: Option<MachineResolutionBudget2>,
    steps_per_millimetre: [Rational; 2],
    timer_ticks_per_second: u64,
    feed_millimetres_per_second: Rational,
    maximum_source_chord_error_mm: Rational,
    maximum_subdivision_depth: usize,
}

impl MotionCompilePolicy2 {
    /// Validate and retain exact machine-lattice and approximation policy facts.
    pub(crate) fn try_new(
        steps_per_millimetre: [Rational; 2],
        timer_ticks_per_second: u64,
        feed_millimetres_per_second: Rational,
        maximum_source_chord_error_mm: Rational,
        maximum_subdivision_depth: usize,
    ) -> MachineCompileResult<Self> {
        if steps_per_millimetre
            .iter()
            .any(|value| value <= &Rational::zero())
        {
            return Err(MachineCompileError::InvalidPolicy(
                "steps per millimetre must be positive",
            ));
        }
        if timer_ticks_per_second == 0 {
            return Err(MachineCompileError::InvalidPolicy(
                "timer ticks per second must be nonzero",
            ));
        }
        if feed_millimetres_per_second <= Rational::zero() {
            return Err(MachineCompileError::InvalidPolicy(
                "feed rate must be positive",
            ));
        }
        if maximum_source_chord_error_mm <= Rational::zero() {
            return Err(MachineCompileError::InvalidPolicy(
                "source chord error must be positive",
            ));
        }
        if maximum_subdivision_depth == 0 {
            return Err(MachineCompileError::InvalidPolicy(
                "subdivision depth must be nonzero",
            ));
        }
        Ok(Self {
            machine_configuration_digest: Digest::ZERO,
            capability_digest: Digest::ZERO,
            resolution_budget: None,
            steps_per_millimetre,
            timer_ticks_per_second,
            feed_millimetres_per_second,
            maximum_source_chord_error_mm,
            maximum_subdivision_depth,
        })
    }

    /// Constructs a production policy only from one validated machine profile
    /// and an error budget certified for the same configuration/capability.
    pub fn from_machine_profile(
        profile: &MachineDynamicsProfile2,
        resolution_budget: &MachineResolutionBudget2,
        feed_millimetres_per_second: Rational,
        maximum_source_chord_error_mm: Rational,
        maximum_subdivision_depth: usize,
    ) -> MachineCompileResult<Self> {
        if resolution_budget.configuration_digest() != profile.configuration_digest()
            || resolution_budget.capability_digest() != profile.capability_digest()
        {
            return Err(MachineCompileError::MachineIdentityMismatch);
        }
        if maximum_source_chord_error_mm
            > resolution_budget.source_curve_allocation_mm_exact().clone()
        {
            return Err(MachineCompileError::SourceErrorBudgetExceeded);
        }
        for (axis, machine_axis) in profile.axes().iter().enumerate() {
            let limit_mm_per_second =
                machine_axis.effective_velocity_limit_metres_per_second() * Rational::from(1_000);
            if feed_millimetres_per_second > limit_mm_per_second {
                return Err(MachineCompileError::FeedLimitExceeded { axis });
            }
        }

        let mut policy = Self::try_new(
            [
                profile.axes()[0]
                    .command_density_steps_per_millimetre()
                    .nominal()
                    .clone(),
                profile.axes()[1]
                    .command_density_steps_per_millimetre()
                    .nominal()
                    .clone(),
            ],
            profile.timer_ticks_per_second(),
            feed_millimetres_per_second,
            maximum_source_chord_error_mm,
            maximum_subdivision_depth,
        )?;
        policy.machine_configuration_digest = profile.configuration_digest();
        policy.capability_digest = profile.capability_digest();
        policy.resolution_budget = Some(resolution_budget.clone());
        Ok(policy)
    }

    /// Exact active machine-configuration identity, absent only for the fixed
    /// non-production representative fixture.
    pub fn machine_configuration_digest(&self) -> Option<Digest> {
        (!self.machine_configuration_digest.is_zero()).then_some(self.machine_configuration_digest)
    }

    /// Exact immutable board-capability identity, absent only for the fixed
    /// non-production representative fixture.
    pub fn capability_digest(&self) -> Option<Digest> {
        (!self.capability_digest.is_zero()).then_some(self.capability_digest)
    }

    /// Full physical and numerical error decomposition retained by a
    /// machine-bound policy.
    pub const fn resolution_budget(&self) -> Option<&MachineResolutionBudget2> {
        self.resolution_budget.as_ref()
    }

    /// Borrow exact axis command densities in steps per millimetre.
    pub const fn steps_per_millimetre(&self) -> &[Rational; 2] {
        &self.steps_per_millimetre
    }

    /// Return the exact integer timer frequency.
    pub const fn timer_ticks_per_second(&self) -> u64 {
        self.timer_ticks_per_second
    }

    /// Borrow the exact requested feed rate.
    pub const fn feed_millimetres_per_second(&self) -> &Rational {
        &self.feed_millimetres_per_second
    }

    /// Borrow the motion-specific source-to-chord error budget.
    pub const fn maximum_source_chord_error_mm(&self) -> &Rational {
        &self.maximum_source_chord_error_mm
    }

    /// Return the maximum certified recursive subdivision depth.
    pub const fn maximum_subdivision_depth(&self) -> usize {
        self.maximum_subdivision_depth
    }
}

/// One exact motion-chord endpoint and its canonical machine coordinate.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalPathPoint2 {
    chord_point_mm: CurvePoint2,
    steps: [CanonicalStep; 2],
    signed_quantization_error_mm: [Real; 2],
}

impl CanonicalPathPoint2 {
    /// Borrow the exact Hypercurve endpoint before machine quantization.
    pub const fn chord_point_mm(&self) -> &CurvePoint2 {
        &self.chord_point_mm
    }

    /// Return the canonical absolute machine-lattice coordinate.
    pub const fn steps(&self) -> [CanonicalStep; 2] {
        self.steps
    }

    /// Borrow signed `canonical - exact chord` errors for X and Y.
    pub const fn signed_quantization_error_mm(&self) -> &[Real; 2] {
        &self.signed_quantization_error_mm
    }
}

/// One ideal cumulative path time and its canonical timer boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalTimeBoundary {
    ideal_seconds: Real,
    tick: CanonicalCycle,
    signed_quantization_error_seconds: Real,
}

impl CanonicalTimeBoundary {
    /// Borrow the exact ideal cumulative time along the retained chord path.
    pub const fn ideal_seconds(&self) -> &Real {
        &self.ideal_seconds
    }

    /// Return the canonical cumulative timer tick.
    pub const fn tick(&self) -> CanonicalCycle {
        self.tick
    }

    /// Borrow signed `canonical - ideal` timer-boundary error.
    pub const fn signed_quantization_error_seconds(&self) -> &Real {
        &self.signed_quantization_error_seconds
    }
}

/// Explicit approximation and lattice error budget retained beside machine IR.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionApproximationEvidence2 {
    maximum_source_chord_error_mm: Real,
    source_fragment_count: usize,
    chord_segment_count: usize,
    maximum_subdivision_depth: usize,
    maximum_axis_quantization_error_mm: [Real; 2],
    maximum_position_quantization_error_mm: Real,
    maximum_curve_to_canonical_chord_error_mm: Real,
    maximum_timer_boundary_error_seconds: Real,
    maximum_segment_duration_error_seconds: Real,
}

impl MotionApproximationEvidence2 {
    /// Borrow Hypercurve's certified source-to-motion-chord error bound.
    pub const fn maximum_source_chord_error_mm(&self) -> &Real {
        &self.maximum_source_chord_error_mm
    }

    /// Return the number of native source fragments covered by subdivision.
    pub const fn source_fragment_count(&self) -> usize {
        self.source_fragment_count
    }

    /// Return the number of canonical motion chords.
    pub const fn chord_segment_count(&self) -> usize {
        self.chord_segment_count
    }

    /// Return the deepest certified source subdivision used.
    pub const fn maximum_subdivision_depth(&self) -> usize {
        self.maximum_subdivision_depth
    }

    /// Borrow the half-command-lattice bound for each axis.
    pub const fn maximum_axis_quantization_error_mm(&self) -> &[Real; 2] {
        &self.maximum_axis_quantization_error_mm
    }

    /// Borrow the Euclidean endpoint/interpolated-chord quantization bound.
    pub const fn maximum_position_quantization_error_mm(&self) -> &Real {
        &self.maximum_position_quantization_error_mm
    }

    /// Borrow the conservative source-curve-to-canonical-command-chord bound.
    ///
    /// This is not a physical following-error claim. Discrete step-event,
    /// calibration, mechanics, and control errors remain separate evidence.
    pub const fn maximum_curve_to_canonical_chord_error_mm(&self) -> &Real {
        &self.maximum_curve_to_canonical_chord_error_mm
    }

    /// Borrow the half-timer-tick cumulative boundary error bound.
    pub const fn maximum_timer_boundary_error_seconds(&self) -> &Real {
        &self.maximum_timer_boundary_error_seconds
    }

    /// Borrow the one-timer-tick segment-duration error bound.
    ///
    /// A segment duration is the difference of two independently rounded
    /// cumulative boundaries, so its conservative bound is twice the
    /// half-tick boundary bound.
    pub const fn maximum_segment_duration_error_seconds(&self) -> &Real {
        &self.maximum_segment_duration_error_seconds
    }
}

/// Canonical two-axis program plus every retained exact boundary witness.
#[derive(Clone, Debug)]
pub struct CanonicalPathProgram2 {
    source: CurvePath2,
    policy: MotionCompilePolicy2,
    points: Vec<CanonicalPathPoint2>,
    time_boundaries: Vec<CanonicalTimeBoundary>,
    segments: Vec<ExecutionSegment<2>>,
    ideal_chord_path_length_mm: Real,
    evidence: MotionApproximationEvidence2,
}

impl CanonicalPathProgram2 {
    /// Borrow the authoritative exact source path.
    pub const fn source(&self) -> &CurvePath2 {
        &self.source
    }

    /// Borrow exact machine and approximation policy inputs.
    pub const fn policy(&self) -> &MotionCompilePolicy2 {
        &self.policy
    }

    /// Borrow every certified chord endpoint and canonical coordinate.
    pub fn points(&self) -> &[CanonicalPathPoint2] {
        &self.points
    }

    /// Borrow cumulative ideal/canonical timer boundaries.
    pub fn time_boundaries(&self) -> &[CanonicalTimeBoundary] {
        &self.time_boundaries
    }

    /// Borrow canonical firmware execution segments.
    pub fn segments(&self) -> &[ExecutionSegment<2>] {
        &self.segments
    }

    /// Borrow the exact total length of the compiled chord path.
    pub const fn ideal_chord_path_length_mm(&self) -> &Real {
        &self.ideal_chord_path_length_mm
    }

    /// Borrow retained approximation and quantization evidence.
    pub const fn evidence(&self) -> &MotionApproximationEvidence2 {
        &self.evidence
    }
}

/// Compile one exact source path into deterministic canonical linear segments.
///
/// Hypercurve subdivision is invoked independently from any display adapter.
/// Cumulative chord length determines ideal constant-feed time, which avoids
/// per-segment rounding drift before certified nearest-tick quantization.
pub fn compile_certified_chord_program(
    source: &CurvePath2,
    policy: &MotionCompilePolicy2,
) -> MachineCompileResult<CanonicalPathProgram2> {
    let maximum_source_chord_error_mm = Real::from(policy.maximum_source_chord_error_mm.clone());
    let options = BezierFlatteningOptions::try_new(
        maximum_source_chord_error_mm.clone(),
        policy.maximum_subdivision_depth,
        &CurveContext::STRICT,
    )?;
    let polyline = match source.segment_certified(&options, &CurveContext::STRICT)? {
        Classification::Decided(polyline) => polyline,
        Classification::Uncertain(reason) => {
            return Err(MachineCompileError::SegmentationUncertain(reason));
        }
    };

    let maximum_axis_quantization_error_mm = [
        half_lattice_unit(&policy.steps_per_millimetre[0])?,
        half_lattice_unit(&policy.steps_per_millimetre[1])?,
    ];
    let maximum_position_quantization_error_mm = (maximum_axis_quantization_error_mm[0].clone()
        * maximum_axis_quantization_error_mm[0].clone()
        + maximum_axis_quantization_error_mm[1].clone()
            * maximum_axis_quantization_error_mm[1].clone())
    .sqrt()?;
    let maximum_curve_to_canonical_chord_error_mm =
        maximum_source_chord_error_mm.clone() + maximum_position_quantization_error_mm.clone();
    let maximum_timer_boundary_error_seconds =
        (Real::one() / (Real::from(2) * Real::from(policy.timer_ticks_per_second)))?;
    let maximum_segment_duration_error_seconds =
        (Real::one() / Real::from(policy.timer_ticks_per_second))?;

    let mut points = Vec::new();
    points
        .try_reserve_exact(polyline.points().len())
        .map_err(|_| MachineCompileError::AllocationOverflow)?;
    for (point_index, point) in polyline.points().iter().enumerate() {
        let (x_steps, x_error) = quantize_axis(
            point.x(),
            &policy.steps_per_millimetre[0],
            &maximum_axis_quantization_error_mm[0],
            point_index,
            0,
        )?;
        let (y_steps, y_error) = quantize_axis(
            point.y(),
            &policy.steps_per_millimetre[1],
            &maximum_axis_quantization_error_mm[1],
            point_index,
            1,
        )?;
        points.push(CanonicalPathPoint2 {
            chord_point_mm: point.clone(),
            steps: [x_steps, y_steps],
            signed_quantization_error_mm: [x_error, y_error],
        });
    }

    let feed = Real::from(policy.feed_millimetres_per_second.clone());
    let timer_frequency = Real::from(policy.timer_ticks_per_second);
    let mut ideal_chord_path_length_mm = Real::zero();
    let mut time_boundaries = Vec::new();
    time_boundaries
        .try_reserve_exact(points.len())
        .map_err(|_| MachineCompileError::AllocationOverflow)?;
    time_boundaries.push(CanonicalTimeBoundary {
        ideal_seconds: Real::zero(),
        tick: CanonicalCycle::new(0),
        signed_quantization_error_seconds: Real::zero(),
    });

    let mut segments = Vec::new();
    segments
        .try_reserve_exact(points.len().saturating_sub(1))
        .map_err(|_| MachineCompileError::AllocationOverflow)?;
    for (segment_index, pair) in points.windows(2).enumerate() {
        let source_pair = &polyline.points()[segment_index..=segment_index + 1];
        let dx = source_pair[1].x() - source_pair[0].x();
        let dy = source_pair[1].y() - source_pair[0].y();
        let chord_length = (dx.clone() * dx + dy.clone() * dy).sqrt()?;
        ideal_chord_path_length_mm += chord_length;
        let ideal_seconds = (ideal_chord_path_length_mm.clone() / feed.clone())?;
        let ideal_ticks = ideal_seconds.clone() * timer_frequency.clone();
        let tick = certified_u64_round(&ideal_ticks, "timer boundary", segment_index)?;
        let start = time_boundaries
            .last()
            .expect("initial time boundary is retained")
            .tick;
        let end = CanonicalCycle::new(tick);
        if end <= start {
            return Err(MachineCompileError::TickBoundaryCollapsed { segment_index });
        }

        let delta = [
            pair[1].steps[0]
                .get()
                .checked_sub(pair[0].steps[0].get())
                .ok_or(MachineCompileError::StepDeltaOverflow {
                    segment_index,
                    axis: 0,
                })?,
            pair[1].steps[1]
                .get()
                .checked_sub(pair[0].steps[1].get())
                .ok_or(MachineCompileError::StepDeltaOverflow {
                    segment_index,
                    axis: 1,
                })?,
        ];
        if delta == [0, 0] {
            return Err(MachineCompileError::SpatialChordCollapsed { segment_index });
        }
        segments.push(canonical_motion_segment(
            start,
            end,
            [CanonicalStep::new(delta[0]), CanonicalStep::new(delta[1])],
        )?);

        let canonical_seconds = (Real::from(tick) / timer_frequency.clone())?;
        let signed_quantization_error_seconds = canonical_seconds - ideal_seconds.clone();
        certify_bound(
            &signed_quantization_error_seconds.abs(),
            &maximum_timer_boundary_error_seconds,
            MachineCompileError::TimerBoundViolated { segment_index },
        )?;
        time_boundaries.push(CanonicalTimeBoundary {
            ideal_seconds,
            tick: end,
            signed_quantization_error_seconds,
        });
    }

    let evidence = MotionApproximationEvidence2 {
        maximum_source_chord_error_mm,
        source_fragment_count: polyline.source_fragment_count(),
        chord_segment_count: polyline.certificate().segment_count(),
        maximum_subdivision_depth: polyline.certificate().max_depth(),
        maximum_axis_quantization_error_mm,
        maximum_position_quantization_error_mm,
        maximum_curve_to_canonical_chord_error_mm,
        maximum_timer_boundary_error_seconds,
        maximum_segment_duration_error_seconds,
    };

    Ok(CanonicalPathProgram2 {
        source: source.clone(),
        policy: policy.clone(),
        points,
        time_boundaries,
        segments,
        ideal_chord_path_length_mm,
        evidence,
    })
}

/// Compile the representative line/arc/cubic source with a deterministic fixture policy.
pub fn compile_representative_program() -> MachineCompileResult<CanonicalPathProgram2> {
    let policy = MotionCompilePolicy2::try_new(
        [Rational::from(80), Rational::from(80)],
        1_000_000,
        Rational::from(10),
        Rational::fraction(1, 1_024)?,
        24,
    )?;
    let source = representative_curve_path()?;
    compile_certified_chord_program(&source, &policy)
}

pub(crate) fn half_lattice_unit(steps_per_millimetre: &Rational) -> Result<Real, Problem> {
    Real::one() / (Real::from(2) * Real::from(steps_per_millimetre.clone()))
}

pub(crate) fn quantize_axis(
    coordinate_mm: &Real,
    steps_per_millimetre: &Rational,
    maximum_error_mm: &Real,
    point_index: usize,
    axis: usize,
) -> MachineCompileResult<(CanonicalStep, Real)> {
    let density = Real::from(steps_per_millimetre.clone());
    let scaled = coordinate_mm * &density;
    let steps = scaled.round_certified()?;
    let steps = i64::try_from(steps).map_err(|_| MachineCompileError::IntegerOverflow {
        domain: "axis step",
        index: point_index,
        axis: Some(axis),
    })?;
    let canonical_coordinate_mm = (Real::from(steps) / density)?;
    let signed_error_mm = canonical_coordinate_mm - coordinate_mm;
    certify_bound(
        &signed_error_mm.abs(),
        maximum_error_mm,
        MachineCompileError::SpatialBoundViolated { point_index, axis },
    )?;
    Ok((CanonicalStep::new(steps), signed_error_mm))
}

pub(crate) fn certified_u64_round(
    value: &Real,
    domain: &'static str,
    index: usize,
) -> MachineCompileResult<u64> {
    let integer = value.round_certified()?;
    u64::try_from(integer).map_err(|_| MachineCompileError::IntegerOverflow {
        domain,
        index,
        axis: None,
    })
}

fn certify_bound(
    magnitude: &Real,
    bound: &Real,
    violated: MachineCompileError,
) -> MachineCompileResult<()> {
    match compare_reals(magnitude, bound, PredicatePolicy::STRICT).value() {
        Some(Ordering::Less | Ordering::Equal) => Ok(()),
        Some(Ordering::Greater) => Err(violated),
        None => Err(MachineCompileError::QuantizationPredicateUnresolved),
    }
}

/// Failure at an exact approximation, quantization, or canonical-IR boundary.
#[derive(Debug)]
pub enum MachineCompileError {
    /// A static machine/compiler policy fact was invalid.
    InvalidPolicy(&'static str),
    /// A resolution certificate belonged to a different machine identity.
    MachineIdentityMismatch,
    /// Requested scalar path feed exceeded a conservative per-axis ceiling.
    FeedLimitExceeded {
        /// Axis imposing the limit.
        axis: usize,
    },
    /// Requested curve approximation exceeded its certified allocation.
    SourceErrorBudgetExceeded,
    /// The representative exact source fixture failed to construct.
    SourceFixture(ToolpathError),
    /// Hypercurve rejected motion-specific subdivision options.
    CurveConstruction(CurveError),
    /// Hypercurve rejected source-path subdivision.
    ExactCurve(ExactCurveError),
    /// Hypercurve could not certify subdivision at the selected depth.
    SegmentationUncertain(UncertaintyReason),
    /// Hyperreal rejected an exact arithmetic or integer-rounding operation.
    Arithmetic(Problem),
    /// A certified integer did not fit the canonical firmware representation.
    IntegerOverflow {
        /// Quantized value domain.
        domain: &'static str,
        /// Point or segment index.
        index: usize,
        /// Axis index when this was a spatial value.
        axis: Option<usize>,
    },
    /// A certified motion chord became a zero-displacement canonical segment.
    SpatialChordCollapsed {
        /// Zero-based chord index.
        segment_index: usize,
    },
    /// A positive ideal chord duration became an empty canonical tick range.
    TickBoundaryCollapsed {
        /// Zero-based chord index.
        segment_index: usize,
    },
    /// Canonical endpoint subtraction overflowed.
    StepDeltaOverflow {
        /// Zero-based chord index.
        segment_index: usize,
        /// Axis whose delta overflowed.
        axis: usize,
    },
    /// A certified nearest-step result violated its half-lattice bound.
    SpatialBoundViolated {
        /// Quantized point index.
        point_index: usize,
        /// Quantized axis index.
        axis: usize,
    },
    /// A certified nearest-tick result violated its half-tick boundary bound.
    TimerBoundViolated {
        /// Quantized chord-end index.
        segment_index: usize,
    },
    /// Hyperlimit could not replay a quantization-bound comparison.
    QuantizationPredicateUnresolved,
    /// Canonical vector allocation could not be represented or reserved.
    AllocationOverflow,
    /// Canonical machine-IR construction rejected a segment.
    CanonicalBoundary(BoundaryError),
}

impl fmt::Display for MachineCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy(reason) => write!(formatter, "invalid motion policy: {reason}"),
            Self::MachineIdentityMismatch => {
                formatter.write_str("machine profile and resolution budget identities do not match")
            }
            Self::FeedLimitExceeded { axis } => write!(
                formatter,
                "requested path feed exceeds conservative axis {axis} velocity"
            ),
            Self::SourceErrorBudgetExceeded => formatter
                .write_str("source-curve approximation exceeds its certified error allocation"),
            Self::SourceFixture(source) => {
                write!(formatter, "exact source fixture failed: {source}")
            }
            Self::CurveConstruction(source) => {
                write!(formatter, "motion subdivision options failed: {source}")
            }
            Self::ExactCurve(source) => write!(formatter, "motion subdivision failed: {source}"),
            Self::SegmentationUncertain(reason) => {
                write!(
                    formatter,
                    "motion subdivision remained uncertain: {reason:?}"
                )
            }
            Self::Arithmetic(source) => {
                write!(formatter, "exact machine compilation failed: {source}")
            }
            Self::IntegerOverflow {
                domain,
                index,
                axis,
            } => write!(
                formatter,
                "{domain} at index {index}{} does not fit canonical integer storage",
                axis.map_or_else(String::new, |axis| format!(" axis {axis}"))
            ),
            Self::SpatialChordCollapsed { segment_index } => write!(
                formatter,
                "motion chord {segment_index} collapsed on the configured command lattice"
            ),
            Self::TickBoundaryCollapsed { segment_index } => write!(
                formatter,
                "motion chord {segment_index} collapsed on the configured timer lattice"
            ),
            Self::StepDeltaOverflow {
                segment_index,
                axis,
            } => write!(
                formatter,
                "motion chord {segment_index} axis {axis} delta overflowed"
            ),
            Self::SpatialBoundViolated { point_index, axis } => write!(
                formatter,
                "point {point_index} axis {axis} exceeded its half-command-lattice bound"
            ),
            Self::TimerBoundViolated { segment_index } => write!(
                formatter,
                "motion chord {segment_index} exceeded its half-timer-tick boundary bound"
            ),
            Self::QuantizationPredicateUnresolved => {
                formatter.write_str("an exact quantization-bound predicate remained unresolved")
            }
            Self::AllocationOverflow => {
                formatter.write_str("canonical program storage could not be reserved")
            }
            Self::CanonicalBoundary(source) => {
                write!(formatter, "canonical machine-IR boundary failed: {source}")
            }
        }
    }
}

impl StdError for MachineCompileError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::SourceFixture(source) => Some(source),
            Self::CurveConstruction(source) => Some(source),
            Self::ExactCurve(source) => Some(source),
            Self::Arithmetic(source) => Some(source),
            Self::CanonicalBoundary(source) => Some(source),
            _ => None,
        }
    }
}

impl From<ToolpathError> for MachineCompileError {
    fn from(value: ToolpathError) -> Self {
        Self::SourceFixture(value)
    }
}

impl From<CurveError> for MachineCompileError {
    fn from(value: CurveError) -> Self {
        Self::CurveConstruction(value)
    }
}

impl From<ExactCurveError> for MachineCompileError {
    fn from(value: ExactCurveError) -> Self {
        Self::ExactCurve(value)
    }
}

impl From<Problem> for MachineCompileError {
    fn from(value: Problem) -> Self {
        Self::Arithmetic(value)
    }
}

impl From<BoundaryError> for MachineCompileError {
    fn from(value: BoundaryError) -> Self {
        Self::CanonicalBoundary(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_program_is_deterministic_and_canonical() {
        let first = compile_representative_program().unwrap();
        let second = compile_representative_program().unwrap();

        assert_eq!(first.segments(), second.segments());
        assert_eq!(first.points(), second.points());
        assert_eq!(first.time_boundaries(), second.time_boundaries());
        assert_eq!(first.source().curves().len(), 3);
        assert!(first.segments().len() > 3);
        assert_eq!(first.points().len(), first.segments().len() + 1);
        assert_eq!(first.time_boundaries().len(), first.points().len());

        for pair in first.segments().windows(2) {
            assert_eq!(pair[0].end_tick, pair[1].start_tick);
        }
        let final_steps = first
            .segments()
            .iter()
            .fold([0_i64; 2], |mut total, segment| {
                total[0] += segment.delta_steps[0];
                total[1] += segment.delta_steps[1];
                total
            });
        assert_eq!(final_steps, [960, 0]);
    }

    #[test]
    fn every_spatial_and_temporal_quantization_stays_inside_its_bound() {
        let program = compile_representative_program().unwrap();
        let evidence = program.evidence();

        for point in program.points() {
            for axis in 0..2 {
                assert!(matches!(
                    compare_reals(
                        &point.signed_quantization_error_mm()[axis].abs(),
                        &evidence.maximum_axis_quantization_error_mm()[axis],
                        PredicatePolicy::STRICT,
                    )
                    .value(),
                    Some(Ordering::Less | Ordering::Equal)
                ));
            }
        }
        for boundary in program.time_boundaries() {
            assert!(matches!(
                compare_reals(
                    &boundary.signed_quantization_error_seconds().abs(),
                    evidence.maximum_timer_boundary_error_seconds(),
                    PredicatePolicy::STRICT,
                )
                .value(),
                Some(Ordering::Less | Ordering::Equal)
            ));
        }
        assert_eq!(evidence.chord_segment_count(), program.segments().len());
    }

    #[test]
    fn invalid_machine_lattices_fail_before_source_compilation() {
        assert!(matches!(
            MotionCompilePolicy2::try_new(
                [Rational::zero(), Rational::from(80)],
                1_000_000,
                Rational::from(10),
                Rational::fraction(1, 1_024).unwrap(),
                24,
            ),
            Err(MachineCompileError::InvalidPolicy(_))
        ));
        assert!(matches!(
            MotionCompilePolicy2::try_new(
                [Rational::from(80), Rational::from(80)],
                0,
                Rational::from(10),
                Rational::fraction(1, 1_024).unwrap(),
                24,
            ),
            Err(MachineCompileError::InvalidPolicy(_))
        ));
    }
}
