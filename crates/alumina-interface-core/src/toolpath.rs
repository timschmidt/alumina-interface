//! Exact source-curve fixtures and the first checked Hypercurve-to-Hyperpath boundary.
//!
//! Hypercurve remains the source-geometry authority. Hyperpath receives a new
//! exact metric carrier either through lossless promotion or through an
//! explicitly budgeted, pointwise-certified source-to-motion reduction over
//! Hypercurve's exact Bezier and de Casteljau objects.
//! Renderer chords never participate in either boundary.

use std::cmp::Ordering;
use std::error::Error as StdError;
use std::fmt;

use hypercurve::{
    CircularArc2, CubicBezier2, Curve2, CurveError, CurveFamily2, CurveGeometry2, CurvePath2,
    CurveRegion2, CurveRegionLoopRole, ExactCurveError, FillRule, LineSeg2, Point2 as CurvePoint2,
    UncertaintyReason,
};
use hyperlimit::{Point2 as PredicatePoint2, PredicatePolicy, compare_reals};
use hyperpath::{
    ArcDirection, CircularArcError, ConstantFeedTimeReport, ExplicitCircularArc, FeedPathElement,
    LinePathSegment, LinePathSegmentError, RouteCertificationError,
    certify_constant_feed_time_for_path,
};
use hyperreal::{Problem, Rational, Real};

/// Result type for window-free exact toolpath construction and promotion.
pub type ToolpathResult<T> = Result<T, ToolpathError>;

/// Caller-owned bounds for converting supported exact source curves into a
/// metric line/arc path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricPathApproximationLimits2 {
    maximum_motion_elements: usize,
    maximum_subdivision_depth: usize,
}

impl MetricPathApproximationLimits2 {
    /// Interactive browser policy for one machine path.
    pub const INTERACTIVE: Self = Self {
        maximum_motion_elements: 16_384,
        maximum_subdivision_depth: 20,
    };

    /// Construct explicit motion-element and recursive-depth bounds.
    pub const fn try_new(
        maximum_motion_elements: usize,
        maximum_subdivision_depth: usize,
    ) -> ToolpathResult<Self> {
        if maximum_motion_elements == 0 || maximum_subdivision_depth == 0 {
            return Err(ToolpathError::InvalidMetricApproximationLimits);
        }
        Ok(Self {
            maximum_motion_elements,
            maximum_subdivision_depth,
        })
    }

    /// Maximum retained line/arc elements after certified source reduction.
    pub const fn maximum_motion_elements(self) -> usize {
        self.maximum_motion_elements
    }

    /// Maximum binary subdivision depth requested from Hypercurve.
    pub const fn maximum_subdivision_depth(self) -> usize {
        self.maximum_subdivision_depth
    }
}

/// Exact source-to-motion provenance for one retained source curve.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertifiedMetricSourceSpan2 {
    source_element: usize,
    source_family: CurveFamily2,
    motion_element_start: usize,
    motion_element_count: usize,
    maximum_error_mm_exact: Rational,
    maximum_subdivision_depth: usize,
}

impl CertifiedMetricSourceSpan2 {
    /// Zero-based exact source-curve index.
    pub const fn source_element(&self) -> usize {
        self.source_element
    }

    /// Retained exact source family.
    pub const fn source_family(&self) -> CurveFamily2 {
        self.source_family
    }

    /// First generated metric-path element.
    pub const fn motion_element_start(&self) -> usize {
        self.motion_element_start
    }

    /// Number of generated metric-path elements.
    pub const fn motion_element_count(&self) -> usize {
        self.motion_element_count
    }

    /// Certified source-to-motion positional error in millimetres.
    pub const fn maximum_error_mm_exact(&self) -> &Rational {
        &self.maximum_error_mm_exact
    }

    /// Deepest recursive subdivision used for this source curve.
    pub const fn maximum_subdivision_depth(&self) -> usize {
        self.maximum_subdivision_depth
    }

    /// Whether this span required a nonzero certified source reduction.
    pub fn is_approximated(&self) -> bool {
        !self.maximum_error_mm_exact.is_zero()
    }
}

/// Exact line/arc motion path plus a certified mapping back to the retained
/// source path.
#[derive(Clone, Debug, PartialEq)]
pub struct CertifiedMetricPath2 {
    path: CurvePath2,
    spans: Vec<CertifiedMetricSourceSpan2>,
    source_element_by_motion: Vec<usize>,
    maximum_source_error_mm_exact: Rational,
}

