//! Exact scene composition using CSGRS geometry and Hypergraphics ownership.

use csgrs::solid::{self, SolidExt as _};
use hypergraphics::{Color3, ExactMesh, Real, Result, axes_mesh, grid_mesh, triangle_mesh};

/// A collection of exact Hypergraphics meshes with no GPU or window state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExactScene {
    meshes: Vec<ExactMesh>,
}

impl ExactScene {
    /// Build the first exact-stack scene from local CSGRS and Hypergraphics.
    pub fn baseline() -> Result<Self> {
        let grid = grid_mesh(12, Real::from(1), Color3::new(0.18, 0.21, 0.25)?)?;
        let axes = axes_mesh(Real::from(6), Real::zero())?;
        let solid =
            solid::cube(Real::from(4)).translated(Real::from(-2), Real::from(-2), Real::zero());
        let solid = triangle_mesh(&solid, Color3::new(0.18, 0.62, 0.82)?)?;
        Ok(Self {
            meshes: vec![grid, axes, solid],
        })
    }

    /// Borrow the exact scene meshes.
    pub fn meshes(&self) -> &[ExactMesh] {
        &self.meshes
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
    fn baseline_uses_current_native_csgrs_mesh() {
        let scene = ExactScene::baseline().unwrap();
        assert_eq!(scene.meshes().len(), 3);
        assert_eq!(scene.triangle_count(), 12);
        assert!(scene.vertex_count() > 36);
    }
}
