//! Window-free exact CAD/CAM state and explicit machine/display boundaries.
//!
//! This crate intentionally has no eframe, GPU-context, browser-storage, or
//! network dependency. It is the shared native/WASM authority for exact source
//! values and the only place that may compile them toward canonical machine IR.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod boundary;
pub mod compiler;
pub mod partition;
pub mod scene;
pub mod toolpath;

pub use boundary::{
    BoundaryError, BoundedMeasurement, CanonicalCycle, CanonicalStep, DisplayScalar, ExactValue,
    Millimetres, Seconds, Unit, canonical_motion_segment, project_for_display,
};
pub use compiler::{
    CanonicalPathPoint2, CanonicalPathProgram2, CanonicalTimeBoundary, MachineCompileError,
    MachineCompileResult, MotionApproximationEvidence2, MotionCompilePolicy2,
    compile_certified_chord_program, compile_representative_program,
};
pub use partition::{
    CanonicalMachinePartition2, CanonicalPartitionChunk, MachinePartitionError,
    MachinePartitionPolicy2, MachinePartitionResult, package_canonical_program,
    representative_partition_policy,
};
pub use scene::{CurveDisplayEvidence, CurveRegionDisplayEvidence, ExactScene, SceneError};
pub use toolpath::{
    ToolpathError, ToolpathResult, promote_metric_path, representative_curve_path,
    representative_curve_region, representative_feed_certificate, representative_metric_path,
};

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