impl CertifiedMetricPath2 {
    /// Borrow the exact reduced line/arc path consumed by Hyperpath.
    pub const fn path(&self) -> &CurvePath2 {
        &self.path
    }

    /// Source-to-motion spans in exact source order.
    pub fn spans(&self) -> &[CertifiedMetricSourceSpan2] {
        &self.spans
    }

    /// Maximum certified source-to-motion positional error.
    pub const fn maximum_source_error_mm_exact(&self) -> &Rational {
        &self.maximum_source_error_mm_exact
    }

    /// Map one reduced motion element back to its retained source curve.
    pub fn source_element_for_motion(&self, motion_element: usize) -> Option<usize> {
        self.source_element_by_motion.get(motion_element).copied()
    }
}

/// A failure at an explicit source-geometry or metric-promotion boundary.
#[derive(Debug)]
pub enum ToolpathError {
    /// Hypercurve rejected source geometry before a path existed.
    CurveConstruction(CurveError),
    /// Hypercurve rejected exact path topology or connectivity.
    ExactCurve(ExactCurveError),
    /// Hyperreal rejected an exact derived quantity such as an arc radius.
    Arithmetic(Problem),
    /// Source approximation policy was empty or inconsistent.
    InvalidMetricApproximationLimits,
    /// The selected source-curve error allocation was negative.
    InvalidMetricApproximationError,
    /// A supported curved source required a positive approximation allocation.
    MetricApproximationRequired {
        /// Zero-based source curve index.
        curve_index: usize,
    },
    /// Exact pointwise predicates could not certify the requested reduction.
    MetricApproximationUncertain {
        /// Zero-based source curve index.
        curve_index: usize,
        /// Exact-ordering blocker.
        reason: UncertaintyReason,
    },
    /// The selected recursive-depth ceiling was reached before certification.
    MetricApproximationDepthExceeded {
        /// Zero-based source curve index.
        curve_index: usize,
        /// Caller-owned or resource-capped depth ceiling.
        maximum_depth: usize,
    },
    /// Source reduction exceeded the caller-owned motion-element budget.
    MetricApproximationBudgetExceeded {
        /// Number of elements required by the candidate prefix.
        required: usize,
        /// Caller-owned maximum.
        maximum: usize,
    },
    /// A bounded source-reduction allocation could not be reserved.
    AllocationOverflow {
        /// Allocation domain.
        domain: &'static str,
    },
    /// A checked element-count calculation overflowed before allocation.
    IntegerOverflow {
        /// Count domain.
        domain: &'static str,
    },
    /// Hyperpath could not certify a source line's retained bounds.
    LinePromotion {
        /// Zero-based source curve index.
        curve_index: usize,
        /// Exact-predicate construction failure.
        source: LinePathSegmentError,
    },
    /// Hyperpath could not certify an explicit source arc.
    ArcPromotion {
        /// Zero-based source curve index.
        curve_index: usize,
        /// Exact-predicate construction failure.
        source: CircularArcError,
    },
    /// The exact source family has no lossless Hyperpath metric carrier yet.
    UnsupportedMetricCurve {
        /// Zero-based source curve index.
        curve_index: usize,
        /// Retained Hypercurve family that was not demoted to chords.
        family: CurveFamily2,
    },
    /// Hyperpath rejected a proposed feed replay.
    FeedCertification(RouteCertificationError),
}

impl fmt::Display for ToolpathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurveConstruction(source) => {
                write!(
                    formatter,
                    "exact source-curve construction failed: {source}"
                )
            }
            Self::ExactCurve(source) => {
                write!(formatter, "exact source path failed: {source}")
            }
            Self::Arithmetic(source) => {
                write!(formatter, "exact derived path quantity failed: {source}")
            }
            Self::InvalidMetricApproximationLimits => {
                formatter.write_str("metric source-approximation limits are invalid")
            }
            Self::InvalidMetricApproximationError => {
                formatter.write_str("metric source-approximation error must be nonnegative")
            }
            Self::MetricApproximationRequired { curve_index } => write!(
                formatter,
                "source curve {curve_index} requires a positive certified metric-approximation allocation"
            ),
            Self::MetricApproximationUncertain {
                curve_index,
                reason,
            } => write!(
                formatter,
                "source curve {curve_index} could not satisfy the pointwise metric-path certificate: {reason:?}"
            ),
            Self::MetricApproximationDepthExceeded {
                curve_index,
                maximum_depth,
            } => write!(
                formatter,
                "source curve {curve_index} requires subdivision beyond the selected depth {maximum_depth}"
            ),
            Self::MetricApproximationBudgetExceeded { required, maximum } => write!(
                formatter,
                "certified metric path requires {required} elements; policy permits {maximum}"
            ),
            Self::AllocationOverflow { domain } => {
                write!(
                    formatter,
                    "bounded metric-path allocation failed for {domain}"
                )
            }
            Self::IntegerOverflow { domain } => {
                write!(
                    formatter,
                    "metric-path element count overflowed for {domain}"
                )
            }
            Self::LinePromotion {
                curve_index,
                source,
            } => write!(
                formatter,
                "source curve {curve_index} could not be promoted as an exact metric line: {source}"
            ),
            Self::ArcPromotion {
                curve_index,
                source,
            } => write!(
                formatter,
                "source curve {curve_index} could not be promoted as an exact metric arc: {source:?}"
            ),
            Self::UnsupportedMetricCurve {
                curve_index,
                family,
            } => write!(
                formatter,
                "source curve {curve_index} has unsupported metric family {family:?}; no display-chord fallback is allowed"
            ),
            Self::FeedCertification(source) => {
                write!(formatter, "exact feed replay failed: {source:?}")
            }
        }
    }
}

