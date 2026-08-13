//! Window-free exact CAD/CAM state and explicit machine/display boundaries.
//!
//! This crate intentionally has no eframe, GPU-context, browser-storage, or
//! network dependency. It is the shared native/WASM authority for exact source
//! values and the only place that may compile them toward canonical machine IR.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod board_explorer;
pub mod boundary;
pub mod compiler;
pub mod diagnostics;
pub mod global_job;
pub mod graph;
pub mod machine_profile;
pub mod motion_schedule;
pub mod partition;
pub mod scene;
pub mod schedule_evidence;
pub mod toolpath;

pub use board_explorer::{
    BoardExplorerError, BoardExplorerHotspot, BoardExplorerResource, BoardExplorerResourceSummary,
    BoardExplorerSnapshot, BoardExplorerVisual, build_board_explorer_snapshot,
};
pub use boundary::{
    BoundaryError, BoundedMeasurement, CanonicalCycle, CanonicalStep, DisplayScalar, ExactValue,
    Millimetres, Seconds, Unit, canonical_motion_segment, project_for_display,
};
pub use compiler::{
    CanonicalPathPoint2, CanonicalPathProgram2, CanonicalTimeBoundary, MachineCompileError,
    MachineCompileResult, MotionApproximationEvidence2, MotionCompilePolicy2,
    compile_certified_chord_program, compile_representative_program,
};
pub use diagnostics::{
    DiagnosticExplorerError, DiagnosticExplorerSnapshot, build_diagnostic_explorer_snapshot,
};
pub use global_job::{
    CanonicalGlobalJob2, CanonicalGlobalManifestChunk, GlobalJobCompileError,
    GlobalJobCompilePolicy, GlobalJobCompileResult, MachineJobParticipantPackage2,
    compile_global_job, compile_representative_global_job,
};
pub use graph::{
    BaseDimensions, CanonicalGraphEncoding, CanonicalGraphProbeEncoding, CanonicalGraphTrace,
    ChannelFullPolicy, ClockDefinition, ClockKind, CombinationalCycle, DependencyLink,
    ExecutionDomain, ExecutionDomainSet, ExternalStreamSample, GRAPH_CHANNEL_ENVELOPE_BYTES,
    GRAPH_DOCUMENT_MAGIC, GRAPH_DOCUMENT_VERSION, GRAPH_PROBE_MAGIC, GRAPH_PROBE_VERSION,
    GRAPH_TRACE_MAGIC, GRAPH_TRACE_VERSION, GraphAnalysis, GraphAnalysisError, GraphAnalysisLimits,
    GraphCapabilityCatalogError, GraphCapabilityCatalogLimits, GraphCapabilityNodeCatalog,
    GraphCapabilityNodeEntry, GraphChannelAllocation, GraphClockId, GraphClockRate,
    GraphDeploymentError, GraphDeploymentImplementation, GraphDeploymentLimits,
    GraphDeploymentNodeKind, GraphDeploymentRegistry, GraphDeploymentReport, GraphDeploymentTarget,
    GraphDocument, GraphDocumentError, GraphLimits, GraphNodeId, GraphNodeRegistry, GraphPortId,
    GraphProbeCapture, GraphProbeDefinition, GraphProbeDocument, GraphProbeError, GraphProbeId,
    GraphProbeLimits, GraphProbeReplay, GraphRateTransition, GraphReplay, GraphSchema,
    GraphSchemaError, GraphSimulation, GraphSimulationError, GraphSimulationHorizon,
    GraphSimulationImplementation, GraphSimulationLimits, GraphSimulationNodeKind,
    GraphSimulationRegistry, GraphStorageError, GraphTraceEntry, GraphTraceEntryKind,
    GraphTraceError, GraphTraceReplay, GraphTypeId, GraphTypeStorageBound, GraphTypeStorageKind,
    GraphValue, GraphValueKind, GraphWireError, GraphWireId, InputConnectionRequirement,
    JobGraphHandle, NodeDefinition, NodeInputChannelContract, NodeInputChannelKind, NodeKind,
    NodeOutputDependency, NodeParameter, NodeParameterContract, NodeRateTransitionContract,
    NodeRegistryError, NodeSchema, NodeStateAllocation, NodeStateContract, PortDefinition,
    RateTransitionKind, RecordField, RecordFieldId, RecordValueField, ResourceClassId,
    ResourceGraphHandle, TypeDefinition, TypeKind, TypedGraphValue, UnitDefinition, UnitId,
    WireDefinition, WireEndpoint, analyze_graph, derive_graph_capability_node_catalog,
    encode_graph_document, encode_graph_probes, encode_graph_trace, graph_resource_label,
    lower_graph_deployment, replay_graph_document, replay_graph_probes, replay_graph_trace,
    simulate_graph,
};
pub use machine_profile::{
    ExactInterval, MachineDynamicsProfile2, MachineProfileError, MachineProfileResult,
    MachineResolutionBudget2, StepperAxisMachineProfile,
};
pub use motion_schedule::{
    CanonicalScheduledProgram2, CertifiedExactStopSchedule2, MotionScheduleError,
    MotionScheduleResult, ScalarMotionLimits2, ScheduledLoweringEvidence2, ScheduledMachinePoint2,
    certify_exact_stop_jerk_schedule, lower_certified_schedule_to_v1,
};
pub use partition::{
    CanonicalMachinePartition2, CanonicalPartitionChunk, MachinePartitionError,
    MachinePartitionPolicy2, MachinePartitionResult, package_canonical_program,
    package_canonical_scheduled_program, representative_partition_policy,
    representative_partition_policy_for,
};
pub use scene::{CurveDisplayEvidence, CurveRegionDisplayEvidence, ExactScene, SceneError};
pub use schedule_evidence::{
    CanonicalScheduleEvidence2, ScheduleEvidenceError, ScheduleEvidenceResult,
    build_canonical_schedule_evidence, replay_canonical_schedule_evidence,
    verify_canonical_schedule_evidence_bytes,
};
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
