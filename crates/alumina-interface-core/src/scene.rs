//! Exact scene composition using current sibling CSGRS, Hypercurve, and Hypergraphics.

use std::error::Error as StdError;
use std::fmt;

use csgrs::solid::{self, SolidExt as _};
use hypercurve::{
    BezierFlatteningOptions, CurveCertainty, CurveContext, CurvePath2, CurveRegion2,
    CurveRegionLoopRole,
};
use hypergraphics::{
    Color3, ExactMesh, Real, axes_mesh, curve_path_line_mesh, curve_region_line_mesh, grid_mesh,
    triangle_mesh,
};

use crate::toolpath::{ToolpathError, representative_curve_path, representative_curve_region};

/// Failures while composing exact, window-free scene state.
#[derive(Debug)]
pub enum SceneError {
    /// Exact source geometry or path connectivity failed.
    Toolpath(ToolpathError),
    /// Hypergraphics rejected an exact presentation adapter.
    Graphics(hypergraphics::Error),
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toolpath(source) => write!(formatter, "toolpath scene input failed: {source}"),
            Self::Graphics(source) => write!(formatter, "exact scene adapter failed: {source}"),
        }
    }
}

impl StdError for SceneError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Toolpath(source) => Some(source),
            Self::Graphics(source) => Some(source),
        }
    }
}

impl From<ToolpathError> for SceneError {
    fn from(value: ToolpathError) -> Self {
        Self::Toolpath(value)
    }
}

impl From<hypergraphics::Error> for SceneError {
    fn from(value: hypergraphics::Error) -> Self {
        Self::Graphics(value)
    }
}

/// Retained proof boundary for one exact source path's display chords.
///
/// These values authorize presentation only. They are never accepted as CAM
/// metric evidence or promoted back into source geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveDisplayEvidence {
    max_source_chord_error: Real,
    chord_segment_count: usize,
    maximum_subdivision_depth: usize,
    source_fragment_count: usize,
}

impl CurveDisplayEvidence {
    /// Return the certified source-curve-to-chord display error bound.
    pub const fn max_source_chord_error(&self) -> &Real {
        &self.max_source_chord_error
    }

    /// Return the number of certified independent display chords.
    pub const fn chord_segment_count(&self) -> usize {
        self.chord_segment_count
    }

    /// Return the deepest exact-predicate subdivision used.
    pub const fn maximum_subdivision_depth(&self) -> usize {
        self.maximum_subdivision_depth
    }

    /// Return the number of native Hypercurve fragments covered by the certificate.
    pub const fn source_fragment_count(&self) -> usize {
        self.source_fragment_count
    }
}

/// Retained one-way display evidence for an exact curved region.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveRegionDisplayEvidence {
    max_source_chord_error: Real,
    loop_count: usize,
    material_loop_count: usize,
    hole_loop_count: usize,
    chord_segment_count: usize,
    source_fragment_count: usize,
    path_materialization_certainty: CurveCertainty,
    role_certainty: CurveCertainty,
}

impl CurveRegionDisplayEvidence {
    /// Return the certified source-curve-to-chord display error bound.
    pub const fn max_source_chord_error(&self) -> &Real {
        &self.max_source_chord_error
    }

    /// Return the number of retained region boundary loops.
    pub const fn loop_count(&self) -> usize {
        self.loop_count
    }

    /// Return the number of authoritative material loops.
    pub const fn material_loop_count(&self) -> usize {
        self.material_loop_count
    }

    /// Return the number of authoritative hole loops.
    pub const fn hole_loop_count(&self) -> usize {
        self.hole_loop_count
    }

    /// Return the aggregate certified region-boundary chord count.
    pub const fn chord_segment_count(&self) -> usize {
        self.chord_segment_count
    }

    /// Return the aggregate native Hypercurve fragment count.
    pub const fn source_fragment_count(&self) -> usize {
        self.source_fragment_count
    }

    /// Return certainty consumed while materializing region paths.
    pub const fn path_materialization_certainty(&self) -> CurveCertainty {
        self.path_materialization_certainty
    }

    /// Return certainty consumed while classifying material and hole roles.
    pub const fn role_certainty(&self) -> CurveCertainty {
        self.role_certainty
    }
}

/// A collection of exact Hypergraphics meshes with no GPU or window state.
#[derive(Clone, Debug, Default)]
pub struct ExactScene {
    meshes: Vec<ExactMesh>,
    displayed_curve_source: Option<CurvePath2>,
    curve_display_evidence: Option<CurveDisplayEvidence>,
    displayed_region_source: Option<CurveRegion2>,
    region_display_evidence: Option<CurveRegionDisplayEvidence>,
}