impl StdError for ToolpathError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::CurveConstruction(source) => Some(source),
            Self::ExactCurve(source) => Some(source),
            Self::Arithmetic(source) => Some(source),
            Self::LinePromotion { source, .. } => Some(source),
            Self::ArcPromotion { .. }
            | Self::InvalidMetricApproximationLimits
            | Self::InvalidMetricApproximationError
            | Self::MetricApproximationRequired { .. }
            | Self::MetricApproximationUncertain { .. }
            | Self::MetricApproximationDepthExceeded { .. }
            | Self::MetricApproximationBudgetExceeded { .. }
            | Self::AllocationOverflow { .. }
            | Self::IntegerOverflow { .. }
            | Self::FeedCertification(_)
            | Self::UnsupportedMetricCurve { .. } => None,
        }
    }
}

impl From<CurveError> for ToolpathError {
    fn from(value: CurveError) -> Self {
        Self::CurveConstruction(value)
    }
}

impl From<ExactCurveError> for ToolpathError {
    fn from(value: ExactCurveError) -> Self {
        Self::ExactCurve(value)
    }
}

impl From<Problem> for ToolpathError {
    fn from(value: Problem) -> Self {
        Self::Arithmetic(value)
    }
}

impl From<RouteCertificationError> for ToolpathError {
    fn from(value: RouteCertificationError) -> Self {
        Self::FeedCertification(value)
    }
}

/// Build a connected exact line/arc/Bezier path for renderer and boundary tests.
///
/// The line and arc are losslessly promotable to Hyperpath. The final general
/// cubic remains inside the representative machine's nonnegative travel while
/// exercising certified source-to-motion reduction.
pub fn representative_curve_path() -> ToolpathResult<CurvePath2> {
    let line = representative_line()?;
    let arc = representative_arc()?;
    let cubic = Curve2::new(CurveGeometry2::CubicBezier(CubicBezier2::new(
        CurvePoint2::from_values(8, 0),
        CurvePoint2::from_values(9, 3),
        CurvePoint2::from_values(11, 3),
        CurvePoint2::from_values(12, 0),
    )));
    Ok(CurvePath2::try_new(vec![line, arc, cubic])?)
}

/// Build the exact line/semicircle prefix used for the first metric replay.
pub fn representative_metric_path() -> ToolpathResult<CurvePath2> {
    Ok(CurvePath2::try_new(vec![
        representative_line()?,
        representative_arc()?,
    ])?)
}

/// Build one exact curved material loop with one exact rectangular hole.
///
/// Explicit loop semantics remain attached to the Hypercurve region. A later
/// display adapter may color those roles, but it does not infer them from the
/// emitted chords.
pub fn representative_curve_region() -> ToolpathResult<CurveRegion2> {
    let outer = CurvePath2::try_new(vec![
        exact_line((-10, -4), (-4, -4))?,
        Curve2::new(CurveGeometry2::CubicBezier(CubicBezier2::new(
            CurvePoint2::from_values(-4, -4),
            CurvePoint2::from_values(-1, -2),
            CurvePoint2::from_values(-1, 0),
            CurvePoint2::from_values(-4, 2),
        ))),
        exact_line((-4, 2), (-10, 2))?,
        exact_line((-10, 2), (-10, -4))?,
    ])?;
    let hole = CurvePath2::try_new(vec![
        exact_line((-8, -2), (-8, 0))?,
        exact_line((-8, 0), (-6, 0))?,
        exact_line((-6, 0), (-6, -2))?,
        exact_line((-6, -2), (-8, -2))?,
    ])?;
    Ok(CurveRegion2::try_from_boundary_paths_with_loop_semantics(
        &[outer, hole],
        &[CurveRegionLoopRole::Material, CurveRegionLoopRole::Hole],
        &[FillRule::NonZero, FillRule::NonZero],
        &hypercurve::CurveContext::STRICT,
    )?
    .into_value())
}

