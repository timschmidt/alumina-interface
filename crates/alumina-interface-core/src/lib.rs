//! Window-free exact CAD/CAM state and explicit machine/display boundaries.
//!
//! This crate intentionally has no eframe, GPU-context, browser-storage, or
//! network dependency. It is the shared native/WASM authority for exact source
//! values and the only place that may compile them toward canonical machine IR.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod boundary;
pub mod scene;

pub use boundary::{
    BoundaryError, BoundedMeasurement, CanonicalCycle, CanonicalStep, DisplayScalar, ExactValue,
    Millimetres, Seconds, Unit, canonical_motion_segment, project_for_display,
};
pub use scene::ExactScene;

/// The current local path-planning carrier selected for authoritative CAM.
pub use hyperpath::FeedPathElement;
/// The current local exact/certified constraint problem selected for CAM.
pub use hypersolve::Problem as ConstraintProblem;

/// Construct an empty current-stack path carrier for compiler pipeline setup.
pub fn empty_path() -> Vec<FeedPathElement> {
    Vec::new()
}

/// Construct an empty current-stack constraint problem for compiler setup.
pub fn empty_constraint_problem() -> ConstraintProblem {
    ConstraintProblem::default()
}