impl ExactScene {
    /// Build the first exact-stack scene from the current local working trees.
    pub fn baseline() -> Result<Self, SceneError> {
        let grid = grid_mesh(12, Real::from(1), Color3::new(0.18, 0.21, 0.25)?)?;
        let axes = axes_mesh(Real::from(6), Real::zero())?;
        let solid =
            solid::cube(Real::from(4)).translated(Real::from(-2), Real::from(-2), Real::zero());
        let solid = triangle_mesh(&solid, Color3::new(0.18, 0.62, 0.82)?)?;

        let curve_source = representative_curve_path()?;
        let max_source_chord_error =
            Real::from(hyperreal::Rational::fraction(1, 1_024).map_err(ToolpathError::from)?);
        let flattening = BezierFlatteningOptions::try_new(
            max_source_chord_error.clone(),
            24,
            &CurveContext::STRICT,
        )
        .map_err(ToolpathError::from)?;
        let certified = curve_path_line_mesh(
            &curve_source,
            &flattening,
            &CurveContext::STRICT,
            Real::from(5),
            Color3::new(0.95, 0.45, 0.12)?,
        )?;
        let curve_display_evidence = CurveDisplayEvidence {
            max_source_chord_error: certified.max_error().clone(),
            chord_segment_count: certified.segment_count(),
            maximum_subdivision_depth: certified.max_depth(),
            source_fragment_count: certified.source_fragment_count(),
        };
        let curve_mesh = certified.into_mesh();

        let region_source = representative_curve_region()?;
        let certified_region = curve_region_line_mesh(
            &region_source,
            &flattening,
            &CurveContext::STRICT,
            Real::from(5),
            Color3::new(0.72, 0.25, 0.82)?,
            Color3::new(0.96, 0.76, 0.18)?,
        )?;
        let material_loop_count = certified_region
            .loop_evidence()
            .iter()
            .filter(|loop_evidence| loop_evidence.role() == CurveRegionLoopRole::Material)
            .count();
        let hole_loop_count = certified_region
            .loop_evidence()
            .iter()
            .filter(|loop_evidence| loop_evidence.role() == CurveRegionLoopRole::Hole)
            .count();
        let region_display_evidence = CurveRegionDisplayEvidence {
            max_source_chord_error,
            loop_count: certified_region.loop_evidence().len(),
            material_loop_count,
            hole_loop_count,
            chord_segment_count: certified_region.segment_count(),
            source_fragment_count: certified_region
                .loop_evidence()
                .iter()
                .map(|loop_evidence| loop_evidence.source_fragment_count())
                .sum(),
            path_materialization_certainty: certified_region.path_materialization_certainty(),
            role_certainty: certified_region.role_certainty(),
        };
        let region_mesh = certified_region.into_mesh();

        Ok(Self {
            meshes: vec![grid, axes, solid, region_mesh, curve_mesh],
            displayed_curve_source: Some(curve_source),
            curve_display_evidence: Some(curve_display_evidence),
            displayed_region_source: Some(region_source),
            region_display_evidence: Some(region_display_evidence),
        })
    }

    /// Borrow the exact scene meshes.
    pub fn meshes(&self) -> &[ExactMesh] {
        &self.meshes
    }

    /// Borrow the exact source path used by the current curve presentation.
    pub const fn displayed_curve_source(&self) -> Option<&CurvePath2> {
        self.displayed_curve_source.as_ref()
    }

    /// Borrow the one-way certified curve-display evidence.
    pub const fn curve_display_evidence(&self) -> Option<&CurveDisplayEvidence> {
        self.curve_display_evidence.as_ref()
    }

    /// Borrow the exact source region used by the current region presentation.
    pub const fn displayed_region_source(&self) -> Option<&CurveRegion2> {
        self.displayed_region_source.as_ref()
    }

    /// Borrow the one-way certified region-display evidence.
    pub const fn region_display_evidence(&self) -> Option<&CurveRegionDisplayEvidence> {
        self.region_display_evidence.as_ref()
    }

    /// Return the total exact scene-vertex count.
    pub fn vertex_count(&self) -> usize {
        self.meshes.iter().map(ExactMesh::vertex_count).sum()
    }

    /// Return the total exact triangle count.
    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().map(ExactMesh::triangle_count).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_uses_current_native_csgrs_mesh_and_certified_curve_adapter() {
        let scene = ExactScene::baseline().unwrap();
        assert_eq!(scene.meshes().len(), 5);
        assert_eq!(scene.triangle_count(), 12);
        assert!(scene.vertex_count() > 36);

        let source = scene.displayed_curve_source().unwrap();
        assert_eq!(source.curves().len(), 3);
        let evidence = scene.curve_display_evidence().unwrap();
        assert_eq!(
            evidence.max_source_chord_error(),
            &Real::from(hyperreal::Rational::fraction(1, 1_024).unwrap())
        );
        assert!(evidence.chord_segment_count() > 3);
        assert!(evidence.source_fragment_count() >= source.curves().len());
        assert!(evidence.maximum_subdivision_depth() <= 24);

        let region = scene.displayed_region_source().unwrap();
        assert_eq!(region.len(), 2);
        let region_evidence = scene.region_display_evidence().unwrap();
        assert_eq!(region_evidence.loop_count(), 2);
        assert_eq!(region_evidence.material_loop_count(), 1);
        assert_eq!(region_evidence.hole_loop_count(), 1);
        assert!(region_evidence.chord_segment_count() > 8);
        assert!(region_evidence.source_fragment_count() >= 8);
        assert_eq!(
            region_evidence.path_materialization_certainty(),
            CurveCertainty::Certified
        );
        assert_eq!(region_evidence.role_certainty(), CurveCertainty::Certified);
    }
}