/// Promote supported Hypercurve source families to exact Hyperpath carriers.
///
/// This operation is all-or-nothing. It returns an explicit blocker for every
/// family without a lossless metric representation and never consumes a
/// renderer polyline or flattening certificate.
pub fn promote_metric_path(path: &CurvePath2) -> ToolpathResult<Vec<FeedPathElement>> {
    path.curves()
        .iter()
        .enumerate()
        .map(|(curve_index, curve)| match curve.geometry() {
            CurveGeometry2::Line(line) => {
                let line = LinePathSegment::new(
                    predicate_point(line.start()),
                    predicate_point(line.end()),
                    PredicatePolicy::STRICT,
                )
                .map_err(|source| ToolpathError::LinePromotion {
                    curve_index,
                    source,
                })?;
                Ok(FeedPathElement::Line(line))
            }
            CurveGeometry2::CircularArc(arc) => {
                let radius = arc.radius_squared().sqrt()?;
                let direction = if arc.is_clockwise() {
                    ArcDirection::Cw
                } else {
                    ArcDirection::Ccw
                };
                let arc = ExplicitCircularArc::new(
                    predicate_point(arc.center()),
                    radius,
                    predicate_point(arc.start()),
                    predicate_point(arc.end()),
                    direction,
                    PredicatePolicy::STRICT,
                )
                .map_err(|source| ToolpathError::ArcPromotion {
                    curve_index,
                    source,
                })?;
                Ok(FeedPathElement::ExplicitArc(arc))
            }
            geometry => Err(ToolpathError::UnsupportedMetricCurve {
                curve_index,
                family: geometry.family(),
            }),
        })
        .collect()
}

