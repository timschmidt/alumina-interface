//! Exact source-curve fixtures and the first checked Hypercurve-to-Hyperpath boundary.
//!
//! Hypercurve remains the source-geometry authority. Hyperpath receives a new
//! exact metric carrier only when the source family has a lossless promotion.
//! In particular, a general Bezier is never silently replaced by display
//! chords for feed scheduling.

use std::error::Error as StdError;
use std::fmt;

use hypercurve::{
    CircularArc2, CubicBezier2, Curve2, CurveError, CurveFamily2, CurveGeometry2, CurvePath2,
    CurveRegion2, CurveRegionLoopRole, ExactCurveError, FillRule, LineSeg2, Point2 as CurvePoint2,
};
use hyperlimit::{Point2 as PredicatePoint2, PredicatePolicy};
use hyperpath::{
    ArcDirection, CircularArcError, ConstantFeedTimeReport, ExplicitCircularArc, FeedPathElement,
    LinePathSegment, LinePathSegmentError, RouteCertificationError,
    certify_constant_feed_time_for_path,
};
use hyperreal::{Problem, Real};

/// Result type for window-free exact toolpath construction and promotion.
pub type ToolpathResult<T> = Result<T, ToolpathError>;

/// A failure at an explicit source-geometry or metric-promotion boundary.
#[derive(Debug)]
pub enum ToolpathError {
    /// Hypercurve rejected source geometry before a path existed.
    CurveConstruction(CurveError),
    /// Hypercurve rejected exact path topology or connectivity.
    ExactCurve(ExactCurveError),
    /// Hyperreal rejected an exact derived quantity such as an arc radius.
    Arithmetic(Problem),
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
/// The line and arc are also losslessly promotable to Hyperpath. The final
/// general cubic deliberately exercises the current metric-compiler blocker.
pub fn representative_curve_path() -> ToolpathResult<CurvePath2> {
    let line = representative_line()?;
    let arc = representative_arc()?;
    let cubic = Curve2::new(CurveGeometry2::CubicBezier(CubicBezier2::new(
        CurvePoint2::from_values(8, 0),
        CurvePoint2::from_values(9, 3),
        CurvePoint2::from_values(11, -3),
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