/// Build a bounded exact line/arc metric path from retained source geometry.
///
/// Lines and explicit circular arcs are preserved losslessly. Cubic Beziers
/// are admitted only under a positive caller-owned positional allocation and
/// an exact parameter-preserving, degree-elevated chord certificate. Each
/// generated chord is an exact `LineSeg2`; no renderer output or finite scalar
/// conversion is accepted. Other curve families remain explicit blockers.
pub fn certify_metric_path(
    source: &CurvePath2,
    maximum_source_error_mm_exact: Rational,
    limits: MetricPathApproximationLimits2,
) -> ToolpathResult<CertifiedMetricPath2> {
    if maximum_source_error_mm_exact < Rational::zero() {
        return Err(ToolpathError::InvalidMetricApproximationError);
    }
    if source.curves().len() > limits.maximum_motion_elements {
        return Err(ToolpathError::MetricApproximationBudgetExceeded {
            required: source.curves().len(),
            maximum: limits.maximum_motion_elements,
        });
    }

    let mut motion_curves = Vec::new();
    let mut source_element_by_motion = Vec::new();
    let mut spans = Vec::new();
    let mut used_source_error_mm_exact = Rational::zero();
    spans
        .try_reserve_exact(source.curves().len())
        .map_err(|_| ToolpathError::AllocationOverflow {
            domain: "metric source spans",
        })?;

    for (source_element, curve) in source.curves().iter().enumerate() {
        let motion_element_start = motion_curves.len();
        let source_family = curve.geometry().family();
        let (maximum_error_mm_exact, maximum_subdivision_depth) = match curve.geometry() {
            CurveGeometry2::Line(_) | CurveGeometry2::CircularArc(_) => {
                push_motion_curve(
                    &mut motion_curves,
                    &mut source_element_by_motion,
                    curve.clone(),
                    source_element,
                    limits,
                )?;
                (Rational::zero(), 0)
            }
            CurveGeometry2::CubicBezier(cubic) => {
                if maximum_source_error_mm_exact.is_zero() {
                    return Err(ToolpathError::MetricApproximationRequired {
                        curve_index: source_element,
                    });
                }
                let remaining = limits
                    .maximum_motion_elements
                    .saturating_sub(motion_curves.len());
                if remaining == 0 {
                    return Err(ToolpathError::MetricApproximationBudgetExceeded {
                        required: motion_curves.len().saturating_add(1),
                        maximum: limits.maximum_motion_elements,
                    });
                }
                let budget_depth = if remaining == 1 {
                    0
                } else {
                    usize::try_from(usize::BITS - 1 - remaining.leading_zeros()).map_err(|_| {
                        ToolpathError::IntegerOverflow {
                            domain: "metric subdivision depth",
                        }
                    })?
                };
                let requested_depth = limits.maximum_subdivision_depth.min(budget_depth);
                let polyline = certify_cubic_motion_polyline(
                    cubic,
                    &maximum_source_error_mm_exact,
                    requested_depth,
                    remaining,
                    motion_curves.len(),
                    limits.maximum_motion_elements,
                    source_element,
                )?;
                let chord_count = polyline.points.len().saturating_sub(1);
                let required = motion_curves.len().checked_add(chord_count).ok_or(
                    ToolpathError::IntegerOverflow {
                        domain: "certified metric chords",
                    },
                )?;
                if chord_count == 0 || required > limits.maximum_motion_elements {
                    return Err(ToolpathError::MetricApproximationBudgetExceeded {
                        required,
                        maximum: limits.maximum_motion_elements,
                    });
                }
                motion_curves.try_reserve(chord_count).map_err(|_| {
                    ToolpathError::AllocationOverflow {
                        domain: "certified metric chords",
                    }
                })?;
                source_element_by_motion
                    .try_reserve(chord_count)
                    .map_err(|_| ToolpathError::AllocationOverflow {
                        domain: "metric motion provenance",
                    })?;
                for pair in polyline.points.windows(2) {
                    motion_curves.push(Curve2::new(CurveGeometry2::Line(LineSeg2::try_new(
                        pair[0].clone(),
                        pair[1].clone(),
                    )?)));
                    source_element_by_motion.push(source_element);
                }
                used_source_error_mm_exact = maximum_source_error_mm_exact.clone();
                (
                    maximum_source_error_mm_exact.clone(),
                    polyline.maximum_subdivision_depth,
                )
            }
            geometry => {
                return Err(ToolpathError::UnsupportedMetricCurve {
                    curve_index: source_element,
                    family: geometry.family(),
                });
            }
        };
        spans.push(CertifiedMetricSourceSpan2 {
            source_element,
            source_family,
            motion_element_start,
            motion_element_count: motion_curves.len() - motion_element_start,
            maximum_error_mm_exact,
            maximum_subdivision_depth,
        });
    }

    Ok(CertifiedMetricPath2 {
        path: CurvePath2::try_new(motion_curves)?,
        spans,
        source_element_by_motion,
        maximum_source_error_mm_exact: used_source_error_mm_exact,
    })
}

struct CertifiedCubicMotionPolyline2 {
    points: Vec<CurvePoint2>,
    maximum_subdivision_depth: usize,
}

/// Certify a pointwise parameter-preserving cubic-to-chord bound.
///
/// A line from `P0` to `P3`, elevated to cubic degree, has controls
/// `P0`, `lerp(P0, P3, 1/3)`, `lerp(P0, P3, 2/3)`, and `P3`. Subtracting those
/// controls from the source cubic gives the exact Bezier representation of
/// `source(t) - chord(t)`. If both interior difference-control norms are no
/// greater than the requested error, convexity proves that same bound for
/// every `t` in `[0, 1]`. This stronger motion predicate also catches a
/// collinear cubic that reverses along its supporting line.
fn certify_cubic_motion_polyline(
    cubic: &CubicBezier2,
    maximum_error: &Rational,
    maximum_subdivision_depth: usize,
    maximum_chords: usize,
    motion_prefix: usize,
    maximum_motion_elements: usize,
    curve_index: usize,
) -> ToolpathResult<CertifiedCubicMotionPolyline2> {
    let maximum_error = Real::from(maximum_error.clone());
    let maximum_error_squared = &maximum_error * &maximum_error;
    let half = (Real::one() / Real::from(2))?;
    let third = (Real::one() / Real::from(3))?;
    let two_thirds = Real::from(2) * &third;
    let mut points = Vec::new();
    points
        .try_reserve(1)
        .map_err(|_| ToolpathError::AllocationOverflow {
            domain: "certified cubic motion points",
        })?;
    points.push(cubic.start().clone());
    let mut maximum_depth_used = 0;
    certify_cubic_motion_recursive(
        cubic.clone(),
        &maximum_error_squared,
        &half,
        &third,
        &two_thirds,
        maximum_subdivision_depth,
        0,
        maximum_chords,
        motion_prefix,
        maximum_motion_elements,
        curve_index,
        &mut points,
        &mut maximum_depth_used,
    )?;
    Ok(CertifiedCubicMotionPolyline2 {
        points,
        maximum_subdivision_depth: maximum_depth_used,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "recursive exact certification carries explicit numerical and resource bounds"
)]
fn certify_cubic_motion_recursive(
    cubic: CubicBezier2,
    maximum_error_squared: &Real,
    half: &Real,
    third: &Real,
    two_thirds: &Real,
    maximum_subdivision_depth: usize,
    depth: usize,
    maximum_chords: usize,
    motion_prefix: usize,
    maximum_motion_elements: usize,
    curve_index: usize,
    points: &mut Vec<CurvePoint2>,
    maximum_depth_used: &mut usize,
) -> ToolpathResult<()> {
    *maximum_depth_used = (*maximum_depth_used).max(depth);
    if cubic_pointwise_chord_within(
        &cubic,
        maximum_error_squared,
        third,
        two_thirds,
        curve_index,
    )? {
        let proposed_chord_count = points.len();
        if proposed_chord_count > maximum_chords {
            let required = motion_prefix.checked_add(proposed_chord_count).ok_or(
                ToolpathError::IntegerOverflow {
                    domain: "certified cubic motion chords",
                },
            )?;
            return Err(ToolpathError::MetricApproximationBudgetExceeded {
                required,
                maximum: maximum_motion_elements,
            });
        }
        points
            .try_reserve(1)
            .map_err(|_| ToolpathError::AllocationOverflow {
                domain: "certified cubic motion points",
            })?;
        points.push(cubic.end().clone());
        return Ok(());
    }
    if depth >= maximum_subdivision_depth {
        return Err(ToolpathError::MetricApproximationDepthExceeded {
            curve_index,
            maximum_depth: maximum_subdivision_depth,
        });
    }
    let (left, right) = cubic.split_at_exact(half.clone());
    certify_cubic_motion_recursive(
        left,
        maximum_error_squared,
        half,
        third,
        two_thirds,
        maximum_subdivision_depth,
        depth + 1,
        maximum_chords,
        motion_prefix,
        maximum_motion_elements,
        curve_index,
        points,
        maximum_depth_used,
    )?;
    certify_cubic_motion_recursive(
        right,
        maximum_error_squared,
        half,
        third,
        two_thirds,
        maximum_subdivision_depth,
        depth + 1,
        maximum_chords,
        motion_prefix,
        maximum_motion_elements,
        curve_index,
        points,
        maximum_depth_used,
    )
}

fn cubic_pointwise_chord_within(
    cubic: &CubicBezier2,
    maximum_error_squared: &Real,
    third: &Real,
    two_thirds: &Real,
    curve_index: usize,
) -> ToolpathResult<bool> {
    let chord_control1 = cubic.start().lerp(cubic.end(), third.clone());
    let chord_control2 = cubic.start().lerp(cubic.end(), two_thirds.clone());
    for distance_squared in [
        cubic.control1().distance_squared(&chord_control1),
        cubic.control2().distance_squared(&chord_control2),
    ] {
        match compare_reals(
            &distance_squared,
            maximum_error_squared,
            PredicatePolicy::STRICT,
        )
        .value()
        {
            Some(Ordering::Less | Ordering::Equal) => {}
            Some(Ordering::Greater) => return Ok(false),
            None => {
                return Err(ToolpathError::MetricApproximationUncertain {
                    curve_index,
                    reason: UncertaintyReason::Ordering,
                });
            }
        }
    }
    Ok(true)
}

fn push_motion_curve(
    motion_curves: &mut Vec<Curve2>,
    source_element_by_motion: &mut Vec<usize>,
    curve: Curve2,
    source_element: usize,
    limits: MetricPathApproximationLimits2,
) -> ToolpathResult<()> {
    let required = motion_curves
        .len()
        .checked_add(1)
        .ok_or(ToolpathError::IntegerOverflow {
            domain: "lossless metric curves",
        })?;
    if required > limits.maximum_motion_elements {
        return Err(ToolpathError::MetricApproximationBudgetExceeded {
            required,
            maximum: limits.maximum_motion_elements,
        });
    }
    motion_curves
        .try_reserve(1)
        .map_err(|_| ToolpathError::AllocationOverflow {
            domain: "lossless metric curves",
        })?;
    source_element_by_motion
        .try_reserve(1)
        .map_err(|_| ToolpathError::AllocationOverflow {
            domain: "metric motion provenance",
        })?;
    motion_curves.push(curve);
    source_element_by_motion.push(source_element);
    Ok(())
}

/// Certify the representative line/semicircle path at one exact unit per time unit.
///
/// The target time is exactly `4 + 2*pi`: a four-unit line followed by a
/// radius-two half turn. Hyperpath replays the same retained symbolic length
/// rather than measuring display chords.
pub fn representative_feed_certificate() -> ToolpathResult<ConstantFeedTimeReport> {
    let source = representative_metric_path()?;
    let route = promote_metric_path(&source)?;
    let target_time = Real::from(4) + Real::from(2) * Real::pi();
    Ok(certify_constant_feed_time_for_path(
        &route,
        Real::one(),
        target_time,
        PredicatePolicy::STRICT,
    )?)
}

fn representative_line() -> Result<Curve2, CurveError> {
    exact_line((0, 0), (4, 0))
}

fn representative_arc() -> Result<Curve2, CurveError> {
    Ok(Curve2::new(CurveGeometry2::CircularArc(
        CircularArc2::try_from_center(
            CurvePoint2::from_values(4, 0),
            CurvePoint2::from_values(8, 0),
            CurvePoint2::from_values(6, 0),
            true,
        )?,
    )))
}

fn predicate_point(point: &CurvePoint2) -> PredicatePoint2 {
    PredicatePoint2::new(point.x().clone(), point.y().clone())
}

fn exact_line(start: (i64, i64), end: (i64, i64)) -> Result<Curve2, CurveError> {
    Ok(Curve2::new(CurveGeometry2::Line(LineSeg2::try_new(
        CurvePoint2::from_values(start.0, start.1),
        CurvePoint2::from_values(end.0, end.1),
    )?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_promotion_preserves_line_and_arc_objects_exactly() {
        let path = representative_metric_path().unwrap();
        let route = promote_metric_path(&path).unwrap();

        assert_eq!(route.len(), 2);
        let FeedPathElement::Line(line) = &route[0] else {
            panic!("first retained element must be the exact source line");
        };
        assert_eq!(line.start().x, Real::zero());
        assert_eq!(line.end().x, Real::from(4));
        let FeedPathElement::ExplicitArc(arc) = &route[1] else {
            panic!("second retained element must be the exact source arc");
        };
        assert_eq!(arc.center().x, Real::from(6));
        assert_eq!(arc.radius(), &Real::from(2));
        assert_eq!(arc.direction(), ArcDirection::Cw);
    }

    #[test]
    fn general_bezier_is_not_silently_demoted_to_display_chords() {
        let path = representative_curve_path().unwrap();

        assert!(matches!(
            promote_metric_path(&path),
            Err(ToolpathError::UnsupportedMetricCurve {
                curve_index: 2,
                family: CurveFamily2::CubicBezier,
            })
        ));
    }

    #[test]
    fn lossless_metric_certification_reports_zero_source_error() {
        let source = representative_metric_path().unwrap();
        let certified = certify_metric_path(
            &source,
            Rational::zero(),
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        assert_eq!(certified.path(), &source);
        assert_eq!(certified.maximum_source_error_mm_exact(), &Rational::zero());
        assert_eq!(certified.spans().len(), source.curves().len());
        assert!(certified.spans().iter().all(|span| {
            span.motion_element_count() == 1
                && span.maximum_error_mm_exact().is_zero()
                && !span.is_approximated()
        }));
        assert_eq!(certified.source_element_for_motion(0), Some(0));
        assert_eq!(certified.source_element_for_motion(1), Some(1));
        assert_eq!(certified.source_element_for_motion(2), None);
    }

    #[test]
    fn cubic_metric_certification_retains_exact_chords_and_provenance() {
        let source = representative_curve_path().unwrap();
        let allocation = Rational::from(1) / Rational::from(1_000);
        let certified = certify_metric_path(
            &source,
            allocation.clone(),
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        assert_eq!(certified.spans().len(), 3);
        assert_eq!(certified.maximum_source_error_mm_exact(), &allocation);
        let cubic_span = &certified.spans()[2];
        assert_eq!(cubic_span.source_element(), 2);
        assert_eq!(cubic_span.source_family(), CurveFamily2::CubicBezier);
        assert_eq!(cubic_span.maximum_error_mm_exact(), &allocation);
        assert!(cubic_span.is_approximated());
        assert!(cubic_span.motion_element_count() > 1);
        assert!(cubic_span.maximum_subdivision_depth() > 0);

        let cubic_motion = &certified.path().curves()[cubic_span.motion_element_start()
            ..cubic_span.motion_element_start() + cubic_span.motion_element_count()];
        assert!(
            cubic_motion
                .iter()
                .all(|curve| { matches!(curve.geometry(), CurveGeometry2::Line(_)) })
        );
        assert!(
            (cubic_span.motion_element_start()
                ..cubic_span.motion_element_start() + cubic_span.motion_element_count())
                .all(|motion_element| {
                    certified.source_element_for_motion(motion_element) == Some(2)
                })
        );

        let CurveGeometry2::CubicBezier(source_cubic) = source.curves()[2].geometry() else {
            panic!("representative source must retain its cubic");
        };
        let CurveGeometry2::Line(first_chord) = cubic_motion[0].geometry() else {
            unreachable!();
        };
        let CurveGeometry2::Line(last_chord) = cubic_motion[cubic_motion.len() - 1].geometry()
        else {
            unreachable!();
        };
        assert_eq!(first_chord.start(), source_cubic.start());
        assert_eq!(last_chord.end(), source_cubic.end());

        let route = promote_metric_path(certified.path()).unwrap();
        assert_eq!(route.len(), certified.path().curves().len());
    }

    #[test]
    fn cubic_metric_certification_requires_an_explicit_error_allocation() {
        let source = representative_curve_path().unwrap();

        assert!(matches!(
            certify_metric_path(
                &source,
                Rational::zero(),
                MetricPathApproximationLimits2::INTERACTIVE,
            ),
            Err(ToolpathError::MetricApproximationRequired { curve_index: 2 })
        ));
    }

    #[test]
    fn cubic_metric_certification_fails_closed_at_the_selected_depth() {
        let cubic = representative_curve_path().unwrap().curves()[2].clone();
        let source = CurvePath2::try_new(vec![cubic]).unwrap();
        let allocation = Rational::from(1) / Rational::from(1_000);
        let limits = MetricPathApproximationLimits2::try_new(1, 1).unwrap();

        assert!(matches!(
            certify_metric_path(&source, allocation, limits),
            Err(ToolpathError::MetricApproximationDepthExceeded {
                curve_index: 0,
                maximum_depth: 0,
            })
        ));
    }

    #[test]
    fn collinear_backtracking_cubic_cannot_collapse_to_its_endpoint_chord() {
        let cubic = CubicBezier2::new(
            CurvePoint2::from_values(0, 0),
            CurvePoint2::from_values(-4, 0),
            CurvePoint2::from_values(5, 0),
            CurvePoint2::from_values(1, 0),
        );
        let source =
            CurvePath2::try_new(vec![Curve2::new(CurveGeometry2::CubicBezier(cubic))]).unwrap();
        let certified = certify_metric_path(
            &source,
            Rational::from(1) / Rational::from(100),
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        assert!(certified.path().curves().len() > 1);
        let retains_negative_excursion = certified.path().curves().iter().any(|curve| {
            let CurveGeometry2::Line(line) = curve.geometry() else {
                return false;
            };
            [line.start().x(), line.end().x()].into_iter().any(|x| {
                compare_reals(x, &Real::zero(), PredicatePolicy::STRICT).value()
                    == Some(Ordering::Less)
            })
        });
        let retains_positive_overshoot = certified.path().curves().iter().any(|curve| {
            let CurveGeometry2::Line(line) = curve.geometry() else {
                return false;
            };
            [line.start().x(), line.end().x()].into_iter().any(|x| {
                compare_reals(x, &Real::one(), PredicatePolicy::STRICT).value()
                    == Some(Ordering::Greater)
            })
        });
        assert!(retains_negative_excursion);
        assert!(retains_positive_overshoot);
        promote_metric_path(certified.path()).unwrap();
    }

    #[test]
    fn exact_symbolic_feed_replay_uses_retained_arc_length() {
        let report = representative_feed_certificate().unwrap();
        let expected = Real::from(4) + Real::from(2) * Real::pi();

        assert_eq!(report.path_length, expected);
        assert!(report.certification.all_satisfied());
    }

    #[test]
    fn representative_region_retains_explicit_material_and_hole_roles() {
        let region = representative_curve_region().unwrap();
        let roles = region
            .loop_roles(&hypercurve::CurveContext::STRICT)
            .unwrap()
            .into_value();
        let hypercurve::Classification::Decided(roles) = roles else {
            panic!("explicit region roles must remain decided");
        };

        assert_eq!(
            roles,
            vec![CurveRegionLoopRole::Material, CurveRegionLoopRole::Hole,]
        );
    }
}
