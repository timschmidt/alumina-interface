//! Bounded browser/native presentation of the representative exact controller.
//!
//! All graph and trace authority remains in `alumina-interface-core`. This
//! module performs only deterministic layout and named, lossy display
//! projection into egui coordinates.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use alumina_board::{OwnerDomain, ResourceId, SafeValue};
use alumina_capability::{
    BoardCapabilityLimits, MAX_CAPABILITY_CHUNK_BYTES, calculate_identity, read_verified_range,
};
use alumina_diagnostics::{
    CaptureQualityFlags, DiagnosticLimits, DigitalCaptureFlags, DigitalLevel, OverviewFlags,
    ResourceValue, SampleProvenance, SampleQuality,
};
use alumina_graph_ir::{GraphIrOpcode, decode_graph_resource_pair_parameter};
use alumina_interface_core::graph::{
    CanonicalGraphAuthoringSessionEncoding, CanonicalGraphComponentEncoding,
    CanonicalGraphDeploymentReplayEvidence1, CanonicalGraphHierarchyEncoding,
    CanonicalGraphHierarchySourceMapEncoding, CanonicalGraphProbeEncoding,
    CanonicalGraphWorkspaceEncoding, CanonicalTypedGraphValueEncoding, ChannelFullPolicy,
    ClockDefinition, ClockKind, ExecutionDomain, ExecutionDomainSet, GRAPH_COMPONENT_INSTANCE_KIND,
    GRAPH_COMPONENT_INSTANCE_VERSION, GRAPH_PROBE_NAME_BYTES, GraphAnalysisLimits,
    GraphAuthoringHierarchyInput, GraphAuthoringSessionDocument, GraphAuthoringSessionHistory,
    GraphAuthoringSessionLimits, GraphAuthoringSessionReplayLimits, GraphCachedJobCatalog,
    GraphCachedJobCatalogLimits, GraphCapabilityCatalogLimits, GraphCapabilityNodeCatalog,
    GraphClockId, GraphComponentDocument, GraphComponentInput, GraphComponentInputId,
    GraphComponentInstance, GraphComponentLimits, GraphComponentOutput, GraphComponentOutputId,
    GraphDeploymentImplementation, GraphDeploymentLimits, GraphDeploymentNodeKind,
    GraphDeploymentRegistry, GraphDeploymentReplayInput, GraphDeploymentReplayLimits,
    GraphDeploymentReplayReleaseOutcome, GraphDeploymentReport, GraphDeploymentResourceSample,
    GraphDeploymentTarget, GraphDocument, GraphFlattenedNodeProvenance,
    GraphFlattenedWireProvenance, GraphFrontPanelBinding, GraphFrontPanelItem,
    GraphFrontPanelItemId, GraphFrontPanelRect, GraphHierarchyDependency, GraphHierarchyDocument,
    GraphHierarchyFlattening, GraphHierarchyLimits, GraphHierarchyNodeOrigin,
    GraphHierarchySourceMapLimits, GraphHierarchyWireOrigin, GraphInstanceScope, GraphLimits,
    GraphLiteralTextLimits, GraphNodeId, GraphNodePlacement, GraphNodePrototype, GraphNodeRegistry,
    GraphPortId, GraphProbeCapture, GraphProbeDefinition, GraphProbeDocument, GraphProbeEdge,
    GraphProbeId, GraphProbeLimits, GraphProbeProjection, GraphProbeProjectionLimits,
    GraphProbeTrigger, GraphProbeTriggerResolution, GraphSchema, GraphSimulationRegistry,
    GraphTraceEntry, GraphTypeId, GraphValue, GraphValuePathSegment, GraphWireId,
    GraphWorkspaceDocument, GraphWorkspaceLimits, InputConnectionRequirement,
    MAX_GRAPH_AUTHORING_SESSION_BYTES, MAX_GRAPH_DEPLOYMENT_REPLAY_EVIDENCE_BYTES,
    MAX_GRAPH_HIERARCHY_SOURCE_MAP_BYTES, NodeDefinition, NodeInputChannelContract,
    NodeInputChannelKind, NodeKind, NodeOutputDependency, NodeParameter, NodeParameterContract,
    NodeSchema, PortDefinition, RecordField, RecordFieldId, RecordValueField,
    RepresentativeControlSignal, RepresentativeExactControlGraph, ResourceClassId,
    ResourceGraphHandle, TypeDefinition, TypeKind, TypedGraphValue, WireDefinition, WireEndpoint,
    analyze_graph_draft, compile_representative_exact_control_graph,
    derive_graph_capability_node_catalog, encode_graph_authoring_session, encode_graph_component,
    encode_graph_hierarchy, encode_graph_hierarchy_source_map, encode_graph_probes,
    encode_graph_workspace, encode_typed_graph_value, flatten_graph_hierarchy,
    format_graph_literal_text, graph_component_instance_input_port,
    graph_component_instance_output_port, graph_component_instance_prototype, graph_resource_label,
    lower_graph_deployment, parse_graph_literal_text, project_graph_probe_replay,
    replay_graph_authoring_session, replay_graph_component, replay_graph_hierarchy_source_map,
    replay_graph_probes, replay_graph_workspace, replay_realtime_graph_deployment,
    select_graph_cached_job_handle, select_graph_capability_node_resource,
    verify_graph_deployment_evidence_bytes,
};
use alumina_interface_core::{
    BoardExplorerSnapshot, CanonicalGlobalJob2, DiagnosticExplorerSnapshot,
    build_board_explorer_snapshot, build_diagnostic_explorer_snapshot,
};
use alumina_protocol::{DeviceCycle, DeviceId, Digest};
use alumina_runtime::graph::GraphExecutionFault;
use alumina_sim::diagnostics::tinybee_diagnostic_fixture;
use eframe::egui;
use hyperreal::Rational;

use crate::cache_delivery::derive_graph_cached_job_catalog_from_ready;
use crate::m7_simulation::simulate_cache_delivery;
use crate::workspace_file::{BoundedFileBridge, BoundedFileEvent, BoundedFileSpec};

const MAXIMUM_VISIBLE_NODES: usize = 256;
const MAXIMUM_VISIBLE_WIRES: usize = 1_024;
const MAXIMUM_POINTS_PER_SERIES: usize = 4_096;
const MAXIMUM_ANALOG_TRACE_GROUPS: usize = 32;
const MAXIMUM_STATE_TRACE_LANES: usize = 64;
const MAXIMUM_STATE_IDENTITY_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_STATE_TEXT_PREVIEW_CHARS: usize = 32;
const ANALOG_TRACE_GROUP_HEIGHT: f32 = 144.0;
const STATE_TRACE_LANE_HEIGHT: f32 = 40.0;
const TRACE_SECTION_GAP: f32 = 10.0;
const NODE_WIDTH: f32 = 218.0;
const COLUMN_GAP: f32 = 82.0;
const NODE_GAP: f32 = 24.0;
const CANVAS_MARGIN: f32 = 28.0;
const NODE_HEADER_HEIGHT: f32 = 48.0;
const PORT_ROW_HEIGHT: f32 = 20.0;
const NEW_NODE_X_GAP: i32 = 300;
const NEW_NODE_ORIGIN: i32 = 28;
const EMPTY_CANVAS_WIDTH: f32 = 720.0;
const EMPTY_CANVAS_HEIGHT: f32 = 280.0;
const PERSISTED_AUTHORING_SESSION_PREFIX: &str = "algs1:";
const ALGW_FILE: BoundedFileSpec = BoundedFileSpec::new("ALGW file", "algw");
const ALGP_FILE: BoundedFileSpec = BoundedFileSpec::new("ALGP file", "algp");
const ALGC_FILE: BoundedFileSpec = BoundedFileSpec::new("selected ALGC dependency", "algc");
const ALGM_FILE: BoundedFileSpec = BoundedFileSpec::new("ALGM source map", "algm");
const ALGS_FILE: BoundedFileSpec = BoundedFileSpec::new("ALGS authoring session", "algs");
const ALGR_SUCCESS_REPLAY_FILE: BoundedFileSpec =
    BoundedFileSpec::new("success ALGRREP1 evidence", "algrrep");
const ALGR_FAULT_REPLAY_FILE: BoundedFileSpec =
    BoundedFileSpec::new("fault ALGRREP1 evidence", "algrrep");
const DIAGNOSTIC_CHANNEL_COLORS: [egui::Color32; 6] = [
    egui::Color32::from_rgb(96, 169, 232),
    egui::Color32::from_rgb(241, 178, 84),
    egui::Color32::from_rgb(123, 214, 149),
    egui::Color32::from_rgb(209, 158, 255),
    egui::Color32::from_rgb(238, 126, 161),
    egui::Color32::from_rgb(101, 205, 196),
];

#[cfg(target_arch = "wasm32")]
pub(crate) const AUTHORING_SESSION_STORAGE_KEY: &str = "alumina.graph-authoring-session.algs.v1";
const SIGNALS: [RepresentativeControlSignal; 7] = [
    RepresentativeControlSignal::Error,
    RepresentativeControlSignal::IntegralPrior,
    RepresentativeControlSignal::ClampedController,
    RepresentativeControlSignal::PermittedOutput,
    RepresentativeControlSignal::MeasurementWithinRange,
    RepresentativeControlSignal::CombinedPermit,
    RepresentativeControlSignal::ExternalPermit,
];
const CACHED_JOB_HANDLE_TYPE: GraphTypeId = GraphTypeId::new(1);
const CACHED_JOB_OPTION_TYPE: GraphTypeId = GraphTypeId::new(2);
const CACHED_JOB_ARRAY_TYPE: GraphTypeId = GraphTypeId::new(3);
const CACHED_JOB_REFERENCE_SET_TYPE: GraphTypeId = GraphTypeId::new(4);
const CACHED_JOB_PARAMETER: u32 = 1;
const CACHED_JOB_KIND_NAME: &str = "alumina.job.cached-reference-set";
const CACHED_JOB_PRIMARY_FIELD: RecordFieldId = RecordFieldId::new(1);
const CACHED_JOB_FALLBACK_FIELD: RecordFieldId = RecordFieldId::new(2);
const CACHED_JOB_MIRRORS_FIELD: RecordFieldId = RecordFieldId::new(3);
const CACHED_JOB_PRIMARY_PATH: [GraphValuePathSegment; 1] =
    [GraphValuePathSegment::RecordField(CACHED_JOB_PRIMARY_FIELD)];
const CACHED_JOB_FALLBACK_PATH: [GraphValuePathSegment; 2] = [
    GraphValuePathSegment::RecordField(CACHED_JOB_FALLBACK_FIELD),
    GraphValuePathSegment::OptionSome,
];
const CACHED_JOB_MIRROR_PATH: [GraphValuePathSegment; 2] = [
    GraphValuePathSegment::RecordField(CACHED_JOB_MIRRORS_FIELD),
    GraphValuePathSegment::ArrayIndex(0),
];
const CACHED_JOB_REFERENCE_SLOTS: [CachedJobReferenceSlot; 3] = [
    CachedJobReferenceSlot {
        label: "primary",
        path: &CACHED_JOB_PRIMARY_PATH,
    },
    CachedJobReferenceSlot {
        label: "fallback.some",
        path: &CACHED_JOB_FALLBACK_PATH,
    },
    CachedJobReferenceSlot {
        label: "mirrors[0]",
        path: &CACHED_JOB_MIRROR_PATH,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CachedJobReferenceSlot {
    label: &'static str,
    path: &'static [GraphValuePathSegment],
}

const TARGET_RESOURCE_BOOL_TYPE: GraphTypeId = GraphTypeId::new(1);
const TARGET_RESOURCE_STREAM_TYPE: GraphTypeId = GraphTypeId::new(2);
const TARGET_RESOURCE_HANDLE_TYPE: GraphTypeId = GraphTypeId::new(3);
const TARGET_RESOURCE_PAIR_TYPE: GraphTypeId = GraphTypeId::new(4);
const TARGET_RESOURCE_PARAMETER: u32 = 1;
const TARGET_RESOURCE_SCALAR_KIND_NAME: &str = "alumina.io.stable-boolean-input";
const TARGET_RESOURCE_PAIR_KIND_NAME: &str = "alumina.io.stable-boolean-pair-all";
const TARGET_RESOURCE_SINK_KIND_NAME: &str = "alumina.io.boolean-stream-sink";
const TARGET_RESOURCE_FIRST_FIELD: RecordFieldId = RecordFieldId::new(1);
const TARGET_RESOURCE_SECOND_FIELD: RecordFieldId = RecordFieldId::new(2);
const TARGET_RESOURCE_FIRST_PATH: [GraphValuePathSegment; 1] =
    [GraphValuePathSegment::RecordField(
        TARGET_RESOURCE_FIRST_FIELD,
    )];
const TARGET_RESOURCE_SECOND_PATH: [GraphValuePathSegment; 1] =
    [GraphValuePathSegment::RecordField(
        TARGET_RESOURCE_SECOND_FIELD,
    )];
const TARGET_RESOURCE_REFERENCE_SLOTS: [TargetResourceReferenceSlot; 2] = [
    TargetResourceReferenceSlot {
        label: "permit",
        path: &TARGET_RESOURCE_FIRST_PATH,
    },
    TargetResourceReferenceSlot {
        label: "interlock",
        path: &TARGET_RESOURCE_SECOND_PATH,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TargetResourceReferenceSlot {
    label: &'static str,
    path: &'static [GraphValuePathSegment],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WirePresentation {
    feedback_lane: Option<usize>,
}

#[derive(Clone, Debug)]
struct NodePresentation {
    rect: egui::Rect,
    rank: usize,
}

#[derive(Clone, Debug)]
struct GraphPresentation {
    nodes: BTreeMap<GraphNodeId, NodePresentation>,
    wires: BTreeMap<GraphWireId, WirePresentation>,
    size: egui::Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeDrag {
    node: GraphNodeId,
    origin: GraphNodePlacement,
    delta: egui::Vec2,
}

#[derive(Clone, Debug)]
struct ComponentDefinitionEditor {
    scope: Option<Digest>,
    palette_index: usize,
    child_component: Option<Digest>,
    selected_node: Option<GraphNodeId>,
    pending_source: Option<WireEndpoint>,
    drag: Option<NodeDrag>,
    parameter_drafts: BTreeMap<(GraphNodeId, u32), String>,
    node_label_drafts: BTreeMap<GraphNodeId, String>,
    status: String,
}

impl Default for ComponentDefinitionEditor {
    fn default() -> Self {
        Self {
            scope: None,
            palette_index: 0,
            child_component: None,
            selected_node: None,
            pending_source: None,
            drag: None,
            parameter_drafts: BTreeMap::new(),
            node_label_drafts: BTreeMap::new(),
            status:
                "select a non-authoritative ALGH library dependency to edit its exact definition"
                    .to_owned(),
        }
    }
}

impl ComponentDefinitionEditor {
    fn reset_scope(&mut self, scope: Option<Digest>, status: String) {
        self.scope = scope;
        self.child_component = None;
        self.selected_node = None;
        self.pending_source = None;
        self.drag = None;
        self.parameter_drafts.clear();
        self.node_label_drafts.clear();
        self.status = status;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum HierarchySourceSelection {
    Node {
        flattened: GraphNodeId,
        origin: GraphHierarchyNodeOrigin,
    },
    Wire {
        flattened: GraphWireId,
        origin: GraphHierarchyWireOrigin,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HierarchySourceDestination {
    Root,
    Component(Digest),
}

impl HierarchySourceSelection {
    fn destination(&self) -> HierarchySourceDestination {
        match self {
            Self::Node {
                origin: GraphHierarchyNodeOrigin::Root(_),
                ..
            }
            | Self::Wire {
                origin: GraphHierarchyWireOrigin::Root(_),
                ..
            } => HierarchySourceDestination::Root,
            Self::Node {
                origin: GraphHierarchyNodeOrigin::Component { component, .. },
                ..
            }
            | Self::Wire {
                origin: GraphHierarchyWireOrigin::Component { component, .. },
                ..
            } => HierarchySourceDestination::Component(*component),
        }
    }
}

#[derive(Clone, Debug)]
struct HierarchySourceBrowser {
    selected_node: Option<GraphNodeId>,
    selected_wire: Option<GraphWireId>,
    last_opened: Option<HierarchySourceSelection>,
    scroll_pending: bool,
    status: String,
}

impl Default for HierarchySourceBrowser {
    fn default() -> Self {
        Self {
            selected_node: None,
            selected_wire: None,
            last_opened: None,
            scroll_pending: false,
            status: "choose one final flattened node or wire to open its exact ALGM source"
                .to_owned(),
        }
    }
}

impl HierarchySourceBrowser {
    fn reset(&mut self, status: impl Into<String>) {
        self.selected_node = None;
        self.selected_wire = None;
        self.last_opened = None;
        self.scroll_pending = false;
        self.status = status.into();
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct HierarchyRootCanvasFocus {
    selected_instance: Option<GraphNodeId>,
    opened_source_node: Option<GraphNodeId>,
    opened_source_wire: Option<GraphWireId>,
}

impl HierarchyRootCanvasFocus {
    fn selects_node(self, node: GraphNodeId) -> bool {
        self.selected_instance == Some(node) || self.opened_source_node == Some(node)
    }

    fn selects_wire(self, wire: &WireDefinition) -> bool {
        self.opened_source_wire == Some(wire.id())
            || self
                .selected_instance
                .is_some_and(|node| node == wire.source().node || node == wire.target().node)
            || self
                .opened_source_node
                .is_some_and(|node| node == wire.source().node || node == wire.target().node)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PanelItemDrag {
    item: GraphFrontPanelItemId,
    origin: GraphFrontPanelRect,
    delta: egui::Vec2,
}

#[derive(Clone, Debug)]
struct NodePaletteEntry {
    display_name: String,
    prototype: GraphNodePrototype,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortEdit {
    SelectOutput(WireEndpoint),
    ConnectInput(WireEndpoint),
    DisconnectInput(WireEndpoint),
}

#[derive(Clone, Debug)]
enum TracePointValue {
    Analog {
        exact: String,
        enclosure: [f64; 2],
    },
    Boolean(bool),
    State {
        summary: String,
        encoding: CanonicalTypedGraphValueEncoding,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TraceSeriesKind {
    Analog,
    Boolean,
    State,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TraceSignal {
    probe: GraphProbeId,
    name: String,
    source: WireEndpoint,
    representative: Option<RepresentativeControlSignal>,
    sample_type: GraphTypeId,
    sample_type_name: String,
    unit_symbol: Option<String>,
}

impl TraceSignal {
    fn label(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug)]
struct TracePoint {
    clock: GraphClockId,
    tick: u64,
    sequence: u64,
    root_tick: Rational,
    value: TracePointValue,
}

#[derive(Clone, Debug)]
struct TraceSeries {
    signal: TraceSignal,
    kind: TraceSeriesKind,
    points: Vec<TracePoint>,
}

#[derive(Debug)]
struct AnalogTraceGroup<'a> {
    sample_type: GraphTypeId,
    sample_type_name: &'a str,
    unit_symbol: &'a str,
    series: Vec<&'a TraceSeries>,
}

impl AnalogTraceGroup<'_> {
    fn label(&self) -> String {
        let unit = if self.unit_symbol.is_empty() {
            "unitless"
        } else {
            self.unit_symbol
        };
        format!(
            "{unit}\n{} [t{}]",
            self.sample_type_name,
            self.sample_type.get()
        )
    }
}

#[derive(Clone, Debug)]
struct TraceProjection {
    exact: GraphProbeProjection,
    traces: Vec<TraceSeries>,
    source_clocks: BTreeSet<GraphClockId>,
    time_axis: Option<TraceTimeAxis>,
}

#[derive(Clone, Debug)]
struct TraceTimeAxis {
    minimum: Rational,
    maximum: Rational,
    selectable_ticks: Vec<Rational>,
    selectable_fractions: Vec<f64>,
    divisions: usize,
}

impl TraceTimeAxis {
    fn try_new(
        traces: &[TraceSeries],
        trigger_window: Option<(&Rational, &Rational, &Rational)>,
    ) -> Result<Option<Self>, String> {
        let mut selectable_ticks = traces
            .iter()
            .flat_map(|series| series.points.iter().map(|point| point.root_tick.clone()))
            .collect::<Vec<_>>();
        if selectable_ticks.is_empty() {
            return Ok(None);
        }
        if let Some((first, trigger, last)) = trigger_window {
            selectable_ticks.extend([first.clone(), trigger.clone(), last.clone()]);
        }
        selectable_ticks
            .sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        if selectable_ticks.windows(2).any(|pair| {
            !matches!(
                pair[0].partial_cmp(&pair[1]),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            )
        }) {
            return Err("exact trace-time values are not totally ordered".to_owned());
        }
        selectable_ticks.dedup();
        let minimum = selectable_ticks
            .first()
            .cloned()
            .ok_or_else(|| "exact trace-time axis has no minimum".to_owned())?;
        let maximum = selectable_ticks
            .last()
            .cloned()
            .ok_or_else(|| "exact trace-time axis has no maximum".to_owned())?;
        let mut selectable_fractions = selectable_ticks
            .iter()
            .map(|tick| exact_time_fraction(tick, &minimum, &maximum))
            .collect::<Result<Vec<_>, _>>()?;
        let mut preceding = 0.0_f64;
        for fraction in &mut selectable_fractions {
            *fraction = (*fraction).max(preceding);
            preceding = *fraction;
        }
        let divisions = selectable_ticks.len().saturating_sub(1).clamp(1, 5);
        Ok(Some(Self {
            minimum,
            maximum,
            selectable_ticks,
            selectable_fractions,
            divisions,
        }))
    }

    fn clamp(&self, tick: &Rational) -> Rational {
        if tick < &self.minimum {
            self.minimum.clone()
        } else if tick > &self.maximum {
            self.maximum.clone()
        } else {
            tick.clone()
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "exact root time is projected one way into bounded egui f32 coordinates"
    )]
    fn plot_x(&self, rect: egui::Rect, tick: &Rational) -> Option<f32> {
        let retained = self
            .selectable_ticks
            .binary_search_by(|candidate| {
                candidate
                    .partial_cmp(tick)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .ok()
            .and_then(|index| self.selectable_fractions.get(index).copied());
        retained
            .or_else(|| exact_time_fraction(tick, &self.minimum, &self.maximum).ok())
            .map(|fraction| rect.left() + rect.width() * fraction as f32)
    }

    fn nearest_tick(&self, rect: egui::Rect, x: f32) -> Option<Rational> {
        let width = rect.width();
        if !width.is_finite() || width <= 0.0 {
            return self.selectable_ticks.first().cloned();
        }
        let target = f64::from(((x - rect.left()) / width).clamp(0.0, 1.0));
        let after = self
            .selectable_fractions
            .partition_point(|fraction| *fraction < target);
        let selected = match after {
            0 => 0,
            index if index == self.selectable_fractions.len() => index - 1,
            index => {
                let before_distance = (target - self.selectable_fractions[index - 1]).abs();
                let after_distance = (self.selectable_fractions[index] - target).abs();
                if before_distance <= after_distance {
                    index - 1
                } else {
                    index
                }
            }
        };
        self.selectable_ticks.get(selected).cloned()
    }

    fn grid_ticks(&self) -> Vec<Rational> {
        if self.minimum == self.maximum {
            return vec![self.minimum.clone()];
        }
        let span = self.maximum.clone() - self.minimum.clone();
        let denominator = Rational::from(
            u64::try_from(self.divisions).expect("trace grid divisions are bounded to five"),
        );
        (0..=self.divisions)
            .map(|index| {
                let numerator = Rational::from(
                    u64::try_from(index).expect("trace grid index is bounded to five"),
                );
                self.minimum.clone() + span.clone() * numerator / denominator.clone()
            })
            .collect()
    }
}

fn exact_time_fraction(
    tick: &Rational,
    minimum: &Rational,
    maximum: &Rational,
) -> Result<f64, String> {
    if minimum == maximum {
        return Ok(0.5);
    }
    let ratio = (tick.clone() - minimum.clone()) / (maximum.clone() - minimum.clone());
    let enclosure = ratio
        .to_f64_enclosure()
        .filter(|bounds| bounds.iter().all(|bound| bound.is_finite()))
        .ok_or_else(|| "exact root-time fraction has no finite display enclosure".to_owned())?;
    Ok((enclosure[0] + (enclosure[1] - enclosure[0]) * 0.5).clamp(0.0, 1.0))
}

#[derive(Clone, Debug)]
struct ComponentPackage {
    document: GraphComponentDocument,
    encoding: CanonicalGraphComponentEncoding,
    hierarchy: HierarchyPackage,
}

#[derive(Clone, Debug)]
struct HierarchyPackage {
    document: GraphHierarchyDocument,
    encoding: CanonicalGraphHierarchyEncoding,
    flattening: GraphHierarchyFlattening,
    source_map: CanonicalGraphHierarchySourceMapEncoding,
}

fn hierarchy_depth(flattening: &GraphHierarchyFlattening) -> usize {
    flattening
        .instances()
        .iter()
        .map(|instance| instance.source_path().len())
        .max()
        .unwrap_or(0)
}

fn hierarchy_source_path_label(path: &[GraphNodeId]) -> String {
    let mut label = String::from("[");
    for (index, node) in path.iter().enumerate() {
        if index != 0 {
            label.push('/');
        }
        label.push_str(&node.get().to_string());
    }
    label.push(']');
    label
}

fn hierarchy_node_origin_label(origin: &GraphHierarchyNodeOrigin) -> String {
    match origin {
        GraphHierarchyNodeOrigin::Root(node) => format!("root ALGW node {}", node.get()),
        GraphHierarchyNodeOrigin::Component {
            source_path,
            component,
            node,
        } => format!(
            "{} · ALGC {}… node {}",
            hierarchy_source_path_label(source_path),
            digest_prefix(component.0),
            node.get()
        ),
    }
}

fn hierarchy_wire_origin_label(origin: &GraphHierarchyWireOrigin) -> String {
    match origin {
        GraphHierarchyWireOrigin::Root(wire) => format!("root ALGW wire {}", wire.get()),
        GraphHierarchyWireOrigin::Component {
            source_path,
            component,
            wire,
        } => format!(
            "{} · ALGC {}… wire {}",
            hierarchy_source_path_label(source_path),
            digest_prefix(component.0),
            wire.get()
        ),
    }
}

fn hierarchy_origin_correlation(
    component: &ComponentPackage,
    node: GraphNodeId,
    port: Option<GraphPortId>,
) -> Option<String> {
    const MAXIMUM_VISIBLE_OCCURRENCES: usize = 4;

    let mut total = 0_usize;
    let mut visible = Vec::new();
    for mapping in component.hierarchy.flattening.node_provenance() {
        let GraphHierarchyNodeOrigin::Component {
            source_path,
            component: origin_component,
            node: origin_node,
        } = mapping.origin()
        else {
            continue;
        };
        if *origin_component != component.encoding.digest() || *origin_node != node {
            continue;
        }
        total += 1;
        if visible.len() < MAXIMUM_VISIBLE_OCCURRENCES {
            let endpoint_suffix = port.map_or_else(String::new, |port| format!(".p{}", port.get()));
            visible.push(format!(
                "{}:n{}{} → flat n{}{}",
                hierarchy_source_path_label(source_path),
                node.get(),
                endpoint_suffix,
                mapping.flattened_node().get(),
                endpoint_suffix,
            ));
        }
    }
    if total == 0 {
        return None;
    }
    let mut label = visible.join(" · ");
    if total > visible.len() {
        let _ = write!(label, " · +{} occurrences", total - visible.len());
    }
    Some(label)
}

fn hierarchy_node_correlation(component: &ComponentPackage, node: GraphNodeId) -> Option<String> {
    hierarchy_origin_correlation(component, node, None)
}

fn hierarchy_endpoint_correlation(
    component: &ComponentPackage,
    endpoint: WireEndpoint,
) -> Option<String> {
    hierarchy_origin_correlation(component, endpoint.node, Some(endpoint.port))
}

#[derive(Clone, Debug)]
struct ProbePackage {
    document: GraphProbeDocument,
    encoding: CanonicalGraphProbeEncoding,
}

struct PreparedAuthoringSession {
    workspace: GraphWorkspaceDocument,
    workspace_encoding: CanonicalGraphWorkspaceEncoding,
    presentation: GraphPresentation,
    component: Option<ComponentPackage>,
    probes: ProbePackage,
    cached_job_workspace: GraphWorkspaceDocument,
    cached_job_encoding: CanonicalGraphWorkspaceEncoding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProbeEditDraft {
    name: String,
    maximum_samples: u32,
    sample_stride: u32,
}

impl ProbeEditDraft {
    fn from_probe(probe: &GraphProbeDefinition) -> Self {
        Self {
            name: probe.name().to_owned(),
            maximum_samples: probe.capture().maximum_samples(),
            sample_stride: probe.capture().sample_stride(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProbeUiAction {
    Remove(GraphProbeId),
    SetTrigger(GraphProbeId, GraphProbeEdge),
    ApplyMetadata(GraphProbeId, ProbeEditDraft),
    ResetMetadata(GraphProbeId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HierarchyUiAction {
    CreateComponent,
    AddRoot(Digest),
    RemoveRoot(GraphNodeId),
    RemoveComponent(Digest),
    MoveRoot {
        node: GraphNodeId,
        x: i32,
        y: i32,
    },
    ConnectRoot {
        source: WireEndpoint,
        target: WireEndpoint,
    },
    DisconnectRoot(GraphWireId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PanelItemDraft {
    name: String,
    binding: GraphFrontPanelBinding,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl PanelItemDraft {
    fn from_item(item: &GraphFrontPanelItem) -> Self {
        let rect = item.rect();
        Self {
            name: item.name().to_owned(),
            binding: item.binding(),
            x: rect.x(),
            y: rect.y(),
            width: rect.width(),
            height: rect.height(),
        }
    }

    const fn rect(&self) -> GraphFrontPanelRect {
        GraphFrontPanelRect::new(self.x, self.y, self.width, self.height)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PanelUiAction {
    Add {
        name: String,
        binding: GraphFrontPanelBinding,
        rect: GraphFrontPanelRect,
    },
    Update {
        item: GraphFrontPanelItemId,
        draft: PanelItemDraft,
    },
    Remove(GraphFrontPanelItemId),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ComponentConnectorSelection {
    Input(GraphComponentInputId),
    Output(GraphComponentOutputId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ComponentConnectorDraft {
    name: String,
    endpoint: WireEndpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ComponentConnectorUiAction {
    AddInput {
        name: String,
        target: WireEndpoint,
    },
    UpdateInput {
        input: GraphComponentInputId,
        draft: ComponentConnectorDraft,
    },
    RemoveInput(GraphComponentInputId),
    AddOutput {
        name: String,
        source: WireEndpoint,
    },
    UpdateOutput {
        output: GraphComponentOutputId,
        draft: ComponentConnectorDraft,
    },
    RemoveOutput(GraphComponentOutputId),
}

#[derive(Clone, Debug)]
struct TargetResourceProof {
    catalog: GraphCapabilityNodeCatalog,
    registry: GraphDeploymentRegistry,
    workspace: GraphWorkspaceDocument,
    encoding: CanonicalGraphWorkspaceEncoding,
    deployment: TargetResourceDeployment,
    catalog_index: usize,
    pair_node: GraphNodeId,
    reference_slot: usize,
    status: String,
}

#[derive(Clone, Debug, PartialEq)]
struct TargetResourceDeployment {
    report: GraphDeploymentReport,
    package_digest: Digest,
    implementation_digest: Digest,
    package_bytes: usize,
    first: ResourceId,
    second: ResourceId,
    realtime_period_cycles: u64,
    realtime_wcet_cycles: u64,
    actor_replay: TargetResourceActorReplay,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetResourceActorReplay {
    success_evidence: CanonicalGraphDeploymentReplayEvidence1,
    fault_evidence: CanonicalGraphDeploymentReplayEvidence1,
    truth_table: [bool; 4],
    ordered_reads: [ResourceId; 2],
    fault_reads: [ResourceId; 2],
    terminal_fault: GraphExecutionFault,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoardResourceFilter {
    All,
    DiagnosticObservable,
    DiagnosticClosed,
    DigitallyCapturable,
    CaptureClosed,
    GraphReadable,
    GraphClosed,
    Hazardous,
    Service,
    Realtime,
}

impl BoardResourceFilter {
    const ALL: [Self; 10] = [
        Self::All,
        Self::DiagnosticObservable,
        Self::DiagnosticClosed,
        Self::DigitallyCapturable,
        Self::CaptureClosed,
        Self::GraphReadable,
        Self::GraphClosed,
        Self::Hazardous,
        Self::Service,
        Self::Realtime,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::All => "all described",
            Self::DiagnosticObservable => "passively observable",
            Self::DiagnosticClosed => "diagnostic-closed",
            Self::DigitallyCapturable => "digitally capturable",
            Self::CaptureClosed => "capture-closed",
            Self::GraphReadable => "graph-readable",
            Self::GraphClosed => "graph-closed",
            Self::Hazardous => "hazardous",
            Self::Service => "Service-owned",
            Self::Realtime => "Realtime-owned",
        }
    }
}

#[derive(Clone, Debug)]
struct BoardExplorerPanel {
    snapshot: BoardExplorerSnapshot,
    diagnostics: DiagnosticExplorerSnapshot,
    filter: BoardResourceFilter,
    search: String,
    selected: Option<ResourceId>,
    diagnostic_cursor_offset: u64,
}

#[derive(Clone, Debug)]
struct ComponentPanelItem {
    id: GraphFrontPanelItemId,
    name: String,
    binding: GraphFrontPanelBinding,
    rect: GraphFrontPanelRect,
    value_type: GraphTypeId,
    output_text: Option<String>,
}

impl BoardExplorerPanel {
    fn show(&mut self, ui: &mut egui::Ui) {
        self.show_summary(ui);
        self.show_filters(ui);
        self.show_resource_ledger(ui);
        self.show_selected_resource(ui);
        self.show_diagnostics(ui);
        self.show_visual_authority(ui);
        self.show_supporting_section_counts(ui);
    }

    fn show_summary(&self, ui: &mut egui::Ui) {
        let summary = self.snapshot.resource_summary();
        let (flash, internal_sram, psram) = self.snapshot.memory_bytes();
        let (service_core, realtime_core) = self.snapshot.core_assignment();
        ui.horizontal_wrapped(|ui| {
            ui.heading("TinyBee board explorer");
            ui.monospace(format!(
                "capability {}… · {} bytes",
                digest_prefix(self.snapshot.identity().digest.0),
                self.snapshot.identity().byte_len
            ));
            ui.colored_label(
                egui::Color32::YELLOW,
                if self.snapshot.armable() {
                    "package claims armable"
                } else {
                    "non-armable package"
                },
            );
        });
        ui.label(format!(
            "{} · {} · {:?} / {:?} · {} application cores",
            self.snapshot.board_id(),
            self.snapshot.revision(),
            self.snapshot.chip(),
            self.snapshot.qualification(),
            self.snapshot.application_cores()
        ));
        ui.horizontal_wrapped(|ui| {
            ui.monospace(format!(
                "flash {} · internal SRAM {} · PSRAM {}",
                display_bytes(flash),
                display_bytes(internal_sram),
                display_bytes(psram)
            ));
            ui.monospace(format!(
                "Service core {service_core} · Realtime core {realtime_core}"
            ));
        });
        ui.colored_label(
            egui::Color32::LIGHT_BLUE,
            "Offline exact capability snapshot only — no board connection, live value, allocation, command, or output authority.",
        );
        ui.label(
            "Descriptive resources state what is routed and its safe/hazard ownership. Passive observations, digital acquisition, and graph operations are separately published authorities; every other operation remains closed.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("{} resources", self.snapshot.resources().len()));
            ui.label(format!("{} Service", summary.service));
            ui.label(format!("{} Realtime", summary.realtime));
            ui.colored_label(
                egui::Color32::from_rgb(241, 104, 104),
                format!("{} hazardous", summary.hazardous),
            );
            ui.colored_label(
                egui::Color32::from_rgb(103, 193, 232),
                format!("{} passively observable", summary.diagnostic_observable),
            );
            ui.colored_label(
                egui::Color32::from_rgb(177, 137, 242),
                format!("{} digitally capturable", summary.digitally_capturable),
            );
            ui.colored_label(
                egui::Color32::from_rgb(123, 214, 149),
                format!("{} graph-readable", summary.graph_addressable),
            );
            ui.label(format!("{} aliases", self.snapshot.alias_count()));
        });
        let diagnostics = self.snapshot.diagnostic_overview();
        if diagnostics.is_implemented() {
            ui.label(format!(
                "Passive overview V{} · {} / {} resources · {} / {} B request/event · {} µs cadence · {} µs freshness ceiling",
                diagnostics.schema_version,
                diagnostics.resource_count,
                diagnostics.maximum_resources,
                diagnostics.telemetry_request_bytes,
                diagnostics.telemetry_event_bytes,
                diagnostics.nominal_period_micros,
                diagnostics.maximum_age_micros,
            ));
        } else {
            ui.weak("This image composes no passive diagnostic-overview provider.");
        }
        let capture = self.snapshot.digital_capture();
        if capture.is_implemented() {
            ui.label(format!(
                "Digital capture V{} · {} / {} channels · {} transitions · {} / {} / {} B configure/record/chunk · {} / {} / {} µs pretrigger/duration/arm horizon",
                capture.schema_version,
                capture.resource_count,
                capture.maximum_channels,
                capture.maximum_transitions,
                capture.configure_bytes,
                capture.record_bytes,
                capture.maximum_chunk_bytes,
                capture.maximum_pretrigger_micros,
                capture.maximum_duration_micros,
                capture.arm_horizon_micros,
            ));
        } else {
            ui.weak("This image composes no device-produced digital-capture provider.");
        }
    }

    fn show_filters(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("tinybee_board_resource_filter")
                .selected_text(self.filter.label())
                .show_ui(ui, |ui| {
                    for filter in BoardResourceFilter::ALL {
                        ui.selectable_value(&mut self.filter, filter, filter.label());
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("resource or alias")
                    .desired_width(220.0),
            );
            if !self.search.is_empty() && ui.small_button("clear search").clicked() {
                self.search.clear();
            }
        });
    }

    fn show_resource_ledger(&mut self, ui: &mut egui::Ui) {
        let search = self.search.trim().to_ascii_lowercase();
        let matching = self
            .snapshot
            .resources()
            .iter()
            .filter(|resource| resource_matches_filter(resource, self.filter))
            .filter(|resource| resource_matches_search(resource, &search))
            .map(|resource| resource.descriptor().id)
            .collect::<Vec<_>>();
        let visible_count = matching.len();
        egui::ScrollArea::vertical()
            .id_salt("tinybee_board_resource_ledger")
            .max_height(285.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for id in matching {
                    let resource = self
                        .snapshot
                        .resource(id)
                        .expect("filtered resource remains in immutable snapshot");
                    let descriptor = resource.descriptor();
                    let aliases = if resource.aliases().is_empty() {
                        "no aliases".to_owned()
                    } else {
                        resource.aliases().join(", ")
                    };
                    let label = graph_resource_label(id);
                    let diagnostic_observable = resource.is_diagnostic_observable();
                    let digital_capture = resource.digital_capture();
                    let graph_addressable = resource.is_graph_addressable();
                    let selected = self.selected == Some(id);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .selectable_label(selected, format!("{label} · {aliases}"))
                            .clicked()
                        {
                            self.selected = Some(id);
                        }
                        ui.weak(owner_label(descriptor.owner));
                        ui.weak(format!("safe {}", safe_value_label(descriptor.safe_value)));
                        if descriptor.hazardous_output {
                            ui.colored_label(
                                egui::Color32::from_rgb(241, 104, 104),
                                "hazardous output",
                            );
                        }
                        ui.colored_label(
                            if digital_capture.is_some() {
                                egui::Color32::from_rgb(177, 137, 242)
                            } else {
                                egui::Color32::GRAY
                            },
                            digital_capture.map_or_else(
                                || "no digital acquisition".to_owned(),
                                |capture| format!("{:?} digital acquisition", capture.source),
                            ),
                        );
                        ui.colored_label(
                            if diagnostic_observable {
                                egui::Color32::from_rgb(103, 193, 232)
                            } else {
                                egui::Color32::GRAY
                            },
                            if diagnostic_observable {
                                "passive semantic observation"
                            } else {
                                "no passive observation"
                            },
                        );
                        ui.colored_label(
                            if graph_addressable {
                                egui::Color32::from_rgb(123, 214, 149)
                            } else {
                                egui::Color32::GRAY
                            },
                            if graph_addressable {
                                "stable Boolean graph read"
                            } else {
                                "no graph operation"
                            },
                        );
                        if let Some(sample) = self.diagnostics.overview_sample(id) {
                            ui.colored_label(
                                overview_quality_color(sample.quality),
                                format!(
                                    "{} {} · {:?} · age {} cycles",
                                    provenance_short(sample.provenance),
                                    resource_value_text(sample.value),
                                    sample.quality,
                                    self.diagnostics.overview_age_cycles(id).unwrap_or(0)
                                ),
                            );
                        }
                        if self.diagnostics.capture_channel_index(id).is_some() {
                            ui.colored_label(egui::Color32::from_rgb(103, 193, 232), "captured");
                        }
                    });
                }
            });
        ui.weak(format!("{visible_count} resources match the current view"));
    }

    fn show_selected_resource(&self, ui: &mut egui::Ui) {
        if let Some(selected) = self.selected
            && let Some(resource) = self.snapshot.resource(selected)
        {
            let descriptor = resource.descriptor();
            ui.group(|ui| {
                ui.strong(format!("Selected {}", graph_resource_label(selected)));
                ui.monospace(format!("typed selector {selected:?}"));
                ui.label(format!(
                    "owner {} · safe {} · {}",
                    owner_label(descriptor.owner),
                    safe_value_label(descriptor.safe_value),
                    if descriptor.hazardous_output {
                        "hazardous-output policy applies"
                    } else {
                        "not marked as a hazardous output"
                    }
                ));
                ui.label(format!(
                    "aliases: {}",
                    if resource.aliases().is_empty() {
                        "none".to_owned()
                    } else {
                        resource.aliases().join(", ")
                    }
                ));
                if resource.graph_accesses().is_empty() {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "No graph read, write, sample, or schedule operation is published.",
                    );
                } else {
                    for access in resource.graph_accesses() {
                        ui.colored_label(
                            egui::Color32::from_rgb(123, 214, 149),
                            format!(
                                "graph class {} · {:?} · {:?}",
                                access.class.get(),
                                access.access,
                                access.support
                            ),
                        );
                    }
                }
                if resource.diagnostic_observations().is_empty() {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "No passive diagnostic observation is published for this resource.",
                    );
                } else {
                    for observation in resource.diagnostic_observations() {
                        ui.colored_label(
                            egui::Color32::from_rgb(103, 193, 232),
                            format!(
                                "passive observation {:?} · {:?}",
                                observation.observation, observation.support
                            ),
                        );
                    }
                }
                if let Some(sample) = self.diagnostics.overview_sample(selected) {
                    ui.separator();
                    ui.colored_label(
                        overview_quality_color(sample.quality),
                        format!(
                            "Overview: {} {} · {:?} · {:?} · captured cycle {} · age {} cycles",
                            provenance_short(sample.provenance),
                            resource_value_text(sample.value),
                            sample.quality,
                            sample.quality_flags,
                            sample.captured_cycle.0,
                            self.diagnostics
                                .overview_age_cycles(selected)
                                .expect("present overview sample has a canonical age")
                        ),
                    );
                } else {
                    ui.weak("No value for this resource is present in the explicit overview record.");
                }
                if let Some(channel) = self.diagnostics.capture_channel_index(selected) {
                    ui.colored_label(
                        egui::Color32::from_rgb(103, 193, 232),
                        format!("Digital capture channel {channel}; selecting the trace and ledger refers to the same typed resource."),
                    );
                } else {
                    ui.weak("This resource is not one of the explicit digital-capture channels.");
                }
            });
        }
    }

    fn show_diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            ui.heading("Resource overview and digital capture");
            ui.monospace(format!(
                "ALMOVW01 {} B · ALMDIG01 {} B",
                self.diagnostics.overview_bytes(),
                self.diagnostics.capture_bytes()
            ));
        });
        let context = self.diagnostics.context();
        let simulated = self
            .diagnostics
            .overview_flags()
            .contains(OverviewFlags::SIMULATED)
            && self
                .diagnostics
                .capture_identity()
                .0
                .contains(DigitalCaptureFlags::SIMULATED);
        ui.colored_label(
            egui::Color32::YELLOW,
            if simulated {
                "DETERMINISTIC SIMULATION — decoded evidence only; no TinyBee connection, measurement, diagnostic lease, command, or output authority."
            } else {
                "Decoded acquisition evidence; connection and authority remain separate state."
            },
        );
        ui.horizontal_wrapped(|ui| {
            ui.monospace(format!(
                "device {} · boot {}… · config {}…",
                printable_identity(&context.device_id.0),
                byte_prefix(&context.boot_id.as_bytes(), 4),
                digest_prefix(context.config_digest.0)
            ));
            ui.monospace(format!(
                "clock {} Hz · capability {}… / {} B",
                context.clock_frequency_hz,
                digest_prefix(context.capability.digest.0),
                context.capability.byte_len
            ));
        });

        let (snapshot_cycle, sequence) = self.diagnostics.overview_position();
        let (start_cycle, end_cycle) = self.diagnostics.capture_window();
        let (requested_pre, requested_post) = self.diagnostics.requested_capture_window();
        let (trigger_cycle, trigger_channel, trigger_condition, trigger_transition) =
            self.diagnostics.trigger();
        let (capacity, stride) = self.diagnostics.capture_retention();
        let trigger_resource = self
            .diagnostics
            .capture_channels()
            .get(usize::from(trigger_channel))
            .map_or_else(
                || "immediate/no channel".to_owned(),
                |channel| graph_resource_label(channel.resource),
            );
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "Overview sequence {sequence} at cycle {} · {} explicit values",
                snapshot_cycle.0,
                self.diagnostics.overview_samples().len()
            ));
            ui.label(format!(
                "Capture {:?} · [{}..{}) · {} transitions / {} capacity · stride {}",
                self.diagnostics.capture_state(),
                start_cycle.0,
                end_cycle.0,
                self.diagnostics.transitions().len(),
                capacity,
                stride
            ));
        });
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(241, 104, 104),
                format!(
                    "trigger {:?} on {trigger_resource} at cycle {} / transition {}",
                    trigger_condition, trigger_cycle.0, trigger_transition
                ),
            );
            ui.label(format!(
                "requested pre/post {requested_pre}/{requested_post} cycles"
            ));
            ui.label(capture_quality_text(
                self.diagnostics.capture_quality_flags(),
            ));
        });

        self.show_digital_capture_plot(ui);
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::too_many_lines,
        reason = "this is a bounded, lossy screen projection; canonical cycles remain u64 data"
    )]
    fn show_digital_capture_plot(&mut self, ui: &mut egui::Ui) {
        let channel_count = self.diagnostics.capture_channels().len();
        if channel_count == 0 {
            ui.weak("The canonical capture has no digital channels.");
            return;
        }
        let width = ui.available_width().max(360.0);
        let height = 48.0 + 42.0 * channel_count as f32;
        let (response, painter) =
            ui.allocate_painter(egui::vec2(width, height), egui::Sense::click_and_drag());
        let plot = egui::Rect::from_min_max(
            response.rect.min + egui::vec2(185.0, 12.0),
            response.rect.max - egui::vec2(12.0, 30.0),
        );
        painter.rect_filled(plot, 3.0, egui::Color32::from_rgb(17, 21, 29));
        let (start_cycle, end_cycle) = self.diagnostics.capture_window();
        let duration = end_cycle.0.saturating_sub(start_cycle.0).max(1);

        if let Some(pointer) = response
            .hover_pos()
            .filter(|position| plot.contains(*position))
        {
            self.diagnostic_cursor_offset = digital_cursor_offset(plot, pointer.x, duration);
            if response.clicked() {
                let lane_height = plot.height() / channel_count as f32;
                let lane = ((pointer.y - plot.top()) / lane_height)
                    .floor()
                    .clamp(0.0, (channel_count - 1) as f32) as usize;
                self.selected = self
                    .diagnostics
                    .capture_channels()
                    .get(lane)
                    .map(|channel| channel.resource);
            }
        }
        self.diagnostic_cursor_offset = self
            .diagnostic_cursor_offset
            .min(duration.saturating_sub(1));

        for grid in 0..=4_u64 {
            let offset = duration.saturating_mul(grid) / 4;
            let x = digital_plot_x(plot, offset, duration);
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                egui::Stroke::new(0.7_f32, egui::Color32::from_gray(49)),
            );
            painter.text(
                egui::pos2(x, plot.bottom() + 5.0),
                egui::Align2::CENTER_TOP,
                format!("{offset}"),
                egui::FontId::monospace(9.0),
                egui::Color32::GRAY,
            );
        }

        let trigger_offset = self
            .diagnostics
            .trigger()
            .0
            .0
            .saturating_sub(start_cycle.0)
            .min(duration);
        let trigger_x = digital_plot_x(plot, trigger_offset, duration);
        painter.line_segment(
            [
                egui::pos2(trigger_x, plot.top()),
                egui::pos2(trigger_x, plot.bottom()),
            ],
            egui::Stroke::new(1.7_f32, egui::Color32::from_rgb(241, 104, 104)),
        );
        painter.text(
            egui::pos2(trigger_x + 3.0, plot.top() + 2.0),
            egui::Align2::LEFT_TOP,
            "TRIGGER",
            egui::FontId::monospace(9.0),
            egui::Color32::from_rgb(241, 104, 104),
        );

        let cursor_x = digital_plot_x(plot, self.diagnostic_cursor_offset, duration);
        painter.line_segment(
            [
                egui::pos2(cursor_x, plot.top()),
                egui::pos2(cursor_x, plot.bottom()),
            ],
            egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(150)),
        );

        let lane_height = plot.height() / channel_count as f32;
        for (index, channel) in self
            .diagnostics
            .capture_channels()
            .iter()
            .copied()
            .enumerate()
        {
            let channel_index =
                u16::try_from(index).expect("canonical capture channels fit the u16 wire index");
            let lane_top = plot.top() + lane_height * index as f32;
            let center = lane_top + lane_height * 0.5;
            if index > 0 {
                painter.line_segment(
                    [
                        egui::pos2(plot.left(), lane_top),
                        egui::pos2(plot.right(), lane_top),
                    ],
                    egui::Stroke::new(0.6_f32, egui::Color32::from_gray(42)),
                );
            }
            let selected = self.selected == Some(channel.resource);
            let color = diagnostic_channel_color(index, selected);
            let label = board_resource_short_label(&self.snapshot, channel.resource);
            painter.text(
                egui::pos2(plot.left() - 8.0, center),
                egui::Align2::RIGHT_CENTER,
                label,
                egui::FontId::monospace(if selected { 11.0 } else { 10.0 }),
                color,
            );

            let mut level = channel.initial_level;
            let mut previous_offset = 0_u64;
            for transition in self
                .diagnostics
                .transitions()
                .iter()
                .copied()
                .filter(|transition| transition.channel_index == channel_index)
            {
                let from_x = digital_plot_x(plot, previous_offset, duration);
                let to_x = digital_plot_x(plot, transition.offset_cycles, duration);
                let from_y = digital_level_y(center, level);
                let to_y = digital_level_y(center, transition.level);
                painter.line_segment(
                    [egui::pos2(from_x, from_y), egui::pos2(to_x, from_y)],
                    egui::Stroke::new(if selected { 2.4_f32 } else { 1.7_f32 }, color),
                );
                painter.line_segment(
                    [egui::pos2(to_x, from_y), egui::pos2(to_x, to_y)],
                    egui::Stroke::new(if selected { 2.4_f32 } else { 1.7_f32 }, color),
                );
                previous_offset = transition.offset_cycles;
                level = transition.level;
            }
            painter.line_segment(
                [
                    egui::pos2(
                        digital_plot_x(plot, previous_offset, duration),
                        digital_level_y(center, level),
                    ),
                    egui::pos2(plot.right(), digital_level_y(center, level)),
                ],
                egui::Stroke::new(if selected { 2.4_f32 } else { 1.7_f32 }, color),
            );
            let cursor_level = self
                .diagnostics
                .digital_level_at(channel.resource, self.diagnostic_cursor_offset)
                .expect("capture channel has an initial level");
            painter.circle_filled(
                egui::pos2(cursor_x, digital_level_y(center, cursor_level)),
                if selected { 3.8 } else { 2.7 },
                color,
            );
        }
        painter.text(
            egui::pos2(plot.center().x, response.rect.bottom() - 2.0),
            egui::Align2::CENTER_BOTTOM,
            "device-cycle offset (edge record; end exclusive)",
            egui::FontId::monospace(9.0),
            egui::Color32::GRAY,
        );

        ui.horizontal_wrapped(|ui| {
            ui.strong(format!(
                "cursor +{} cycles · absolute {}",
                self.diagnostic_cursor_offset,
                start_cycle.0.saturating_add(self.diagnostic_cursor_offset)
            ));
            for channel in self.diagnostics.capture_channels() {
                let level = self
                    .diagnostics
                    .digital_level_at(channel.resource, self.diagnostic_cursor_offset)
                    .expect("capture channel has an initial level");
                ui.colored_label(
                    diagnostic_channel_color(
                        usize::from(
                            self.diagnostics
                                .capture_channel_index(channel.resource)
                                .expect("iterated channel has an index"),
                        ),
                        self.selected == Some(channel.resource),
                    ),
                    format!(
                        "{} {}",
                        board_resource_short_label(&self.snapshot, channel.resource),
                        digital_level_text(level)
                    ),
                );
            }
        });
        ui.weak(
            "Hover for an exact cycle cursor; click a lane to select the same typed resource in the board ledger. Rendering is a lossy view of retained integer edge records.",
        );
    }

    fn show_supporting_section_counts(&self, ui: &mut egui::Ui) {
        let sections = self.snapshot.supporting_section_counts();
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!(
                "validated ledger: {} buses · {} devices · {} flash regions · {} clocks · {} electrical constraints · {} interrupts · {} safe images · {} HIL gates",
                sections[0],
                sections[1],
                sections[2],
                sections[3],
                sections[4],
                sections[5],
                sections[6],
                self.snapshot.hil_requirement_count()
            ));
        });
    }

    fn show_visual_authority(&self, ui: &mut egui::Ui) {
        ui.add_space(5.0);
        ui.strong("Physical view authority");
        if self.snapshot.visuals().is_empty() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_height(72.0);
                ui.vertical_centered(|ui| {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "NO LICENSED REVISION PHOTO OR REVIEWED HOTSPOTS IN THIS CAPABILITY",
                    );
                    ui.label(
                        "No board silhouette or connector placement is inferred. Add an operator-owned TinyBee V1.0 photograph, exact digest/license/attribution, and physically reconciled resource polygons before an overlay can claim correspondence to the object.",
                    );
                });
            });
            return;
        }
        for visual in self.snapshot.visuals() {
            ui.group(|ui| {
                let (width, height) = visual.pixel_dimensions();
                ui.strong(format!("{} · {width}×{height}", visual.id()));
                ui.monospace(format!(
                    "{} · {} · {}…",
                    visual.asset_path(),
                    visual.media_type(),
                    digest_prefix(visual.asset_digest().0)
                ));
                ui.label(format!(
                    "{} · {} · {} reviewed hotspots",
                    visual.license(),
                    visual.attribution(),
                    visual.hotspots().len()
                ));
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "The canonical visual record is decoded; raster fetch and digest verification must succeed before drawing an image or hotspot overlay.",
                );
            });
        }
    }
}

fn resource_matches_filter(
    resource: &alumina_interface_core::BoardExplorerResource,
    filter: BoardResourceFilter,
) -> bool {
    let descriptor = resource.descriptor();
    match filter {
        BoardResourceFilter::All => true,
        BoardResourceFilter::DiagnosticObservable => resource.is_diagnostic_observable(),
        BoardResourceFilter::DiagnosticClosed => !resource.is_diagnostic_observable(),
        BoardResourceFilter::DigitallyCapturable => resource.is_digitally_capturable(),
        BoardResourceFilter::CaptureClosed => !resource.is_digitally_capturable(),
        BoardResourceFilter::GraphReadable => resource.is_graph_addressable(),
        BoardResourceFilter::GraphClosed => !resource.is_graph_addressable(),
        BoardResourceFilter::Hazardous => descriptor.hazardous_output,
        BoardResourceFilter::Service => descriptor.owner == OwnerDomain::Service,
        BoardResourceFilter::Realtime => descriptor.owner == OwnerDomain::Realtime,
    }
}

fn resource_matches_search(
    resource: &alumina_interface_core::BoardExplorerResource,
    search: &str,
) -> bool {
    search.is_empty()
        || graph_resource_label(resource.descriptor().id)
            .to_ascii_lowercase()
            .contains(search)
        || resource
            .aliases()
            .iter()
            .any(|alias| alias.to_ascii_lowercase().contains(search))
}

const fn owner_label(owner: OwnerDomain) -> &'static str {
    match owner {
        OwnerDomain::Service => "Service",
        OwnerDomain::Realtime => "Realtime",
    }
}

const fn safe_value_label(value: SafeValue) -> &'static str {
    match value {
        SafeValue::NotApplicable => "n/a",
        SafeValue::HighImpedance => "high-Z",
        SafeValue::Low => "low",
        SafeValue::High => "high",
        SafeValue::EngineImage => "engine image",
    }
}

fn display_bytes(bytes: u64) -> String {
    if bytes.is_multiple_of(1_024 * 1_024) {
        format!("{} MiB", bytes / (1_024 * 1_024))
    } else if bytes.is_multiple_of(1_024) {
        format!("{} KiB", bytes / 1_024)
    } else {
        format!("{bytes} B")
    }
}

const fn overview_quality_color(quality: SampleQuality) -> egui::Color32 {
    match quality {
        SampleQuality::Valid => egui::Color32::from_rgb(123, 214, 149),
        SampleQuality::Stale => egui::Color32::YELLOW,
        SampleQuality::Unavailable => egui::Color32::GRAY,
        SampleQuality::Faulted => egui::Color32::from_rgb(241, 104, 104),
    }
}

const fn provenance_short(provenance: SampleProvenance) -> &'static str {
    match provenance {
        SampleProvenance::Measured => "MEASURED",
        SampleProvenance::Latched => "LATCHED",
        SampleProvenance::Inferred => "INFERRED",
        SampleProvenance::LastCommanded => "LAST-COMMANDED",
        SampleProvenance::Simulated => "SIM",
    }
}

fn resource_value_text(value: ResourceValue) -> String {
    match value {
        ResourceValue::Unavailable => "unavailable".to_owned(),
        ResourceValue::Boolean(value) => if value { "HIGH" } else { "LOW" }.to_owned(),
        ResourceValue::Unsigned(value) => value.to_string(),
        ResourceValue::Signed(value) => value.to_string(),
        ResourceValue::ExactRatio {
            numerator,
            denominator,
        } => format!("{numerator}/{denominator}"),
    }
}

fn capture_quality_text(flags: CaptureQualityFlags) -> String {
    let mut labels = Vec::new();
    if flags.contains(CaptureQualityFlags::OVERFLOW) {
        labels.push("OVERFLOW");
    }
    if flags.contains(CaptureQualityFlags::PRETRIGGER_TRUNCATED) {
        labels.push("PRETRIGGER TRUNCATED");
    }
    if flags.contains(CaptureQualityFlags::POSTTRIGGER_TRUNCATED) {
        labels.push("POSTTRIGGER TRUNCATED");
    }
    if flags.contains(CaptureQualityFlags::DECIMATED) {
        labels.push("DECIMATED");
    }
    if flags.contains(CaptureQualityFlags::CLOCK_UNQUALIFIED) {
        labels.push("CLOCK UNQUALIFIED");
    }
    if flags.contains(CaptureQualityFlags::DISCONTINUITY) {
        labels.push("DISCONTINUITY");
    }
    if flags.contains(CaptureQualityFlags::SOFTWARE_SAMPLED) {
        labels.push("SOFTWARE SAMPLED");
    }
    if labels.is_empty() {
        "quality: no loss flags".to_owned()
    } else {
        format!("quality: {}", labels.join(" · "))
    }
}

fn printable_identity(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).map_or_else(
        |_| byte_prefix(bytes, bytes.len()),
        |text| {
            if text.chars().all(|character| !character.is_control()) {
                text.to_owned()
            } else {
                byte_prefix(bytes, bytes.len())
            }
        },
    )
}

fn byte_prefix(bytes: &[u8], count: usize) -> String {
    let mut result = String::with_capacity(count.min(bytes.len()) * 2);
    for byte in bytes.iter().take(count) {
        use core::fmt::Write as _;
        let _ = write!(result, "{byte:02x}");
    }
    result
}

fn board_resource_short_label(snapshot: &BoardExplorerSnapshot, resource: ResourceId) -> String {
    let canonical = graph_resource_label(resource);
    snapshot
        .resource(resource)
        .and_then(|record| record.aliases().first())
        .map_or(canonical.clone(), |alias| format!("{alias} · {canonical}"))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "pointer position is a deliberately lossy screen projection onto an integer cursor"
)]
fn digital_cursor_offset(rect: egui::Rect, x: f32, duration: u64) -> u64 {
    if rect.width() <= 0.0 {
        return 0;
    }
    let fraction = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    (f64::from(fraction) * duration as f64).round() as u64
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "canonical integer cycles are projected only into finite f32 screen coordinates"
)]
fn digital_plot_x(rect: egui::Rect, offset: u64, duration: u64) -> f32 {
    if duration == 0 {
        rect.left()
    } else {
        rect.left() + rect.width() * (offset as f64 / duration as f64) as f32
    }
}

fn diagnostic_channel_color(index: usize, selected: bool) -> egui::Color32 {
    if selected {
        return egui::Color32::WHITE;
    }
    DIAGNOSTIC_CHANNEL_COLORS[index % DIAGNOSTIC_CHANNEL_COLORS.len()]
}

const fn digital_level_y(center: f32, level: DigitalLevel) -> f32 {
    match level {
        DigitalLevel::Unknown => center,
        DigitalLevel::Low => center + 8.0,
        DigitalLevel::High => center - 8.0,
    }
}

const fn digital_level_text(level: DigitalLevel) -> &'static str {
    match level {
        DigitalLevel::Unknown => "?",
        DigitalLevel::Low => "LOW",
        DigitalLevel::High => "HIGH",
    }
}

/// Offline authoring proof that joins canonical CAM output to simulated,
/// reconciled cache observations before exposing any graph job identity.
struct CachedJobGraphProof {
    catalog: GraphCachedJobCatalog,
    registry: GraphNodeRegistry,
    workspace: GraphWorkspaceDocument,
    encoding: CanonicalGraphWorkspaceEncoding,
    selected_entry: usize,
    selected_slot: usize,
    selected_node: Option<GraphNodeId>,
    status: String,
}

impl CachedJobGraphProof {
    fn try_new(job: &CanonicalGlobalJob2) -> Result<Self, String> {
        let ready = simulate_cache_delivery(job).map_err(|error| error.to_string())?;
        let catalog = derive_graph_cached_job_catalog_from_ready(
            job,
            &ready,
            GraphCachedJobCatalogLimits::interactive(),
        )
        .map_err(|error| error.to_string())?;
        let registry = cached_job_registry()?;
        let graph = GraphDocument::try_new(
            0,
            registry.context_schema().clone(),
            registry.context_clocks().to_vec(),
            Vec::new(),
            Vec::new(),
        )
        .map_err(|error| error.to_string())?;
        let mut workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            0,
            1,
            1,
            graph,
            Vec::new(),
        )
        .map_err(|error| error.to_string())?;
        let prototype = cached_job_prototype(&catalog, &registry, 0, "Cached participant 1")?;
        let selected_node = workspace
            .create_node(prototype, 28, 28)
            .map_err(|error| error.to_string())?;
        validate_cached_job_workspace(&catalog, &registry, &workspace)?;
        let encoding = encode_graph_workspace(&workspace).map_err(|error| error.to_string())?;
        Ok(Self {
            catalog,
            registry,
            workspace,
            encoding,
            selected_entry: 0,
            selected_slot: 0,
            selected_node: Some(selected_node),
            status:
                "derived from complete simulated partition + global-manifest cache reconciliation"
                    .to_owned(),
        })
    }

    fn show(&mut self, ui: &mut egui::Ui) -> Option<CachedJobGraphAction> {
        ui.heading("Cached job composite references");
        ui.label(
            "Offline proof only: one exact record retains primary, optional fallback, and bounded-array mirror references. Choices come from canonically replayed CAM artifacts observed in each simulated MCU cache. Every leaf is inert data and does not prepare, arm, or start a job.",
        );
        ui.label(format!(
            "{} participants · ALGW {} bytes · revision {} · unified ALGS history",
            self.catalog.entries().len(),
            self.encoding.bytes().len(),
            self.workspace.revision(),
        ));
        ui.monospace(format!(
            "global job {}… · participant set {}… · graph {}…",
            digest_prefix(self.catalog.global_job_digest().0),
            digest_prefix(self.catalog.participant_set_digest().0),
            digest_prefix(self.encoding.digest().0),
        ));

        self.show_selection(ui);

        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button("add inert reference").clicked() {
                action = Some(CachedJobGraphAction::Add);
            }
            if ui
                .add_enabled(
                    self.selected_node.is_some(),
                    egui::Button::new("rebind selected"),
                )
                .clicked()
            {
                action = Some(CachedJobGraphAction::Rebind);
            }
        });
        ui.label(&self.status);
        ui.weak(
            "The value path uses stable record-field IDs, explicit active branches, and bounded array indices. No digest, device-ID, partition-ID, file path, or command text is accepted; deployment and deterministic start remain separate authorities.",
        );
        action
    }

    fn show_selection(&mut self, ui: &mut egui::Ui) {
        let selected_entry_label = self.catalog.entries().get(self.selected_entry).map_or_else(
            || "unavailable participant".to_owned(),
            cached_job_entry_label,
        );
        egui::ComboBox::from_id_salt("cached_job_catalog_entry")
            .selected_text(selected_entry_label)
            .show_ui(ui, |ui| {
                for (index, entry) in self.catalog.entries().iter().enumerate() {
                    ui.selectable_value(
                        &mut self.selected_entry,
                        index,
                        cached_job_entry_label(entry),
                    );
                }
            });

        let selected_slot = self.show_reference_slot_selection(ui);

        let node_choices: Vec<_> = self
            .workspace
            .graph()
            .nodes()
            .iter()
            .filter_map(|node| {
                cached_job_handle_at_path(node, self.registry.context_schema(), selected_slot.path)
                    .and_then(|handle| {
                        self.catalog
                            .entry_index_for_handle(handle)
                            .map(|entry| (node.id(), entry, node.label().to_owned()))
                    })
            })
            .collect();
        if self
            .selected_node
            .is_none_or(|selected| !node_choices.iter().any(|choice| choice.0 == selected))
        {
            self.selected_node = node_choices.first().map(|choice| choice.0);
        }
        let selected_node_label = self
            .selected_node
            .and_then(|selected| {
                node_choices
                    .iter()
                    .find(|choice| choice.0 == selected)
                    .map(|choice| {
                        format!(
                            "#{} {} · {} participant {}",
                            selected.get(),
                            choice.2,
                            selected_slot.label,
                            choice.1 + 1
                        )
                    })
            })
            .unwrap_or_else(|| "no managed job-reference node".to_owned());
        egui::ComboBox::from_id_salt("cached_job_graph_node")
            .selected_text(selected_node_label)
            .show_ui(ui, |ui| {
                for (node, entry, label) in &node_choices {
                    ui.selectable_value(
                        &mut self.selected_node,
                        Some(*node),
                        format!(
                            "#{} {label} · {} participant {}",
                            node.get(),
                            selected_slot.label,
                            entry + 1
                        ),
                    );
                }
            });
        ui.monospace(format!(
            "selected value path: references.{} · {} segment(s)",
            selected_slot.label,
            selected_slot.path.len()
        ));

        if let Some(entry) = self.catalog.entries().get(self.selected_entry) {
            let participant = entry.participant();
            ui.label(format!(
                "Selected cache fact: {} partition bytes · {} blocks · {} Hz local timer",
                participant.partition_byte_len, participant.block_count, participant.local_timer_hz,
            ));
            ui.monospace(format!(
                "capability {}… · config {}… · partition {}…",
                digest_prefix(participant.capability_digest.0),
                digest_prefix(participant.config_digest.0),
                digest_prefix(participant.partition_digest.0),
            ));
        }
    }

    fn show_reference_slot_selection(&mut self, ui: &mut egui::Ui) -> CachedJobReferenceSlot {
        if self.selected_slot >= CACHED_JOB_REFERENCE_SLOTS.len() {
            self.selected_slot = 0;
        }
        egui::ComboBox::from_id_salt("cached_job_reference_slot")
            .selected_text(format!(
                "composite leaf · {}",
                CACHED_JOB_REFERENCE_SLOTS[self.selected_slot].label
            ))
            .show_ui(ui, |ui| {
                for (index, slot) in CACHED_JOB_REFERENCE_SLOTS.iter().enumerate() {
                    ui.selectable_value(
                        &mut self.selected_slot,
                        index,
                        format!("composite leaf · {}", slot.label),
                    );
                }
            });
        CACHED_JOB_REFERENCE_SLOTS[self.selected_slot]
    }

    fn prepare_add_selected_reference(&self) -> Result<CachedJobGraphUpdate, String> {
        let index = self.workspace.graph().nodes().len();
        let ordinal = index
            .checked_add(1)
            .ok_or_else(|| "cached job node count overflow".to_owned())?;
        let x = i32::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(260))
            .and_then(|offset| offset.checked_add(28))
            .ok_or_else(|| "cached job canvas coordinate overflow".to_owned())?;
        let prototype = cached_job_prototype(
            &self.catalog,
            &self.registry,
            self.selected_entry,
            &format!("Cached participant {ordinal}"),
        )?;
        let mut candidate = self.workspace.clone();
        let node = candidate
            .create_node(prototype, x, 28)
            .map_err(|error| error.to_string())?;
        validate_cached_job_workspace(&self.catalog, &self.registry, &candidate)?;
        let encoding = encode_graph_workspace(&candidate).map_err(|error| error.to_string())?;
        Ok(CachedJobGraphUpdate {
            workspace: candidate,
            encoding,
            selected_node: Some(node),
            status: format!(
                "added inert cached-job reference #{} from audited catalog entry {}",
                node.get(),
                self.selected_entry + 1
            ),
        })
    }

    fn prepare_rebind_selected_reference(&self) -> Result<CachedJobGraphUpdate, String> {
        let node = self
            .selected_node
            .ok_or_else(|| "select a catalog-managed job node first".to_owned())?;
        let slot = CACHED_JOB_REFERENCE_SLOTS
            .get(self.selected_slot)
            .copied()
            .ok_or_else(|| "select a bounded cached-job composite leaf first".to_owned())?;
        let current_entry = self
            .workspace
            .graph()
            .node(node)
            .and_then(|node| {
                cached_job_handle_at_path(node, self.registry.context_schema(), slot.path)
            })
            .and_then(|handle| self.catalog.entry_index_for_handle(handle));
        if current_entry == Some(self.selected_entry) {
            return Ok(CachedJobGraphUpdate {
                workspace: self.workspace.clone(),
                encoding: self.encoding.clone(),
                selected_node: self.selected_node,
                status: format!(
                    "retained inert reference #{} at references.{} on audited catalog entry {}",
                    node.get(),
                    slot.label,
                    self.selected_entry + 1
                ),
            });
        }
        let mut candidate = self.workspace.clone();
        let encoding = select_graph_cached_job_handle(
            &self.catalog,
            &self.registry,
            &mut candidate,
            node,
            CACHED_JOB_PARAMETER,
            slot.path,
            self.selected_entry,
        )
        .map_err(|error| error.to_string())?;
        validate_cached_job_workspace(&self.catalog, &self.registry, &candidate)?;
        Ok(CachedJobGraphUpdate {
            workspace: candidate,
            encoding,
            selected_node: self.selected_node,
            status: format!(
                "rebound inert reference #{} at references.{} to audited catalog entry {}",
                node.get(),
                slot.label,
                self.selected_entry + 1
            ),
        })
    }

    fn prepare_action(&self, action: CachedJobGraphAction) -> Result<CachedJobGraphUpdate, String> {
        match action {
            CachedJobGraphAction::Add => self.prepare_add_selected_reference(),
            CachedJobGraphAction::Rebind => self.prepare_rebind_selected_reference(),
        }
    }

    fn commit_update(&mut self, update: CachedJobGraphUpdate) {
        self.workspace = update.workspace;
        self.encoding = update.encoding;
        self.selected_node = update.selected_node;
        self.status = update.status;
    }

    fn replay_persisted_workspace(
        &self,
        bytes: &[u8],
    ) -> Result<(GraphWorkspaceDocument, CanonicalGraphWorkspaceEncoding), String> {
        let replay = replay_graph_workspace(
            bytes,
            self.workspace.limits(),
            self.workspace.graph().schema().limits(),
        )
        .map_err(|error| error.to_string())?;
        validate_cached_job_workspace(&self.catalog, &self.registry, replay.document())?;
        Ok((replay.document().clone(), replay.encoding().clone()))
    }

    fn restore_persisted_workspace(
        &mut self,
        workspace: GraphWorkspaceDocument,
        encoding: CanonicalGraphWorkspaceEncoding,
    ) {
        self.workspace = workspace;
        self.encoding = encoding;
        self.selected_node = self
            .workspace
            .graph()
            .nodes()
            .first()
            .map(NodeDefinition::id);
        if self.selected_slot >= CACHED_JOB_REFERENCE_SLOTS.len() {
            self.selected_slot = 0;
        }
        let slot = CACHED_JOB_REFERENCE_SLOTS[self.selected_slot];
        self.selected_entry = self
            .selected_node
            .and_then(|node| self.workspace.graph().node(node))
            .and_then(|node| {
                cached_job_handle_at_path(node, self.registry.context_schema(), slot.path)
            })
            .and_then(|handle| self.catalog.entry_index_for_handle(handle))
            .unwrap_or(0);
        "restored catalog-bound cached-job ALGW from application storage"
            .clone_into(&mut self.status);
    }
}

#[derive(Clone, Copy)]
enum CachedJobGraphAction {
    Add,
    Rebind,
}

struct CachedJobGraphUpdate {
    workspace: GraphWorkspaceDocument,
    encoding: CanonicalGraphWorkspaceEncoding,
    selected_node: Option<GraphNodeId>,
    status: String,
}

fn cached_job_registry() -> Result<GraphNodeRegistry, String> {
    let schema = GraphSchema::try_new(
        GraphLimits::interactive(),
        Vec::new(),
        vec![
            TypeDefinition::new(
                CACHED_JOB_HANDLE_TYPE,
                "job.cached.participant",
                TypeKind::JobHandle,
            ),
            TypeDefinition::new(
                CACHED_JOB_OPTION_TYPE,
                "job.cached.optional-participant",
                TypeKind::Option {
                    value: CACHED_JOB_HANDLE_TYPE,
                },
            ),
            TypeDefinition::new(
                CACHED_JOB_ARRAY_TYPE,
                "job.cached.participant-array",
                TypeKind::Array {
                    element: CACHED_JOB_HANDLE_TYPE,
                    maximum_items: 4,
                },
            ),
            TypeDefinition::new(
                CACHED_JOB_REFERENCE_SET_TYPE,
                "job.cached.reference-set",
                TypeKind::Record {
                    fields: vec![
                        RecordField::new(
                            CACHED_JOB_PRIMARY_FIELD,
                            "primary",
                            CACHED_JOB_HANDLE_TYPE,
                        ),
                        RecordField::new(
                            CACHED_JOB_FALLBACK_FIELD,
                            "fallback",
                            CACHED_JOB_OPTION_TYPE,
                        ),
                        RecordField::new(
                            CACHED_JOB_MIRRORS_FIELD,
                            "mirrors",
                            CACHED_JOB_ARRAY_TYPE,
                        ),
                    ],
                },
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let context = GraphDocument::try_new(0, schema, Vec::new(), Vec::new(), Vec::new())
        .map_err(|error| error.to_string())?;
    GraphNodeRegistry::try_new(
        GraphAnalysisLimits::interactive(),
        &context,
        vec![NodeSchema::new(
            NodeKind::new(CACHED_JOB_KIND_NAME, 1),
            ExecutionDomainSet::HOST_EXACT,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![NodeParameterContract::new(
                CACHED_JOB_PARAMETER,
                "references",
                CACHED_JOB_REFERENCE_SET_TYPE,
            )],
            Vec::new(),
            Vec::new(),
            None,
        )],
    )
    .map_err(|error| error.to_string())
}

fn cached_job_prototype(
    catalog: &GraphCachedJobCatalog,
    registry: &GraphNodeRegistry,
    entry: usize,
    label: &str,
) -> Result<GraphNodePrototype, String> {
    let entry = catalog
        .entries()
        .get(entry)
        .copied()
        .ok_or_else(|| "cached job catalog entry is unavailable".to_owned())?;
    let handle = entry.handle();
    let value = TypedGraphValue::try_new(
        registry.context_schema(),
        CACHED_JOB_REFERENCE_SET_TYPE,
        GraphValue::Record(vec![
            RecordValueField {
                field: CACHED_JOB_PRIMARY_FIELD,
                value: GraphValue::JobHandle(handle),
            },
            RecordValueField {
                field: CACHED_JOB_FALLBACK_FIELD,
                value: GraphValue::OptionSome(Box::new(GraphValue::JobHandle(handle))),
            },
            RecordValueField {
                field: CACHED_JOB_MIRRORS_FIELD,
                value: GraphValue::Array(vec![GraphValue::JobHandle(handle)]),
            },
        ]),
    )
    .map_err(|error| error.to_string())?;
    Ok(GraphNodePrototype::new(
        NodeKind::new(CACHED_JOB_KIND_NAME, 1),
        label,
        ExecutionDomain::HostExact,
        Vec::new(),
        Vec::new(),
        vec![NodeParameter::new(
            CACHED_JOB_PARAMETER,
            "references",
            value,
        )],
    ))
}

fn cached_job_handle_at_path(
    node: &NodeDefinition,
    schema: &GraphSchema,
    path: &[GraphValuePathSegment],
) -> Option<alumina_interface_core::JobGraphHandle> {
    let parameter = node
        .parameters()
        .iter()
        .find(|parameter| parameter.id() == CACHED_JOB_PARAMETER)?;
    let (value_type, value) = parameter.value().value_at_path(schema, path).ok()?;
    if !matches!(schema.value_type(value_type)?.kind(), TypeKind::JobHandle) {
        return None;
    }
    match value {
        GraphValue::JobHandle(handle) => Some(*handle),
        _ => None,
    }
}

fn validate_cached_job_workspace(
    catalog: &GraphCachedJobCatalog,
    registry: &GraphNodeRegistry,
    workspace: &GraphWorkspaceDocument,
) -> Result<(), String> {
    analyze_graph_draft(workspace.graph(), registry).map_err(|error| error.to_string())?;
    for node in workspace.graph().nodes() {
        for slot in CACHED_JOB_REFERENCE_SLOTS {
            let handle = cached_job_handle_at_path(node, registry.context_schema(), slot.path)
                .ok_or_else(|| {
                    format!(
                        "cached job node #{} has no typed references.{} job leaf",
                        node.id().get(),
                        slot.label
                    )
                })?;
            if catalog.entry_index_for_handle(handle).is_none() {
                return Err(format!(
                    "cached job node #{} references.{} carries a raw, stale, or foreign identity",
                    node.id().get(),
                    slot.label
                ));
            }
        }
    }
    Ok(())
}

fn cached_job_entry_label(
    entry: &alumina_interface_core::graph::GraphCachedJobCatalogEntry,
) -> String {
    let participant = entry.participant();
    format!(
        "MCU {}… · partition {}…",
        digest_prefix16(participant.device_id.0),
        digest_prefix(participant.partition_digest.0),
    )
}

/// Browser/native inspector for one shared exact control graph and trace.
pub(crate) struct ExactControlWorkspace {
    fixture: RepresentativeExactControlGraph,
    workspace: GraphWorkspaceDocument,
    workspace_encoding: CanonicalGraphWorkspaceEncoding,
    component: Option<ComponentPackage>,
    component_status: String,
    probes: Option<ProbePackage>,
    probe_status: String,
    board_explorer: BoardExplorerPanel,
    target_resources: TargetResourceProof,
    cached_jobs: CachedJobGraphProof,
    history: GraphAuthoringSessionHistory,
    presentation: GraphPresentation,
    traces: Vec<TraceSeries>,
    palette: Vec<NodePaletteEntry>,
    palette_index: usize,
    parameter_drafts: BTreeMap<(GraphNodeId, u32), String>,
    node_label_drafts: BTreeMap<GraphNodeId, String>,
    probe_drafts: BTreeMap<GraphProbeId, ProbeEditDraft>,
    panel_item_drafts: BTreeMap<GraphFrontPanelItemId, PanelItemDraft>,
    component_connector_drafts: BTreeMap<ComponentConnectorSelection, ComponentConnectorDraft>,
    component_connector_scope: Option<Digest>,
    component_definition: ComponentDefinitionEditor,
    selected_node: Option<GraphNodeId>,
    selected_panel_item: Option<GraphFrontPanelItemId>,
    selected_component_connector: Option<ComponentConnectorSelection>,
    new_component_input_name: String,
    new_component_input_target: Option<WireEndpoint>,
    new_component_output_name: String,
    new_component_output_source: Option<WireEndpoint>,
    new_panel_item_name: String,
    new_panel_binding: Option<GraphFrontPanelBinding>,
    panel_drag: Option<PanelItemDrag>,
    selected_hierarchy_component: Option<Digest>,
    selected_hierarchy_instance: Option<GraphNodeId>,
    new_library_component_name: String,
    hierarchy_source_browser: HierarchySourceBrowser,
    pending_hierarchy_source: Option<WireEndpoint>,
    hierarchy_drag: Option<NodeDrag>,
    pending_source: Option<WireEndpoint>,
    drag: Option<NodeDrag>,
    edit_status: String,
    persistence_dirty: bool,
    persistence_attempted: bool,
    file_status: String,
    authoring_session_file_bridge: BoundedFileBridge,
    workspace_file_bridge: BoundedFileBridge,
    probe_file_bridge: BoundedFileBridge,
    component_file_bridge: BoundedFileBridge,
    hierarchy_source_file_bridge: BoundedFileBridge,
    target_success_replay_file_bridge: BoundedFileBridge,
    target_fault_replay_file_bridge: BoundedFileBridge,
    trigger_pre_samples: u32,
    trigger_post_samples: u32,
    cursor_root_tick: Rational,
}

impl ExactControlWorkspace {
    #[cfg(test)]
    pub(crate) fn try_new() -> Result<Self, String> {
        Self::try_new_with_persisted(None)
    }

    #[cfg(test)]
    pub(crate) fn try_new_with_persisted(persisted: Option<&str>) -> Result<Self, String> {
        let program = alumina_interface_core::compile_representative_program()
            .map_err(|error| error.to_string())?;
        let job = alumina_interface_core::compile_representative_global_job(&program)
            .map_err(|error| error.to_string())?;
        Self::try_new_with_persisted_job(persisted, &job)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the complete initial ALGS fixture and all editor-local draft state are constructed together"
    )]
    pub(crate) fn try_new_with_persisted_job(
        persisted: Option<&str>,
        job: &CanonicalGlobalJob2,
    ) -> Result<Self, String> {
        let fixture =
            compile_representative_exact_control_graph().map_err(|error| error.to_string())?;
        let (workspace, workspace_encoding, presentation) = initial_workspace(&fixture)?;
        let component = representative_component(&workspace, &fixture)?;
        let selected_hierarchy_component = Some(component.encoding.digest());
        let selected_hierarchy_instance = component
            .hierarchy
            .document
            .instances()
            .iter()
            .find(|instance| instance.scope() == GraphInstanceScope::Root)
            .map(|instance| instance.node());
        let selected_panel_item = component
            .document
            .panel_items()
            .first()
            .map(GraphFrontPanelItem::id);
        let selected_component_connector = component
            .document
            .inputs()
            .first()
            .map(|input| ComponentConnectorSelection::Input(input.id()))
            .or_else(|| {
                component
                    .document
                    .outputs()
                    .first()
                    .map(|output| ComponentConnectorSelection::Output(output.id()))
            });
        let probes = representative_probes(&workspace)?;
        let board_explorer = tinybee_board_explorer()?;
        let target_resources = tinybee_resource_proof()?;
        let cached_jobs = CachedJobGraphProof::try_new(job)?;
        let traces = trace_series(&fixture, &workspace, &probes.document)?;
        let palette = control_palette(&fixture)?;
        let trigger = probes
            .document
            .trigger()
            .ok_or_else(|| "reference ALGP has no edge trigger".to_owned())?;
        let initial_projection = exact_probe_projection(&fixture, &workspace, &probes.document)?;
        let cursor_root_tick = initial_projection
            .trigger_root_window()
            .map_or_else(|| Rational::from(0), |(_, trigger, _)| trigger.clone());
        let mut result = Self {
            fixture,
            workspace,
            workspace_encoding,
            component: Some(component),
            component_status: "canonical ALGC connector pane and front panel attached".to_owned(),
            probes: Some(probes),
            probe_status: "canonical ALGP diagnostic probes attached".to_owned(),
            board_explorer,
            target_resources,
            cached_jobs,
            history: GraphAuthoringSessionHistory::default(),
            presentation,
            traces,
            palette,
            palette_index: 0,
            parameter_drafts: BTreeMap::new(),
            node_label_drafts: BTreeMap::new(),
            probe_drafts: BTreeMap::new(),
            panel_item_drafts: BTreeMap::new(),
            component_connector_drafts: BTreeMap::new(),
            component_connector_scope: selected_hierarchy_component,
            component_definition: ComponentDefinitionEditor::default(),
            selected_node: None,
            selected_panel_item,
            selected_component_connector,
            new_component_input_name: String::new(),
            new_component_input_target: None,
            new_component_output_name: String::new(),
            new_component_output_source: None,
            new_panel_item_name: String::new(),
            new_panel_binding: None,
            panel_drag: None,
            selected_hierarchy_component,
            selected_hierarchy_instance,
            new_library_component_name: "user.new_component".to_owned(),
            hierarchy_source_browser: HierarchySourceBrowser::default(),
            pending_hierarchy_source: None,
            hierarchy_drag: None,
            pending_source: None,
            drag: None,
            edit_status: "canonical workspace ready; no structural edits".to_owned(),
            persistence_dirty: persisted.is_none(),
            persistence_attempted: false,
            file_status: "canonical workspace has not been exported this session".to_owned(),
            authoring_session_file_bridge: BoundedFileBridge::default(),
            workspace_file_bridge: BoundedFileBridge::default(),
            probe_file_bridge: BoundedFileBridge::default(),
            component_file_bridge: BoundedFileBridge::default(),
            hierarchy_source_file_bridge: BoundedFileBridge::default(),
            target_success_replay_file_bridge: BoundedFileBridge::default(),
            target_fault_replay_file_bridge: BoundedFileBridge::default(),
            trigger_pre_samples: trigger.pretrigger_samples(),
            trigger_post_samples: trigger.posttrigger_samples(),
            cursor_root_tick,
        };
        result.reset_probe_drafts();
        if let Some(persisted) = persisted {
            match decode_persisted_authoring_session(persisted)
                .and_then(|bytes| result.restore_authoring_session_bytes(&bytes))
            {
                Ok(()) => {
                    "restored one exact canonical ALGS authoring session from application storage"
                        .clone_into(&mut result.edit_status);
                }
                Err(error) => {
                    result.persistence_dirty = true;
                    result.edit_status = format!(
                        "persisted ALGS authoring session rejected atomically; canonical reference loaded instead: {error}"
                    );
                }
            }
        }
        Ok(result)
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) const fn persistence_pending(&self) -> bool {
        self.persistence_dirty && !self.persistence_attempted
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn persisted_authoring_session(&self) -> Result<String, String> {
        let encoding = self.authoring_session_encoding()?;
        encode_persisted_authoring_session(&encoding)
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn mark_persisted(&mut self) {
        self.persistence_dirty = false;
        self.persistence_attempted = false;
    }

    pub(crate) fn note_persistence_error(&mut self, error: &str) {
        self.persistence_attempted = true;
        self.file_status = format!("browser persistence failed: {error}");
    }

    pub(crate) fn show_sidebar(&self, ui: &mut egui::Ui) {
        let document = self.workspace.graph();
        ui.label("Exact control graph workspace");
        ui.label(format!(
            "Draft nodes / wires: {} / {} · revision {}",
            document.nodes().len(),
            document.wires().len(),
            self.workspace.revision()
        ));
        ui.label(format!(
            "Canonical workspace: {} bytes",
            self.workspace_encoding.bytes().len()
        ));
        ui.monospace(format!(
            "workspace {}…",
            digest_prefix(self.workspace_encoding.digest().0)
        ));
        if let Some(component) = &self.component {
            ui.label(format!(
                "Component: {} v{} · {} inputs / {} outputs / {} panel items · {} bytes",
                component.document.name(),
                component.document.component_version(),
                component.document.inputs().len(),
                component.document.outputs().len(),
                component.document.panel_items().len(),
                component.encoding.bytes().len()
            ));
            ui.monospace(format!(
                "component {}…",
                digest_prefix(component.encoding.digest().0)
            ));
            ui.label(format!(
                "Hierarchy: {} expanded / depth {} → {} nodes / {} wires · {} bytes",
                component.hierarchy.document.flattened_instance_count(),
                hierarchy_depth(&component.hierarchy.flattening),
                component.hierarchy.document.flattened_node_count(),
                component.hierarchy.document.flattened_wire_count(),
                component.hierarchy.encoding.bytes().len()
            ));
            ui.monospace(format!(
                "hierarchy {}… → ALGW {}…",
                digest_prefix(component.hierarchy.encoding.digest().0),
                digest_prefix(component.hierarchy.flattening.encoding().digest().0)
            ));
            ui.monospace(format!(
                "source map {}… · {} bytes",
                digest_prefix(component.hierarchy.source_map.digest().0),
                component.hierarchy.source_map.bytes().len()
            ));
        }
        self.show_probe_sidebar(ui);
        ui.label(format!(
            "Authoring-session history: {} undo / {} redo · {} ALGS bytes",
            self.history.undo_len(),
            self.history.redo_len(),
            self.history.retained_bytes()
        ));
        #[cfg(target_arch = "wasm32")]
        ui.label(if self.persistence_dirty && self.persistence_attempted {
            "Browser persistence: failed; edit to retry"
        } else if self.persistence_dirty {
            "Browser persistence: pending"
        } else {
            "Browser persistence: exact ALGS authoring session saved"
        });
        ui.label(format!(
            "Reference trace: {} entries / {} bytes",
            self.fixture.simulation().entries().len(),
            self.fixture.trace().bytes().len()
        ));
        ui.monospace(format!(
            "draft graph {}…",
            digest_prefix(self.workspace.graph_digest().0)
        ));
        ui.monospace(format!(
            "registry {}…",
            digest_prefix(self.fixture.simulation().registry_digest().0)
        ));
        ui.monospace(format!(
            "trace {}…",
            digest_prefix(self.fixture.trace().digest().0)
        ));
        ui.label(format!(
            "Cached-job proof: {} exact participants · {} canonical bytes",
            self.cached_jobs.catalog.entries().len(),
            self.cached_jobs.encoding.bytes().len()
        ));
        ui.colored_label(
            egui::Color32::YELLOW,
            "Editor draft only — no deployment, firmware, or output authority.",
        );
        if let Some(selected) = self.selected_node
            && let Some(node) = document.node(selected)
        {
            ui.separator();
            ui.strong("Selected node");
            ui.label(format!("#{} {}", selected.get(), node.label()));
            ui.monospace(format!("{} v{}", node.kind().name(), node.kind().version()));
        }
    }

    fn show_probe_sidebar(&self, ui: &mut egui::Ui) {
        let Some(probes) = &self.probes else {
            ui.colored_label(egui::Color32::YELLOW, &self.probe_status);
            return;
        };
        ui.label(format!(
            "Diagnostic probes: {} bindings / {} canonical bytes",
            probes.document.probes().len(),
            probes.encoding.bytes().len()
        ));
        ui.monospace(format!(
            "probe sidecar {}…",
            digest_prefix(probes.encoding.digest().0)
        ));
        if let Some(trigger) = probes.document.trigger() {
            ui.label(format!(
                "Replay trigger: p{} {} · {} pre / {} post",
                trigger.probe().get(),
                probe_edge_label(trigger.edge()),
                trigger.pretrigger_samples(),
                trigger.posttrigger_samples()
            ));
        }
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        self.handle_history_shortcuts(ui);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.heading("Exact PID / interlock");
            ui.label("50 Hz acquisition to 10 Hz control");
            let trace_current = self.reference_trace_is_current();
            ui.colored_label(
                if trace_current {
                    egui::Color32::LIGHT_BLUE
                } else {
                    egui::Color32::YELLOW
                },
                if trace_current {
                    "canonical reference replay attached"
                } else {
                    "draft graph changed; reference replay detached"
                },
            );
            if ui.small_button("reset draft").clicked() {
                self.reset_draft();
            }
        });
        ui.label(
            "Add audited node kinds, drag headers onto the integer canvas, edit schema-directed exact scalar/composite parameters, and connect typed ports in the in-memory ALGW draft. Resource/job identities require selectors. Secondary-click an input to disconnect. Editing never arms or commands firmware.",
        );
        self.show_workspace_controls(ui);
        self.show_palette(ui);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::from_rgb(96, 169, 232), "— exact Stream");
            ui.colored_label(egui::Color32::from_rgb(241, 178, 84), "— Boolean Stream");
            ui.colored_label(egui::Color32::from_rgb(209, 158, 255), "outlined state");
            if let Some(source) = self.pending_source {
                ui.colored_label(
                    egui::Color32::WHITE,
                    format!("wiring #{}.{}", source.node.get(), source.port.get()),
                );
                if ui.small_button("cancel wire").clicked() {
                    self.pending_source = None;
                    "pending wire cancelled".clone_into(&mut self.edit_status);
                }
            }
        });
        ui.label(&self.edit_status);
        ui.separator();

        self.board_explorer.show(ui);
        ui.separator();

        self.show_target_resources(ui);
        ui.separator();

        if let Some(action) = self.cached_jobs.show(ui) {
            self.apply_cached_job_action(action);
        }
        ui.separator();

        self.show_hierarchy_authoring(ui);
        ui.separator();

        self.show_selected_component_definition(ui);
        ui.separator();

        self.show_front_panel(ui);
        ui.separator();

        let graph_height = (ui.available_height() * 0.5).clamp(230.0, 430.0);
        self.show_graph(ui, graph_height);
        self.show_selected_node(ui);
        ui.separator();
        self.show_probes(ui);
        ui.separator();
        if self.reference_trace_is_current() {
            self.show_trace(ui);
        } else {
            ui.colored_label(
                egui::Color32::YELLOW,
                "The exact trace is hidden because ALGT binds the unedited reference graph. Reset the draft or simulate a newly reviewed graph before plotting it.",
            );
        }
    }

    fn handle_history_shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().wants_keyboard_input() {
            return;
        }
        let (undo, redo) = ui.input(|input| {
            let command = input.modifiers.command;
            let undo = command && !input.modifiers.shift && input.key_pressed(egui::Key::Z);
            let redo = command
                && (input.key_pressed(egui::Key::Y)
                    || (input.modifiers.shift && input.key_pressed(egui::Key::Z)));
            (undo, redo)
        });
        if undo {
            self.navigate_history(false);
        } else if redo {
            self.navigate_history(true);
        }
    }

    fn show_workspace_controls(&mut self, ui: &mut egui::Ui) {
        let mut navigate = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.history.can_undo(), egui::Button::new("undo"))
                .clicked()
            {
                navigate = Some(false);
            }
            if ui
                .add_enabled(self.history.can_redo(), egui::Button::new("redo"))
                .clicked()
            {
                navigate = Some(true);
            }
            ui.weak(format!(
                "{} back / {} forward · {} retained ALGS bytes",
                self.history.undo_len(),
                self.history.redo_len(),
                self.history.retained_bytes()
            ));
        });
        if let Some(redo) = navigate {
            self.navigate_history(redo);
        }

        self.show_authoring_session_file_controls(ui);

        let download_name = format!(
            "alumina-{}.algw",
            digest_prefix(self.workspace_encoding.digest().0)
        );
        let events = self.workspace_file_bridge.show(
            ui,
            self.workspace_encoding.bytes(),
            GraphWorkspaceLimits::interactive().maximum_workspace_bytes,
            &download_name,
            ALGW_FILE,
        );
        for event in events {
            match event {
                BoundedFileEvent::Import(Ok(bytes)) => match self.import_workspace_bytes(&bytes) {
                    Ok(()) => {
                        self.file_status = format!(
                            "imported {} canonical ALGW bytes after full replay",
                            bytes.len()
                        );
                    }
                    Err(error) => {
                        self.file_status =
                            format!("ALGW import rejected without mutation: {error}");
                    }
                },
                BoundedFileEvent::Import(Err(error)) => {
                    self.file_status = format!("ALGW file read rejected: {error}");
                }
                BoundedFileEvent::Export(Ok(bytes)) => {
                    self.file_status = format!("exported {bytes} exact canonical ALGW bytes");
                }
                BoundedFileEvent::Export(Err(error)) => {
                    self.file_status = format!("ALGW export failed: {error}");
                }
            }
        }

        self.show_probe_file_controls(ui);
        ui.weak(&self.file_status);
    }

    fn show_authoring_session_file_controls(&mut self, ui: &mut egui::Ui) {
        let encoding = match self.authoring_session_encoding() {
            Ok(encoding) => encoding,
            Err(error) => {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("ALGS file exchange unavailable: {error}"),
                );
                return;
            }
        };
        let download_name = format!(
            "alumina-session-{}.algs",
            digest_prefix(encoding.digest().0)
        );
        let events = self.authoring_session_file_bridge.show(
            ui,
            encoding.bytes(),
            MAX_GRAPH_AUTHORING_SESSION_BYTES,
            &download_name,
            ALGS_FILE,
        );
        for event in events {
            match event {
                BoundedFileEvent::Import(Ok(bytes)) => {
                    match self.restore_authoring_session_bytes(&bytes) {
                        Ok(()) => {
                            self.persistence_dirty = true;
                            self.persistence_attempted = false;
                            self.file_status = format!(
                                "imported {} exact ALGS bytes after atomic complete-session replay",
                                bytes.len()
                            );
                        }
                        Err(error) => {
                            self.file_status = format!(
                                "ALGS import rejected without authoring-state mutation: {error}"
                            );
                        }
                    }
                }
                BoundedFileEvent::Import(Err(error)) => {
                    self.file_status = format!("ALGS file read rejected: {error}");
                }
                BoundedFileEvent::Export(Ok(bytes)) => {
                    self.file_status =
                        format!("exported {bytes} exact canonical ALGS authoring-session bytes");
                }
                BoundedFileEvent::Export(Err(error)) => {
                    self.file_status = format!("ALGS export failed: {error}");
                }
            }
        }
    }

    fn show_probe_file_controls(&mut self, ui: &mut egui::Ui) {
        if let Some(probes) = &self.probes {
            let download_name =
                format!("alumina-{}.algp", digest_prefix(probes.encoding.digest().0));
            let events = self.probe_file_bridge.show(
                ui,
                probes.encoding.bytes(),
                GraphProbeLimits::interactive().maximum_probe_document_bytes,
                &download_name,
                ALGP_FILE,
            );
            for event in events {
                match event {
                    BoundedFileEvent::Import(Ok(bytes)) => match self.import_probe_bytes(&bytes) {
                        Ok(changed) => {
                            self.file_status = if changed {
                                format!(
                                    "imported {} exact ALGP bytes bound to the current ALGW",
                                    bytes.len()
                                )
                            } else {
                                format!(
                                    "replayed {} exact ALGP bytes; sidecar already matched",
                                    bytes.len()
                                )
                            };
                        }
                        Err(error) => {
                            self.file_status = format!(
                                "ALGP import rejected without graph or sidecar mutation: {error}"
                            );
                        }
                    },
                    BoundedFileEvent::Import(Err(error)) => {
                        self.file_status = format!("ALGP file read rejected: {error}");
                    }
                    BoundedFileEvent::Export(Ok(bytes)) => {
                        self.file_status = format!(
                            "exported {bytes} exact canonical ALGP bytes bound to the current ALGW"
                        );
                    }
                    BoundedFileEvent::Export(Err(error)) => {
                        self.file_status = format!("ALGP export failed: {error}");
                    }
                }
            }
        } else {
            ui.colored_label(
                egui::Color32::YELLOW,
                "ALGP file exchange unavailable because no canonical sidecar is attached",
            );
        }
    }

    fn navigate_history(&mut self, redo: bool) {
        let current = match self.authoring_session_encoding() {
            Ok(current) => current,
            Err(error) => {
                self.edit_status =
                    format!("history current session rejected without mutation: {error}");
                return;
            }
        };
        let mut history = self.history.clone();
        let navigation = if redo {
            history.redo(current, GraphAuthoringSessionReplayLimits::interactive())
        } else {
            history.undo(current, GraphAuthoringSessionReplayLimits::interactive())
        };
        let replay = match navigation {
            Ok(Some(replay)) => replay,
            Ok(None) => {
                self.edit_status = if redo {
                    "no later complete canonical ALGS session is retained".to_owned()
                } else {
                    "no prior complete canonical ALGS session is retained".to_owned()
                };
                return;
            }
            Err(error) => {
                self.edit_status = format!("history navigation rejected without mutation: {error}");
                return;
            }
        };
        let target_digest = replay.encoding().digest();
        let prepared = match self.prepare_authoring_session_bytes(replay.encoding().bytes()) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.edit_status =
                    format!("history target failed UI/catalog admission without mutation: {error}");
                return;
            }
        };
        self.commit_prepared_authoring_session(prepared);
        self.history = history;
        self.persistence_dirty = true;
        self.persistence_attempted = false;
        "restored exact ALGS-bound probe sidecar from unified history"
            .clone_into(&mut self.probe_status);
        self.edit_status = format!(
            "{} complete canonical ALGS session {}…",
            if redo { "redid" } else { "undid to" },
            digest_prefix(target_digest.0),
        );
    }

    fn show_palette(&mut self, ui: &mut egui::Ui) {
        let selected = self
            .palette
            .get(self.palette_index)
            .map_or("palette unavailable", |entry| entry.display_name.as_str());
        let mut add_requested = false;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Audited node palette");
            egui::ComboBox::from_id_salt("exact_control_node_palette")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (index, entry) in self.palette.iter().enumerate() {
                        ui.selectable_value(&mut self.palette_index, index, &entry.display_name);
                    }
                });
            add_requested = ui.small_button("add node").clicked();
            ui.weak(format!("{} fixed HostExact kinds", self.palette.len()));
        });
        if add_requested {
            self.add_palette_node();
        }
    }

    fn show_target_resources(&mut self, ui: &mut egui::Ui) {
        let Self {
            target_resources: proof,
            target_success_replay_file_bridge: success_bridge,
            target_fault_replay_file_bridge: fault_bridge,
            ..
        } = self;
        ui.horizontal_wrapped(|ui| {
            ui.heading("TinyBee executable target-I/O draft");
            ui.monospace(format!(
                "capability {}… · {} advertised / {} reviewed entries",
                digest_prefix(proof.catalog.capability_identity().digest.0),
                proof.catalog.advertised_resource_count(),
                proof.catalog.entries().len()
            ));
            ui.monospace(format!(
                "ALGW {}… · {} nodes / {} wire",
                digest_prefix(proof.encoding.digest().0),
                proof.workspace.graph().nodes().len(),
                proof.workspace.graph().wires().len()
            ));
            ui.monospace(format!(
                "ALGR {}… · {} bytes · implementation {}…",
                digest_prefix(proof.deployment.package_digest.0),
                proof.deployment.package_bytes,
                digest_prefix(proof.deployment.implementation_digest.0)
            ));
            ui.monospace(format!(
                "offline device {}… / config {}…",
                digest_prefix16(proof.catalog.target().device_id.0),
                digest_prefix(proof.catalog.target().config_digest.0)
            ));
        });
        ui.label(
            "This separate Realtime draft is derived from the exact MKS TinyBee V1 8 MiB board package. One paired-input node reads both selected stable Boolean resources every release, emits their conjunction, and feeds a required sink. Each edit is admitted through the exact capability document and lowered offline into the fixed 4 KiB firmware graph package.",
        );
        proof.show_catalog_selector(ui);
        proof.show_reference_selector(ui);
        ui.monospace(format!(
            "lowered opcode {:?} · first {} · second {} · period {} cycles · WCET {} cycles",
            GraphIrOpcode::StableBooleanPairAll,
            graph_resource_label(proof.deployment.first),
            graph_resource_label(proof.deployment.second),
            proof.deployment.realtime_period_cycles,
            proof.deployment.realtime_wcet_cycles,
        ));
        let actor_replay = &proof.deployment.actor_replay;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Portable firmware-actor replay");
            ui.monospace(format!(
                "00→{} · 01→{} · 10→{} · 11→{}",
                u8::from(actor_replay.truth_table[0]),
                u8::from(actor_replay.truth_table[1]),
                u8::from(actor_replay.truth_table[2]),
                u8::from(actor_replay.truth_table[3]),
            ));
        });
        ui.monospace(format!(
            "actual provider order {} → {} · success evidence {}…",
            graph_resource_label(actor_replay.ordered_reads[0]),
            graph_resource_label(actor_replay.ordered_reads[1]),
            digest_prefix(actor_replay.success_evidence.digest().0),
        ));
        ui.monospace(format!(
            "unavailable first input → {:?} · retained reads {} → {} · no completed sink report · fault evidence {}…",
            actor_replay.terminal_fault,
            graph_resource_label(actor_replay.fault_reads[0]),
            graph_resource_label(actor_replay.fault_reads[1]),
            digest_prefix(actor_replay.fault_evidence.digest().0),
        ));
        proof.show_replay_evidence_files(ui, success_bridge, fault_bridge);
        ui.label(&proof.status);
        ui.label(
            "Both resource identities are selectable only from the digest-verified scalar catalog. Stable record-field IDs determine ordered lowering and provider calls. Candidate edits commit ALGW, ALGR, and replay evidence together only after the actual fixed-memory firmware actors reproduce the four-case conjunction and the ordered unavailable-resource fault. Duplicate physical identities and capability-unadvertised resources fail without changing that transaction.",
        );
        ui.colored_label(
            egui::Color32::YELLOW,
            "Offline native/WASM actor replay only: there is no MCU session, upload, install, start, GPIO configuration, or physical read authority. ADC, UART, timers, shifted outputs, and raw GPIO remain closed until matching firmware opcodes/access descriptors are published.",
        );
    }

    fn scroll_to_open_hierarchy_source(
        &mut self,
        ui: &egui::Ui,
        destination: HierarchySourceDestination,
    ) {
        if self.hierarchy_source_browser.scroll_pending
            && self
                .hierarchy_source_browser
                .last_opened
                .as_ref()
                .is_some_and(|selection| selection.destination() == destination)
        {
            ui.scroll_to_cursor(Some(egui::Align::Center));
            self.hierarchy_source_browser.scroll_pending = false;
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the two selectors share one immutable canonical hierarchy snapshot and defer their single action until all display borrows end"
    )]
    fn show_hierarchy_authoring(&mut self, ui: &mut egui::Ui) {
        self.scroll_to_open_hierarchy_source(ui, HierarchySourceDestination::Root);
        ui.heading("Component library / root instances");
        ui.label(
            "Choose one exact embedded ALGC dependency, instantiate it in the hierarchy root, or remove a selected root occurrence. These authoring-only placeholders flatten through ALGH/ALGM and never deploy by themselves.",
        );
        self.reconcile_hierarchy_selection(None);
        let Some(component) = self.component.as_ref() else {
            ui.colored_label(egui::Color32::YELLOW, &self.component_status);
            return;
        };
        let dependencies = component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .map(|dependency| {
                (
                    dependency.digest(),
                    format!(
                        "{} · {}…",
                        dependency.document().name(),
                        digest_prefix(dependency.digest().0)
                    ),
                )
            })
            .collect::<Vec<_>>();
        let root_instances = component
            .hierarchy
            .document
            .instances()
            .iter()
            .filter(|instance| instance.scope() == GraphInstanceScope::Root)
            .map(|instance| {
                let node = instance.node();
                let node_label = component
                    .hierarchy
                    .document
                    .root()
                    .graph()
                    .node(node)
                    .map_or("missing placeholder", NodeDefinition::label);
                let component_name = component
                    .hierarchy
                    .document
                    .dependency(instance.component())
                    .map_or("missing component", |dependency| {
                        dependency.document().name()
                    });
                (
                    node,
                    format!(
                        "#{} {} → {} · {}…",
                        node.get(),
                        node_label,
                        component_name,
                        digest_prefix(instance.component().0)
                    ),
                )
            })
            .collect::<Vec<_>>();
        let selected_dependency_label = dependencies
            .iter()
            .find(|(digest, _)| Some(*digest) == self.selected_hierarchy_component)
            .map_or("no component dependency", |(_, label)| label.as_str());
        let selected_instance_label = root_instances
            .iter()
            .find(|(node, _)| Some(*node) == self.selected_hierarchy_instance)
            .map_or("no root instance", |(_, label)| label.as_str());
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Library component");
            egui::ComboBox::from_id_salt("hierarchy_library_component")
                .selected_text(selected_dependency_label)
                .show_ui(ui, |ui| {
                    for (digest, label) in &dependencies {
                        ui.selectable_value(
                            &mut self.selected_hierarchy_component,
                            Some(*digest),
                            label,
                        );
                    }
                });
            if ui
                .add_enabled(
                    self.selected_hierarchy_component.is_some(),
                    egui::Button::new("add root instance"),
                )
                .clicked()
                && let Some(digest) = self.selected_hierarchy_component
            {
                action = Some(HierarchyUiAction::AddRoot(digest));
            }
        });
        let selected_dependency = self.selected_hierarchy_component;
        let selected_dependency_in_use = selected_dependency.is_some_and(|digest| {
            component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.component() == digest
                        || instance.scope() == GraphInstanceScope::Component(digest)
                })
        });
        let selected_dependency_is_authoritative =
            selected_dependency == Some(component.encoding.digest());
        ui.horizontal_wrapped(|ui| {
            let removable = selected_dependency.is_some()
                && !selected_dependency_in_use
                && !selected_dependency_is_authoritative;
            if ui
                .add_enabled(removable, egui::Button::new("remove library component"))
                .clicked()
                && let Some(digest) = selected_dependency
            {
                action = Some(HierarchyUiAction::RemoveComponent(digest));
            }
            ui.weak(if selected_dependency_is_authoritative {
                "the selected ALGC is the complete-session control authority"
            } else if selected_dependency_in_use {
                "remove every root/nested reference before removing this dependency"
            } else {
                "only exact unreferenced non-authoritative dependencies are removable"
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.strong("New library component");
            ui.add(
                egui::TextEdit::singleline(&mut self.new_library_component_name)
                    .desired_width(260.0)
                    .hint_text("user.component_name"),
            );
            if ui
                .add_enabled(
                    !self.new_library_component_name.is_empty(),
                    egui::Button::new("create empty component"),
                )
                .clicked()
            {
                action = Some(HierarchyUiAction::CreateComponent);
            }
            ui.weak("version 1 · shared exact schema/clocks · empty ALGW · no authority");
        });
        ui.horizontal_wrapped(|ui| {
            ui.strong("Root occurrence");
            egui::ComboBox::from_id_salt("hierarchy_root_instance")
                .selected_text(selected_instance_label)
                .show_ui(ui, |ui| {
                    for (node, label) in &root_instances {
                        ui.selectable_value(
                            &mut self.selected_hierarchy_instance,
                            Some(*node),
                            label,
                        );
                    }
                });
            if ui
                .add_enabled(
                    self.selected_hierarchy_instance.is_some(),
                    egui::Button::new("delete root instance"),
                )
                .clicked()
                && let Some(node) = self.selected_hierarchy_instance
            {
                action = Some(HierarchyUiAction::RemoveRoot(node));
            }
        });
        let component_file_events = selected_dependency
            .and_then(|digest| component.hierarchy.document.dependency(digest))
            .map_or_else(Vec::new, |dependency| {
                let download_name = format!(
                    "alumina-component-{}.algc",
                    digest_prefix(dependency.digest().0)
                );
                self.component_file_bridge.show(
                    ui,
                    dependency.encoding().bytes(),
                    GraphComponentLimits::interactive().maximum_component_bytes,
                    &download_name,
                    ALGC_FILE,
                )
            });
        ui.weak(
            "ALGC import accepts a bounded canonical leaf component only after exact replay, audited graph admission, and current hierarchy-context validation.",
        );
        ui.monospace(format!(
            "{} exact dependencies · {} root bindings · next root node {}",
            dependencies.len(),
            root_instances.len(),
            component.hierarchy.document.root().next_node_id(),
        ));
        ui.label(&self.component_status);
        let root = component.hierarchy.document.root().clone();
        let root_instance_nodes = root_instances
            .iter()
            .map(|(node, _)| *node)
            .collect::<BTreeSet<_>>();
        if action.is_none() {
            action = self.show_hierarchy_root_canvas(ui, &root, &root_instance_nodes);
        }
        if let Some(action) = action {
            self.apply_hierarchy_action(action);
        }
        for event in component_file_events {
            self.handle_component_file_event(event);
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the structural root canvas keeps layered node/port interaction and one deferred canonical hierarchy action in one egui frame"
    )]
    fn show_hierarchy_root_canvas(
        &mut self,
        ui: &mut egui::Ui,
        root: &GraphWorkspaceDocument,
        root_instance_nodes: &BTreeSet<GraphNodeId>,
    ) -> Option<HierarchyUiAction> {
        ui.strong("Canonical hierarchy root canvas");
        ui.weak(
            "Drag a component header to an exact integer placement. Select an output, then a typed input, to connect; secondary-click an owned input to disconnect. Every accepted edit freshly flattens and commits one complete ALGS session.",
        );
        let presentation = match structural_workspace_presentation(root) {
            Ok(presentation) => presentation,
            Err(error) => {
                self.component_status =
                    format!("hierarchy root canvas rejected without mutation: {error}");
                return None;
            }
        };
        let nodes = root.graph().nodes().to_vec();
        let wires = root.graph().wires().to_vec();
        let (opened_root_node, opened_root_wire) =
            match self.hierarchy_source_browser.last_opened.as_ref() {
                Some(HierarchySourceSelection::Node {
                    origin: GraphHierarchyNodeOrigin::Root(node),
                    ..
                }) => (Some(*node), None),
                Some(HierarchySourceSelection::Wire {
                    origin: GraphHierarchyWireOrigin::Root(wire),
                    ..
                }) => (None, Some(*wire)),
                _ => (None, None),
            };
        let focus = HierarchyRootCanvasFocus {
            selected_instance: self.selected_hierarchy_instance,
            opened_source_node: opened_root_node,
            opened_source_wire: opened_root_wire,
        };
        let mut action = None;
        let mut clicked_instance = None;
        egui::ScrollArea::both()
            .id_salt("hierarchy_root_canvas")
            .auto_shrink([false, false])
            .min_scrolled_height(260.0)
            .max_height(340.0)
            .show(ui, |ui| {
                let canvas_size = egui::vec2(
                    presentation.size.x.max(ui.available_width()),
                    presentation.size.y.max(260.0),
                );
                let (canvas, painter) = ui.allocate_painter(canvas_size, egui::Sense::click());
                painter.rect_filled(canvas.rect, 0.0, egui::Color32::from_rgb(17, 21, 29));
                paint_grid(&painter, canvas.rect);
                let origin = canvas.rect.min.to_vec2();
                for wire in &wires {
                    paint_hierarchy_root_wire(
                        &painter,
                        origin,
                        root.graph(),
                        &presentation,
                        focus,
                        wire,
                    );
                }
                for node in &nodes {
                    let Some(node_presentation) = presentation.nodes.get(&node.id()) else {
                        continue;
                    };
                    let rect = node_presentation.rect.translate(origin);
                    let header = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.right(), rect.top() + NODE_HEADER_HEIGHT),
                    );
                    let bound_instance = root_instance_nodes.contains(&node.id());
                    let response = ui.interact(
                        header,
                        egui::Id::new(("hierarchy_root_node", node.id().get())),
                        if bound_instance {
                            egui::Sense::click_and_drag()
                        } else {
                            egui::Sense::click()
                        },
                    );
                    if response.drag_started()
                        && bound_instance
                        && let Some(placement) = root.placement(node.id())
                    {
                        self.hierarchy_drag = Some(NodeDrag {
                            node: node.id(),
                            origin: placement,
                            delta: egui::Vec2::ZERO,
                        });
                    }
                    if response.dragged()
                        && let Some(drag) = self.hierarchy_drag.as_mut()
                        && drag.node == node.id()
                    {
                        drag.delta += response.drag_delta();
                    }
                    let dragging = self
                        .hierarchy_drag
                        .filter(|drag| drag.node == node.id())
                        .filter(|_| response.dragged() || response.drag_stopped());
                    let painted_rect = dragging.map_or(rect, |drag| rect.translate(drag.delta));
                    if response.drag_stopped()
                        && let Some(drag) = dragging
                    {
                        match (
                            quantized_canvas_coordinate(drag.origin.x(), drag.delta.x),
                            quantized_canvas_coordinate(drag.origin.y(), drag.delta.y),
                        ) {
                            (Ok(x), Ok(y)) => {
                                action = Some(HierarchyUiAction::MoveRoot {
                                    node: drag.node,
                                    x,
                                    y,
                                });
                            }
                            (Err(error), _) | (_, Err(error)) => {
                                self.component_status = format!(
                                    "hierarchy root move rejected without mutation: {error}"
                                );
                            }
                        }
                        self.hierarchy_drag = None;
                    }
                    if response.clicked() && bound_instance {
                        clicked_instance = Some(node.id());
                    }
                    for (index, port) in node.inputs().iter().enumerate() {
                        let anchor = port_anchor_for_rect(painted_rect, index, false);
                        let port_response = ui.interact(
                            egui::Rect::from_center_size(anchor, egui::vec2(18.0, 18.0)),
                            egui::Id::new((
                                "hierarchy_root_input",
                                node.id().get(),
                                port.id().get(),
                            )),
                            egui::Sense::click(),
                        );
                        let target = WireEndpoint {
                            node: node.id(),
                            port: port.id(),
                        };
                        if port_response.secondary_clicked() {
                            if let Some(wire) = wires
                                .iter()
                                .find(|wire| wire.target() == target)
                                .map(|wire| wire.id())
                            {
                                action = Some(HierarchyUiAction::DisconnectRoot(wire));
                            } else {
                                self.component_status = format!(
                                    "hierarchy root input #{}.{} is already disconnected",
                                    target.node.get(),
                                    target.port.get()
                                );
                            }
                        } else if port_response.clicked() {
                            if let Some(source) = self.pending_hierarchy_source {
                                action = Some(HierarchyUiAction::ConnectRoot { source, target });
                            } else {
                                self.component_status = format!(
                                    "hierarchy root input #{}.{} selected; choose an output first",
                                    target.node.get(),
                                    target.port.get()
                                );
                            }
                        }
                    }
                    for (index, port) in node.outputs().iter().enumerate() {
                        let anchor = port_anchor_for_rect(painted_rect, index, true);
                        let port_response = ui.interact(
                            egui::Rect::from_center_size(anchor, egui::vec2(18.0, 18.0)),
                            egui::Id::new((
                                "hierarchy_root_output",
                                node.id().get(),
                                port.id().get(),
                            )),
                            egui::Sense::click(),
                        );
                        if port_response.clicked() {
                            let source = WireEndpoint {
                                node: node.id(),
                                port: port.id(),
                            };
                            if self.pending_hierarchy_source == Some(source) {
                                self.pending_hierarchy_source = None;
                                "pending hierarchy root wire cancelled"
                                    .clone_into(&mut self.component_status);
                            } else {
                                self.pending_hierarchy_source = Some(source);
                                self.component_status = format!(
                                    "selected hierarchy root output #{}.{}; choose one typed input",
                                    source.node.get(),
                                    source.port.get()
                                );
                            }
                        }
                    }
                    paint_hierarchy_root_node(
                        &painter,
                        painted_rect,
                        node,
                        focus.selects_node(node.id()),
                        bound_instance,
                        root.placement(node.id()),
                    );
                }
                if let Some(source) = self.pending_hierarchy_source
                    && let Some(source_anchor) =
                        port_anchor(root.graph(), &presentation, source, true)
                    && let Some(pointer) = ui.ctx().pointer_hover_pos()
                {
                    painter.line_segment(
                        [source_anchor + origin, pointer],
                        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    );
                }
            });
        if let Some(node) = clicked_instance {
            self.selected_hierarchy_instance = Some(node);
        }
        action
    }

    #[allow(
        clippy::too_many_lines,
        reason = "node and wire provenance selectors share one immutable ALGM snapshot and defer one transient navigation action until display borrows end"
    )]
    fn show_hierarchy_source_browser(&mut self, ui: &mut egui::Ui, component: &ComponentPackage) {
        self.reconcile_hierarchy_source_browser();
        let flattened = &component.hierarchy.flattening;
        let workspace = flattened.workspace();
        let node_choices = flattened.node_provenance();
        let wire_choices = flattened.wire_provenance();
        let node_choice_label = |mapping: &GraphFlattenedNodeProvenance| {
            let flattened = mapping.flattened_node();
            let node_label = workspace
                .graph()
                .node(flattened)
                .map_or("missing final node", NodeDefinition::label);
            format!(
                "final n{} {node_label} · {}",
                flattened.get(),
                hierarchy_node_origin_label(mapping.origin())
            )
        };
        let wire_choice_label = |mapping: &GraphFlattenedWireProvenance| {
            let flattened = mapping.flattened_wire();
            let endpoints = workspace
                .graph()
                .wires()
                .iter()
                .find(|wire| wire.id() == flattened)
                .map_or_else(
                    || "missing final wire".to_owned(),
                    |wire| {
                        format!(
                            "n{}.p{} → n{}.p{}",
                            wire.source().node.get(),
                            wire.source().port.get(),
                            wire.target().node.get(),
                            wire.target().port.get()
                        )
                    },
                );
            format!(
                "final w{} {endpoints} · {}",
                flattened.get(),
                hierarchy_wire_origin_label(mapping.origin())
            )
        };
        let selected_node_label = node_choices
            .iter()
            .find(|mapping| {
                Some(mapping.flattened_node()) == self.hierarchy_source_browser.selected_node
            })
            .map_or_else(|| "no final node".to_owned(), node_choice_label);
        let selected_wire_label = wire_choices
            .iter()
            .find(|mapping| {
                Some(mapping.flattened_wire()) == self.hierarchy_source_browser.selected_wire
            })
            .map_or_else(|| "no final wire".to_owned(), wire_choice_label);
        let mut action = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Flattened hierarchy source browser");
                ui.weak("read-only ALGM navigation");
            });
            ui.label(
                "Choose one final flattened item. Opening its exact source follows the stable occurrence path into the root or component definition and changes only transient UI selection.",
            );
            ui.horizontal_wrapped(|ui| {
                ui.strong("Final node");
                egui::ComboBox::from_id_salt("hierarchy_source_final_node")
                    .width(640.0)
                    .selected_text(selected_node_label)
                    .show_ui(ui, |ui| {
                        for mapping in node_choices {
                            ui.selectable_value(
                                &mut self.hierarchy_source_browser.selected_node,
                                Some(mapping.flattened_node()),
                                node_choice_label(mapping),
                            );
                        }
                    });
                if ui
                    .add_enabled(
                        self.hierarchy_source_browser.selected_node.is_some(),
                        egui::Button::new("open exact node source"),
                    )
                    .clicked()
                    && let Some(selected) = self.hierarchy_source_browser.selected_node
                    && let Some(mapping) = node_choices
                        .iter()
                        .find(|mapping| mapping.flattened_node() == selected)
                {
                    action = Some(HierarchySourceSelection::Node {
                        flattened: selected,
                        origin: mapping.origin().clone(),
                    });
                }
            });
            if let Some(selected) = self.hierarchy_source_browser.selected_node
                && let Some(mapping) = node_choices
                    .iter()
                    .find(|mapping| mapping.flattened_node() == selected)
            {
                ui.monospace(format!(
                    "exact node origin: {}",
                    hierarchy_node_origin_label(mapping.origin())
                ));
            }
            ui.horizontal_wrapped(|ui| {
                ui.strong("Final wire");
                egui::ComboBox::from_id_salt("hierarchy_source_final_wire")
                    .width(640.0)
                    .selected_text(selected_wire_label)
                    .show_ui(ui, |ui| {
                        for mapping in wire_choices {
                            ui.selectable_value(
                                &mut self.hierarchy_source_browser.selected_wire,
                                Some(mapping.flattened_wire()),
                                wire_choice_label(mapping),
                            );
                        }
                    });
                if ui
                    .add_enabled(
                        self.hierarchy_source_browser.selected_wire.is_some(),
                        egui::Button::new("open exact wire source"),
                    )
                    .clicked()
                    && let Some(selected) = self.hierarchy_source_browser.selected_wire
                    && let Some(mapping) = wire_choices
                        .iter()
                        .find(|mapping| mapping.flattened_wire() == selected)
                {
                    action = Some(HierarchySourceSelection::Wire {
                        flattened: selected,
                        origin: mapping.origin().clone(),
                    });
                }
            });
            if let Some(selected) = self.hierarchy_source_browser.selected_wire
                && let Some(mapping) = wire_choices
                    .iter()
                    .find(|mapping| mapping.flattened_wire() == selected)
            {
                ui.monospace(format!(
                    "exact wire origin: {}",
                    hierarchy_wire_origin_label(mapping.origin())
                ));
            }
            ui.label(&self.hierarchy_source_browser.status);
        });
        if let Some(action) = action {
            self.open_hierarchy_source(action);
        }
    }

    fn open_hierarchy_source(&mut self, selection: HierarchySourceSelection) {
        match self.try_open_hierarchy_source(&selection) {
            Ok(status) => {
                self.hierarchy_source_browser.last_opened = Some(selection);
                self.hierarchy_source_browser.scroll_pending = true;
                self.hierarchy_source_browser.status = status;
            }
            Err(error) => {
                self.hierarchy_source_browser.status =
                    format!("ALGM source navigation rejected without authoring mutation: {error}");
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the four exact ALGM node/wire and root/component origin variants remain visibly exhaustive at one fail-closed navigation boundary"
    )]
    fn try_open_hierarchy_source(
        &mut self,
        selection: &HierarchySourceSelection,
    ) -> Result<String, String> {
        let package = self
            .component
            .clone()
            .ok_or_else(|| "no complete ALGH/ALGM source authority is attached".to_owned())?;
        match selection {
            HierarchySourceSelection::Node { flattened, origin } => {
                if package.hierarchy.flattening.node_origin(*flattened) != Some(origin) {
                    return Err(
                        "the selected final node no longer has that exact origin".to_owned()
                    );
                }
                match origin {
                    GraphHierarchyNodeOrigin::Root(node) => {
                        if package
                            .hierarchy
                            .document
                            .root()
                            .graph()
                            .node(*node)
                            .is_none()
                        {
                            return Err("the exact root source node is unavailable".to_owned());
                        }
                        self.pending_hierarchy_source = None;
                        self.hierarchy_drag = None;
                        Ok(format!(
                            "opened final node {} as root ALGW node {}; canonical ALGS, history, and persistence are unchanged",
                            flattened.get(),
                            node.get()
                        ))
                    }
                    GraphHierarchyNodeOrigin::Component {
                        source_path,
                        component,
                        node,
                    } => {
                        Self::validate_component_source(
                            &package,
                            source_path,
                            *component,
                            Some(*node),
                            None,
                        )?;
                        self.select_component_source(&package, source_path, *component, *node);
                        Ok(format!(
                            "opened final node {} at exact occurrence {} as ALGC {}… node {}; canonical ALGS, history, and persistence are unchanged",
                            flattened.get(),
                            hierarchy_source_path_label(source_path),
                            digest_prefix(component.0),
                            node.get()
                        ))
                    }
                }
            }
            HierarchySourceSelection::Wire { flattened, origin } => {
                if package.hierarchy.flattening.wire_origin(*flattened) != Some(origin) {
                    return Err(
                        "the selected final wire no longer has that exact origin".to_owned()
                    );
                }
                match origin {
                    GraphHierarchyWireOrigin::Root(wire) => {
                        let source = package
                            .hierarchy
                            .document
                            .root()
                            .graph()
                            .wires()
                            .iter()
                            .find(|candidate| candidate.id() == *wire)
                            .ok_or_else(|| {
                                "the exact root source wire is unavailable".to_owned()
                            })?;
                        self.pending_hierarchy_source = None;
                        self.hierarchy_drag = None;
                        Ok(format!(
                            "opened final wire {} as root ALGW wire {} (n{}.p{} → n{}.p{}); canonical ALGS, history, and persistence are unchanged",
                            flattened.get(),
                            wire.get(),
                            source.source().node.get(),
                            source.source().port.get(),
                            source.target().node.get(),
                            source.target().port.get()
                        ))
                    }
                    GraphHierarchyWireOrigin::Component {
                        source_path,
                        component,
                        wire,
                    } => {
                        let source = Self::validate_component_source(
                            &package,
                            source_path,
                            *component,
                            None,
                            Some(*wire),
                        )?
                        .ok_or_else(|| {
                            "the exact component source wire is unavailable".to_owned()
                        })?;
                        self.select_component_source(
                            &package,
                            source_path,
                            *component,
                            source.target().node,
                        );
                        Ok(format!(
                            "opened final wire {} at exact occurrence {} as ALGC {}… wire {} (n{}.p{} → n{}.p{}); canonical ALGS, history, and persistence are unchanged",
                            flattened.get(),
                            hierarchy_source_path_label(source_path),
                            digest_prefix(component.0),
                            wire.get(),
                            source.source().node.get(),
                            source.source().port.get(),
                            source.target().node.get(),
                            source.target().port.get()
                        ))
                    }
                }
            }
        }
    }

    fn validate_component_source(
        package: &ComponentPackage,
        source_path: &[GraphNodeId],
        source_component: Digest,
        node: Option<GraphNodeId>,
        wire: Option<GraphWireId>,
    ) -> Result<Option<WireDefinition>, String> {
        if package
            .hierarchy
            .document
            .component_at_instance_path(source_path)
            != Some(source_component)
        {
            return Err(
                "the selected ALGM occurrence path no longer resolves to its exact component"
                    .to_owned(),
            );
        }
        let dependency = package
            .hierarchy
            .document
            .dependency(source_component)
            .ok_or_else(|| "the selected ALGM source component is unavailable".to_owned())?;
        if let Some(wire) = wire {
            return dependency
                .document()
                .workspace()
                .graph()
                .wires()
                .iter()
                .find(|candidate| candidate.id() == wire)
                .copied()
                .map(Some)
                .ok_or_else(|| "the selected ALGM source wire is unavailable".to_owned());
        }
        let node = node.ok_or_else(|| "the selected ALGM source has no node".to_owned())?;
        if dependency
            .document()
            .workspace()
            .graph()
            .node(node)
            .is_none()
        {
            return Err("the selected ALGM source node is unavailable".to_owned());
        }
        Ok(None)
    }

    fn select_component_source(
        &mut self,
        package: &ComponentPackage,
        source_path: &[GraphNodeId],
        source_component: Digest,
        node: GraphNodeId,
    ) {
        self.selected_hierarchy_component = Some(source_component);
        self.selected_hierarchy_instance = source_path.first().copied();
        self.pending_hierarchy_source = None;
        self.hierarchy_drag = None;
        self.reconcile_hierarchy_selection(None);
        self.reconcile_component_definition_editor();
        if source_component == package.encoding.digest() {
            self.selected_node = Some(node);
            self.pending_source = None;
            self.drag = None;
        } else {
            self.component_definition.selected_node = Some(node);
            self.component_definition.pending_source = None;
            self.component_definition.drag = None;
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "selected dependency identity, palette, exact canvas interaction, and node inspector form one scoped definition editor"
    )]
    fn show_selected_component_definition(&mut self, ui: &mut egui::Ui) {
        self.reconcile_component_definition_editor();
        ui.horizontal_wrapped(|ui| {
            ui.heading("Selected component definition");
            ui.weak("ALGC workspace canvas · authoring only");
        });
        ui.label(
            "Edit the internal ALGW of the selected non-authoritative library dependency. Every accepted node, parameter, placement, or typed-wire edit replaces that exact ALGC, recursively refreshes ALGH/ALGM, and commits one complete ALGS history state.",
        );
        let Some(component) = self.component.clone() else {
            ui.colored_label(egui::Color32::YELLOW, &self.component_definition.status);
            return;
        };
        let Some(scope) = self.component_definition.scope else {
            ui.colored_label(egui::Color32::YELLOW, &self.component_definition.status);
            return;
        };
        let Some(dependency) = component.hierarchy.document.dependency(scope) else {
            ui.colored_label(
                egui::Color32::YELLOW,
                "the selected exact component definition is no longer available",
            );
            return;
        };
        let document = dependency.document().clone();
        let workspace = document.workspace().clone();
        let authoritative = scope == component.encoding.digest();
        ui.monospace(format!(
            "{} · {}… · ALGW {}… · revision {} · {} nodes / {} wires{}",
            document.name(),
            digest_prefix(scope.0),
            digest_prefix(workspace.graph_digest().0),
            document.revision(),
            workspace.graph().nodes().len(),
            workspace.graph().wires().len(),
            if authoritative {
                " · control authority"
            } else {
                " · editable library definition"
            },
        ));
        if authoritative {
            ui.colored_label(
                egui::Color32::YELLOW,
                "This ALGC supplies the complete-session control ALGW. Edit it on the main exact-control canvas below so probes, panel bindings, and control authority move together.",
            );
            ui.label(&self.component_definition.status);
            return;
        }
        self.scroll_to_open_hierarchy_source(ui, HierarchySourceDestination::Component(scope));
        let presentation = match structural_workspace_presentation(&workspace) {
            Ok(presentation) => presentation,
            Err(error) => {
                self.component_definition.status =
                    format!("component definition canvas rejected without mutation: {error}");
                ui.colored_label(egui::Color32::YELLOW, &self.component_definition.status);
                return;
            }
        };

        if self.component_definition.palette_index >= self.palette.len() {
            self.component_definition.palette_index = 0;
        }
        let palette_label = self
            .palette
            .get(self.component_definition.palette_index)
            .map_or("audited palette unavailable", |entry| {
                entry.display_name.as_str()
            });
        let mut add_requested = false;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Definition palette");
            egui::ComboBox::from_id_salt("component_definition_palette")
                .selected_text(palette_label)
                .show_ui(ui, |ui| {
                    for (index, entry) in self.palette.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.component_definition.palette_index,
                            index,
                            &entry.display_name,
                        );
                    }
                });
            add_requested = ui.button("add node to definition").clicked();
            if let Some(source) = self.component_definition.pending_source {
                ui.colored_label(
                    egui::Color32::WHITE,
                    format!("wiring #{}.{}", source.node.get(), source.port.get()),
                );
                if ui.small_button("cancel definition wire").clicked() {
                    self.component_definition.pending_source = None;
                    "pending component-definition wire cancelled"
                        .clone_into(&mut self.component_definition.status);
                }
            }
        });
        if add_requested {
            self.add_component_definition_node(scope);
            ui.label(&self.component_definition.status);
            return;
        }

        let selected_child_label = self
            .component_definition
            .child_component
            .and_then(|child| component.hierarchy.document.dependency(child))
            .map_or_else(
                || "no other library dependency".to_owned(),
                |dependency| {
                    format!(
                        "{} · {}…",
                        dependency.document().name(),
                        digest_prefix(dependency.digest().0)
                    )
                },
            );
        let mut add_child_requested = false;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Child component");
            egui::ComboBox::from_id_salt(("component_definition_child", scope.0))
                .selected_text(selected_child_label)
                .show_ui(ui, |ui| {
                    for dependency in component
                        .hierarchy
                        .document
                        .dependencies()
                        .iter()
                        .filter(|dependency| dependency.digest() != scope)
                    {
                        let digest = dependency.digest();
                        ui.selectable_value(
                            &mut self.component_definition.child_component,
                            Some(digest),
                            format!(
                                "{} · {}…",
                                dependency.document().name(),
                                digest_prefix(digest.0)
                            ),
                        );
                    }
                });
            add_child_requested = ui
                .add_enabled(
                    self.component_definition.child_component.is_some(),
                    egui::Button::new("add child occurrence"),
                )
                .clicked();
            ui.weak("one ALGW placeholder + one scoped ALGH binding");
        });
        if add_child_requested && let Some(child) = self.component_definition.child_component {
            self.add_component_definition_child(scope, child);
            ui.label(&self.component_definition.status);
            return;
        }

        if self.show_component_definition_canvas(ui, scope, &workspace, &presentation) {
            ui.label(&self.component_definition.status);
            return;
        }
        self.show_component_definition_selected_node(ui, scope, &workspace);
        ui.label(&self.component_definition.status);
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the scoped definition canvas keeps node, port, wire, and drag interactions together before one deferred transaction"
    )]
    fn show_component_definition_canvas(
        &mut self,
        ui: &mut egui::Ui,
        scope: Digest,
        workspace: &GraphWorkspaceDocument,
        presentation: &GraphPresentation,
    ) -> bool {
        ui.strong("Canonical selected-definition canvas");
        ui.weak(
            "Drag node headers on the exact integer canvas. Select an output then a typed input to connect; secondary-click an owned input to disconnect. Component-instance placeholders may be wired or relabeled; their dedicated inspector action removes the placeholder and scoped ALGH occurrence atomically.",
        );
        let nodes = workspace.graph().nodes().to_vec();
        let wires = workspace.graph().wires().to_vec();
        let mut clicked_node = None;
        let mut canvas_clicked = false;
        let mut move_request = None;
        let mut port_edit = None;
        egui::ScrollArea::both()
            .id_salt(("selected_component_definition_canvas", scope.0))
            .auto_shrink([false, false])
            .min_scrolled_height(260.0)
            .max_height(360.0)
            .show(ui, |ui| {
                let canvas_size = egui::vec2(
                    presentation.size.x.max(ui.available_width()),
                    presentation.size.y.max(260.0),
                );
                let (canvas, painter) = ui.allocate_painter(canvas_size, egui::Sense::click());
                canvas_clicked = canvas.clicked();
                painter.rect_filled(canvas.rect, 0.0, egui::Color32::from_rgb(17, 21, 29));
                paint_grid(&painter, canvas.rect);
                let origin = canvas.rect.min.to_vec2();
                for wire in &wires {
                    Self::paint_wire(
                        &painter,
                        origin,
                        workspace.graph(),
                        presentation,
                        self.component_definition.selected_node,
                        wire,
                    );
                }
                for node in &nodes {
                    let Some(node_presentation) = presentation.nodes.get(&node.id()) else {
                        continue;
                    };
                    let rect = node_presentation.rect.translate(origin);
                    let header_rect = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.right(), rect.top() + NODE_HEADER_HEIGHT),
                    );
                    let response = ui.interact(
                        header_rect,
                        egui::Id::new((
                            "selected_component_definition_node",
                            scope.0,
                            node.id().get(),
                        )),
                        egui::Sense::click_and_drag(),
                    );
                    if response.drag_started()
                        && let Some(placement) = workspace.placement(node.id())
                    {
                        self.component_definition.drag = Some(NodeDrag {
                            node: node.id(),
                            origin: placement,
                            delta: egui::Vec2::ZERO,
                        });
                    }
                    if response.dragged()
                        && let Some(drag) = self.component_definition.drag.as_mut()
                        && drag.node == node.id()
                    {
                        drag.delta += response.drag_delta();
                    }
                    let dragging = self
                        .component_definition
                        .drag
                        .filter(|drag| drag.node == node.id())
                        .filter(|_| response.dragged() || response.drag_stopped());
                    let painted_rect = dragging.map_or(rect, |drag| rect.translate(drag.delta));
                    if response.drag_stopped()
                        && let Some(drag) = dragging
                    {
                        move_request = Some((drag, drag.delta));
                        self.component_definition.drag = None;
                    }
                    if response.clicked() {
                        clicked_node = Some(node.id());
                    }
                    for (index, port) in node.inputs().iter().enumerate() {
                        let anchor = port_anchor_for_rect(painted_rect, index, false);
                        let response = ui.interact(
                            egui::Rect::from_center_size(anchor, egui::vec2(18.0, 18.0)),
                            egui::Id::new((
                                "selected_component_definition_input",
                                scope.0,
                                node.id().get(),
                                port.id().get(),
                            )),
                            egui::Sense::click(),
                        );
                        let endpoint = WireEndpoint {
                            node: node.id(),
                            port: port.id(),
                        };
                        if response.secondary_clicked() {
                            port_edit = Some(PortEdit::DisconnectInput(endpoint));
                            clicked_node = Some(node.id());
                        } else if response.clicked() {
                            port_edit = Some(PortEdit::ConnectInput(endpoint));
                            clicked_node = Some(node.id());
                        }
                    }
                    for (index, port) in node.outputs().iter().enumerate() {
                        let anchor = port_anchor_for_rect(painted_rect, index, true);
                        let response = ui.interact(
                            egui::Rect::from_center_size(anchor, egui::vec2(18.0, 18.0)),
                            egui::Id::new((
                                "selected_component_definition_output",
                                scope.0,
                                node.id().get(),
                                port.id().get(),
                            )),
                            egui::Sense::click(),
                        );
                        if response.clicked() {
                            port_edit = Some(PortEdit::SelectOutput(WireEndpoint {
                                node: node.id(),
                                port: port.id(),
                            }));
                            clicked_node = Some(node.id());
                        }
                    }
                    self.paint_node(
                        &painter,
                        painted_rect,
                        node,
                        node_presentation.rank,
                        self.component_definition.selected_node,
                    );
                }
                if let Some(source) = self.component_definition.pending_source
                    && let Some(anchor) = port_anchor(workspace.graph(), presentation, source, true)
                    && let Some(pointer) = ui.ctx().pointer_hover_pos()
                {
                    painter.line_segment(
                        [anchor + origin, pointer],
                        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    );
                }
            });
        let interaction_consumed =
            clicked_node.is_some() || move_request.is_some() || port_edit.is_some();
        if let Some((drag, delta)) = move_request {
            return self.commit_component_definition_node_drag(scope, drag, delta);
        }
        if let Some(edit) = port_edit
            && self.handle_component_definition_port_edit(scope, edit)
        {
            return true;
        }
        if let Some(node) = clicked_node {
            self.component_definition.selected_node = Some(node);
        } else if canvas_clicked && !interaction_consumed {
            self.component_definition.selected_node = None;
        }
        false
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the definition inspector presents one exact node and defers each possible canonical mutation until the display borrow ends"
    )]
    fn show_component_definition_selected_node(
        &mut self,
        ui: &mut egui::Ui,
        scope: Digest,
        workspace: &GraphWorkspaceDocument,
    ) {
        let Some(id) = self.component_definition.selected_node else {
            ui.weak("Select a definition node to inspect exact ports and edit its metadata.");
            return;
        };
        let Some(node) = workspace.graph().node(id).cloned() else {
            return;
        };
        let graph = workspace.graph().clone();
        let placement = workspace.placement(id);
        let schema = self
            .fixture
            .registry()
            .semantic_registry()
            .schema(node.kind());
        let state = schema.and_then(alumina_interface_core::graph::NodeSchema::state);
        let domain_choices = schema.map_or_else(Vec::new, |schema| {
            audited_domain_choices(&graph, schema.allowed_domains())
        });
        let maximum_label_bytes = graph.schema().limits().maximum_label_bytes;
        let mut label_text = self
            .component_definition
            .node_label_drafts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| node.label().to_owned());
        let placeholder = node.kind().name() == GRAPH_COMPONENT_INSTANCE_KIND
            && node.kind().version() == GRAPH_COMPONENT_INSTANCE_VERSION;
        let mut delete_requested = false;
        let mut label_request = None;
        let mut domain_request = None;
        let mut parameter_request = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!("definition #{} {}", id.get(), node.label()));
                ui.monospace(format!("{} v{}", node.kind().name(), node.kind().version()));
                if ui.small_button("clear").clicked() {
                    self.component_definition.selected_node = None;
                }
                delete_requested = if placeholder {
                    ui.button("delete child occurrence + wires")
                        .on_hover_text(
                            "Atomically removes this parent-local ALGW placeholder and its scoped ALGH binding. Public connector or panel bindings must be rebound first.",
                        )
                        .clicked()
                } else {
                    ui.button("delete node + wires").clicked()
                };
            });
            (label_request, domain_request) = show_node_identity_editors(
                ui,
                &node,
                &domain_choices,
                maximum_label_bytes,
                &mut label_text,
            );
            if let Some(placement) = placement {
                ui.monospace(format!(
                    "ALGC {}… · canvas = ({}, {}) logical px",
                    digest_prefix(scope.0),
                    placement.x(),
                    placement.y()
                ));
            }
            ui.horizontal_wrapped(|ui| {
                for port in node.inputs() {
                    ui.monospace(port_description(&graph, "in", port));
                }
                for port in node.outputs() {
                    ui.monospace(port_description(&graph, "out", port));
                }
            });
            parameter_request = show_node_parameter_editors(
                ui,
                &graph,
                &node,
                &mut self.component_definition.parameter_drafts,
            );
            if placeholder {
                ui.weak(
                    "Collapsed nested ALGC occurrence · this placeholder and its scoped ALGH binding have one lifecycle",
                );
            } else if let Some(state) = state {
                ui.label(format!(
                    "Explicit state: clock {}, t{}, read-before-write, ≤{} canonical bytes",
                    state.clock().get(),
                    state.value_type().get(),
                    state.declared_storage_bytes()
                ));
            }
        });
        self.component_definition
            .node_label_drafts
            .insert(id, label_text);
        if delete_requested {
            if placeholder {
                self.remove_component_definition_child(scope, id);
            } else {
                self.delete_component_definition_node(scope, id);
            }
        } else if let Some(label) = label_request {
            self.commit_component_definition_node_label(scope, id, &label);
        } else if let Some(domain) = domain_request {
            self.commit_component_definition_node_domain(scope, id, domain);
        } else if let Some((parameter, text)) = parameter_request {
            self.commit_component_definition_parameter_text(scope, id, parameter, &text);
        }
    }

    fn editable_component_definition(
        &self,
        scope: Digest,
    ) -> Result<(ComponentPackage, GraphComponentDocument), String> {
        let current = self
            .component
            .clone()
            .ok_or_else(|| "no selected component hierarchy is attached".to_owned())?;
        if self.component_definition.scope != Some(scope)
            || self.selected_hierarchy_component != Some(scope)
        {
            return Err("the selected component-definition scope changed".to_owned());
        }
        if current.encoding.digest() == scope {
            return Err(
                "the complete-session control ALGC must be edited on the main canvas".to_owned(),
            );
        }
        let document = current
            .hierarchy
            .document
            .dependency(scope)
            .ok_or_else(|| "the selected component definition is unavailable".to_owned())?
            .document()
            .clone();
        Ok((current, document))
    }

    fn reject_component_definition_edit(&mut self, context: &str, error: impl std::fmt::Display) {
        self.component_definition.status =
            format!("component definition {context} rejected without mutation: {error}");
        self.component_status
            .clone_from(&self.component_definition.status);
        self.edit_status
            .clone_from(&self.component_definition.status);
    }

    fn commit_component_definition_workspace(
        &mut self,
        scope: Digest,
        candidate: GraphWorkspaceDocument,
        success: &str,
    ) -> bool {
        let (current, mut document) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("edit", error);
                return false;
            }
        };
        if &candidate == document.workspace() {
            self.component_definition.status =
                format!("{success}; canonical component definition already matched");
            self.component_status
                .clone_from(&self.component_definition.status);
            self.edit_status
                .clone_from(&self.component_definition.status);
            return true;
        }
        if let Err(error) = structural_workspace_presentation(&candidate) {
            self.reject_component_definition_edit("presentation", error);
            return false;
        }
        if let Err(error) = document.replace_workspace(candidate) {
            self.reject_component_definition_edit("workspace replacement", error);
            return false;
        }
        match self.commit_component_dependency_document(
            &current,
            scope,
            document,
            success,
            self.selected_panel_item,
        ) {
            Ok(_) => {
                self.component_definition.scope = self.selected_hierarchy_component;
                self.reconcile_component_definition_editor();
                self.component_definition
                    .status
                    .clone_from(&self.component_status);
                true
            }
            Err(error) => {
                self.reject_component_definition_edit("transaction", error);
                false
            }
        }
    }

    fn add_component_definition_node(&mut self, scope: Digest) {
        let Some(entry) = self
            .palette
            .get(self.component_definition.palette_index)
            .cloned()
        else {
            self.reject_component_definition_edit(
                "node creation",
                "palette selection is unavailable",
            );
            return;
        };
        let (_, document) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("node creation", error);
                return;
            }
        };
        let mut candidate = document.workspace().clone();
        let (x, y) = match new_node_position(&candidate) {
            Ok(position) => position,
            Err(error) => {
                self.reject_component_definition_edit("node creation", error);
                return;
            }
        };
        let id = match candidate.create_node(entry.prototype, x, y) {
            Ok(id) => id,
            Err(error) => {
                self.reject_component_definition_edit("node creation", error);
                return;
            }
        };
        if self.commit_component_definition_workspace(
            scope,
            candidate,
            &format!(
                "created {} as selected-definition node {} at canonical canvas ({x}, {y})",
                entry.display_name,
                id.get()
            ),
        ) {
            self.component_definition.selected_node = Some(id);
            self.component_definition.pending_source = None;
        }
    }

    fn add_component_definition_child(&mut self, scope: Digest, child: Digest) {
        let (current, document) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("child occurrence creation", error);
                return;
            }
        };
        let Some(child_dependency) = current.hierarchy.document.dependency(child) else {
            self.reject_component_definition_edit(
                "child occurrence creation",
                format!("child component {} is unavailable", digest_prefix(child.0)),
            );
            return;
        };
        let child_name = child_dependency.document().name().to_owned();
        let (x, y) = match new_node_position(document.workspace()) {
            Ok(position) => position,
            Err(error) => {
                self.reject_component_definition_edit("child occurrence creation", error);
                return;
            }
        };
        let label = format!(
            "{child_name} occurrence {}",
            document.workspace().next_node_id()
        );
        let mut hierarchy_document = current.hierarchy.document.clone();
        let (node, report) = match hierarchy_document.add_nested_instance(scope, child, label, x, y)
        {
            Ok(result) => result,
            Err(error) => {
                self.reject_component_definition_edit("child occurrence creation", error);
                return;
            }
        };
        let authoritative = current.encoding.digest();
        if report.resolve(authoritative) != authoritative {
            self.reject_component_definition_edit(
                "child occurrence creation",
                "the nested occurrence would recursively rewrite the complete-session control ALGC; coordinated control-workspace replacement is not available",
            );
            return;
        }
        let replacement = report.resolve(scope);
        let retained_child = report.resolve(child);
        let remaps = report.remaps().len();
        let selected_instance = self.selected_hierarchy_instance;
        let status = format!(
            "added selected-definition child occurrence node {} of {child_name} {}… at canonical canvas ({x}, {y}) with one scoped ALGH binding across {remaps} recursive identity remap(s)",
            node.get(),
            digest_prefix(retained_child.0)
        );
        match self.commit_hierarchy_document(
            current,
            hierarchy_document,
            &status,
            Some(replacement),
            selected_instance,
        ) {
            Ok(_) => {
                self.component_definition.scope = Some(replacement);
                self.component_definition.child_component = Some(retained_child);
                self.component_definition.selected_node = Some(node);
                self.component_definition.pending_source = None;
                self.component_definition.drag = None;
                self.component_definition
                    .status
                    .clone_from(&self.component_status);
            }
            Err(error) => {
                self.reject_component_definition_edit("child occurrence transaction", error);
            }
        }
    }

    fn remove_component_definition_child(&mut self, scope: Digest, node: GraphNodeId) {
        let (current, _) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("child occurrence deletion", error);
                return;
            }
        };
        let instance_scope = GraphInstanceScope::Component(scope);
        let Some(instance) = current
            .hierarchy
            .document
            .instances()
            .iter()
            .copied()
            .find(|instance| instance.scope() == instance_scope && instance.node() == node)
        else {
            self.reject_component_definition_edit(
                "child occurrence deletion",
                format!("node {} has no scoped ALGH binding", node.get()),
            );
            return;
        };
        let child = instance.component();
        let child_name = current
            .hierarchy
            .document
            .dependency(child)
            .map_or("unknown child", |dependency| dependency.document().name())
            .to_owned();
        let mut hierarchy_document = current.hierarchy.document.clone();
        let (removed_wires, report) = match hierarchy_document.remove_nested_instance(scope, node) {
            Ok(result) => result,
            Err(error) => {
                self.reject_component_definition_edit("child occurrence deletion", error);
                return;
            }
        };
        let authoritative = current.encoding.digest();
        if report.resolve(authoritative) != authoritative {
            self.reject_component_definition_edit(
                "child occurrence deletion",
                "the nested occurrence removal would recursively rewrite the complete-session control ALGC; coordinated control-workspace replacement is not available",
            );
            return;
        }
        let replacement = report.resolve(scope);
        let retained_child = report.resolve(child);
        let remaps = report.remaps().len();
        let selected_instance = self.selected_hierarchy_instance;
        let status = format!(
            "removed selected-definition child occurrence node {} of {child_name} {}…, {removed_wires} incident wire(s), and its scoped ALGH binding across {remaps} recursive identity remap(s)",
            node.get(),
            digest_prefix(retained_child.0)
        );
        match self.commit_hierarchy_document(
            current,
            hierarchy_document,
            &status,
            Some(replacement),
            selected_instance,
        ) {
            Ok(_) => {
                self.component_definition.scope = Some(replacement);
                self.component_definition.child_component = Some(retained_child);
                self.component_definition.selected_node = None;
                self.component_definition.pending_source = self
                    .component_definition
                    .pending_source
                    .filter(|source| source.node != node);
                self.component_definition.drag = self
                    .component_definition
                    .drag
                    .filter(|drag| drag.node != node);
                self.component_definition
                    .parameter_drafts
                    .retain(|(draft_node, _), _| *draft_node != node);
                self.component_definition.node_label_drafts.remove(&node);
                self.component_definition
                    .status
                    .clone_from(&self.component_status);
            }
            Err(error) => {
                self.reject_component_definition_edit("child occurrence transaction", error);
            }
        }
    }

    fn delete_component_definition_node(&mut self, scope: Digest, id: GraphNodeId) {
        let (_, document) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("node deletion", error);
                return;
            }
        };
        let Some(node) = document.workspace().graph().node(id) else {
            self.reject_component_definition_edit(
                "node deletion",
                format!("node {} is unavailable", id.get()),
            );
            return;
        };
        if node.kind().name() == GRAPH_COMPONENT_INSTANCE_KIND
            && node.kind().version() == GRAPH_COMPONENT_INSTANCE_VERSION
        {
            self.reject_component_definition_edit(
                "node deletion",
                "ALGH owns component-instance placeholders",
            );
            return;
        }
        let mut candidate = document.workspace().clone();
        let removed_wires = match candidate.delete_node(id) {
            Ok(count) => count,
            Err(error) => {
                self.reject_component_definition_edit("node deletion", error);
                return;
            }
        };
        if self.commit_component_definition_workspace(
            scope,
            candidate,
            &format!(
                "deleted selected-definition node {} and {removed_wires} incident wire(s) without reusing identities",
                id.get()
            ),
        ) {
            self.component_definition.selected_node = None;
            self.component_definition.pending_source = self
                .component_definition
                .pending_source
                .filter(|source| source.node != id);
            self.component_definition
                .parameter_drafts
                .retain(|(node, _), _| *node != id);
            self.component_definition.node_label_drafts.remove(&id);
        }
    }

    fn commit_component_definition_node_label(
        &mut self,
        scope: Digest,
        id: GraphNodeId,
        label: &str,
    ) {
        let (_, document) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("node label", error);
                return;
            }
        };
        let mut candidate = document.workspace().clone();
        if let Err(error) = candidate.set_node_label(id, label) {
            self.reject_component_definition_edit("node label", error);
            return;
        }
        if self.commit_component_definition_workspace(
            scope,
            candidate,
            &format!(
                "set selected-definition node {} label to exact UTF-8 {label:?}",
                id.get()
            ),
        ) {
            self.component_definition
                .node_label_drafts
                .insert(id, label.to_owned());
        }
    }

    fn commit_component_definition_node_domain(
        &mut self,
        scope: Digest,
        id: GraphNodeId,
        domain: ExecutionDomain,
    ) {
        let (_, document) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("node domain", error);
                return;
            }
        };
        let mut candidate = document.workspace().clone();
        if let Err(error) = candidate.set_node_domain(id, domain) {
            self.reject_component_definition_edit("node domain", error);
            return;
        }
        self.commit_component_definition_workspace(
            scope,
            candidate,
            &format!(
                "set selected-definition node {} execution placement to {}",
                id.get(),
                domain_choice_label(domain)
            ),
        );
    }

    fn commit_component_definition_parameter_text(
        &mut self,
        scope: Digest,
        id: GraphNodeId,
        parameter_id: u32,
        text: &str,
    ) {
        let (_, document) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("parameter", error);
                return;
            }
        };
        let workspace = document.workspace();
        let Some(parameter) = workspace
            .graph()
            .node(id)
            .and_then(|node| {
                node.parameters()
                    .iter()
                    .find(|value| value.id() == parameter_id)
            })
            .cloned()
        else {
            self.reject_component_definition_edit(
                "parameter",
                format!("node {} parameter {parameter_id} is unavailable", id.get()),
            );
            return;
        };
        let value =
            match parse_parameter_text(workspace.graph(), parameter.value().value_type(), text) {
                Ok(value) => value,
                Err(error) => {
                    self.reject_component_definition_edit("parameter", error);
                    return;
                }
            };
        let canonical =
            parameter_edit_text(workspace.graph(), &value).unwrap_or_else(|| text.to_owned());
        let mut candidate = workspace.clone();
        if let Err(error) = candidate.set_parameter(id, parameter_id, value) {
            self.reject_component_definition_edit("parameter", error);
            return;
        }
        if self.commit_component_definition_workspace(
            scope,
            candidate,
            &format!(
                "set selected-definition node {} parameter {} to exact canonical {canonical}",
                id.get(),
                parameter.name()
            ),
        ) {
            self.component_definition
                .parameter_drafts
                .insert((id, parameter_id), canonical);
        }
    }

    fn commit_component_definition_node_drag(
        &mut self,
        scope: Digest,
        drag: NodeDrag,
        delta: egui::Vec2,
    ) -> bool {
        let (x, y) = match (
            quantized_canvas_coordinate(drag.origin.x(), delta.x),
            quantized_canvas_coordinate(drag.origin.y(), delta.y),
        ) {
            (Ok(x), Ok(y)) => (x, y),
            (Err(error), _) | (_, Err(error)) => {
                self.reject_component_definition_edit("node move", error);
                return false;
            }
        };
        if x == drag.origin.x() && y == drag.origin.y() {
            self.component_definition.status = format!(
                "selected-definition node {} already matched canonical canvas ({x}, {y})",
                drag.node.get()
            );
            self.component_status
                .clone_from(&self.component_definition.status);
            self.edit_status
                .clone_from(&self.component_definition.status);
            return true;
        }
        let (_, document) = match self.editable_component_definition(scope) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.reject_component_definition_edit("node move", error);
                return false;
            }
        };
        let mut candidate = document.workspace().clone();
        if let Err(error) = candidate.move_node(drag.node, x, y) {
            self.reject_component_definition_edit("node move", error);
            return false;
        }
        self.commit_component_definition_workspace(
            scope,
            candidate,
            &format!(
                "moved selected-definition node {} to canonical canvas ({x}, {y})",
                drag.node.get()
            ),
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "typed output selection plus transactional connect/disconnect share one scoped pending-wire state"
    )]
    fn handle_component_definition_port_edit(&mut self, scope: Digest, edit: PortEdit) -> bool {
        match edit {
            PortEdit::SelectOutput(source) => {
                if self.component_definition.pending_source == Some(source) {
                    self.component_definition.pending_source = None;
                    "pending component-definition wire cancelled"
                        .clone_into(&mut self.component_definition.status);
                } else {
                    self.component_definition.pending_source = Some(source);
                    self.component_definition.status = format!(
                        "selected definition output #{}.{}; choose one typed input",
                        source.node.get(),
                        source.port.get()
                    );
                }
                false
            }
            PortEdit::ConnectInput(target) => {
                let Some(source) = self.component_definition.pending_source else {
                    self.component_definition.status = format!(
                        "definition input #{}.{} selected; choose an output first",
                        target.node.get(),
                        target.port.get()
                    );
                    return false;
                };
                let (_, document) = match self.editable_component_definition(scope) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        self.reject_component_definition_edit("wire", error);
                        return false;
                    }
                };
                let mut candidate = document.workspace().clone();
                let id = match candidate.connect(source, target) {
                    Ok(id) => id,
                    Err(error) => {
                        self.reject_component_definition_edit("wire", error);
                        return false;
                    }
                };
                let changed = self.commit_component_definition_workspace(
                    scope,
                    candidate,
                    &format!(
                        "connected selected-definition wire {} from #{}.{} to #{}.{}",
                        id.get(),
                        source.node.get(),
                        source.port.get(),
                        target.node.get(),
                        target.port.get()
                    ),
                );
                if changed {
                    self.component_definition.pending_source = None;
                }
                changed
            }
            PortEdit::DisconnectInput(target) => {
                let (_, document) = match self.editable_component_definition(scope) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        self.reject_component_definition_edit("wire removal", error);
                        return false;
                    }
                };
                let Some(id) = document
                    .workspace()
                    .graph()
                    .wires()
                    .iter()
                    .find(|wire| wire.target() == target)
                    .map(|wire| wire.id())
                else {
                    self.component_definition.status = format!(
                        "definition input #{}.{} is already disconnected",
                        target.node.get(),
                        target.port.get()
                    );
                    return false;
                };
                let mut candidate = document.workspace().clone();
                if let Err(error) = candidate.disconnect(id) {
                    self.reject_component_definition_edit("wire removal", error);
                    return false;
                }
                let changed = self.commit_component_definition_workspace(
                    scope,
                    candidate,
                    &format!(
                        "disconnected selected-definition wire {} from input #{}.{}",
                        id.get(),
                        target.node.get(),
                        target.port.get()
                    ),
                );
                if changed {
                    self.component_definition.pending_source = None;
                }
                changed
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "candidate hierarchy mutation, ALGH/ALGM regeneration, complete-session admission, history recording, and commit remain one auditable transaction"
    )]
    fn apply_hierarchy_action(&mut self, action: HierarchyUiAction) {
        let Some(current) = self.component.clone() else {
            "hierarchy edit rejected without mutation: no selected component hierarchy is attached"
                .clone_into(&mut self.component_status);
            return;
        };
        let mut document = current.hierarchy.document.clone();
        let clears_pending_source = matches!(
            action,
            HierarchyUiAction::ConnectRoot { .. } | HierarchyUiAction::DisconnectRoot(_)
        );
        let clears_new_component_name = action == HierarchyUiAction::CreateComponent;
        let (status, selected_component, selected_instance) = match action {
            HierarchyUiAction::CreateComponent => {
                let name = self.new_library_component_name.clone();
                let component = match empty_library_component(&name, &self.workspace) {
                    Ok(component) => component,
                    Err(error) => {
                        self.component_status =
                            format!("empty library component rejected without mutation: {error}");
                        return;
                    }
                };
                let encoding = match encode_graph_component(&component) {
                    Ok(encoding) => encoding,
                    Err(error) => {
                        self.component_status =
                            format!("empty library component rejected without mutation: {error}");
                        return;
                    }
                };
                let digest = encoding.digest();
                if let Some(existing) = document.dependencies().iter().find(|dependency| {
                    dependency.document().name() == name && dependency.digest() != digest
                }) {
                    self.component_status = format!(
                        "empty library component rejected without mutation: stable name {name} already belongs to ALGC {}…",
                        digest_prefix(existing.digest().0)
                    );
                    return;
                }
                let already_present = document.dependency(digest).is_some();
                if let Err(error) = document.add_component(component) {
                    self.component_status =
                        format!("empty library component rejected without mutation: {error}");
                    return;
                }
                (
                    format!(
                        "{} deterministic empty library component {name} ALGC {}… with shared exact schema/clocks and no execution authority",
                        if already_present {
                            "selected existing"
                        } else {
                            "created"
                        },
                        digest_prefix(digest.0)
                    ),
                    Some(digest),
                    self.selected_hierarchy_instance,
                )
            }
            HierarchyUiAction::AddRoot(component) => {
                let maximum_x = document
                    .root()
                    .placements()
                    .iter()
                    .map(|placement| placement.x())
                    .max();
                let x = maximum_x.map_or(Ok(NEW_NODE_ORIGIN), |maximum_x| {
                    maximum_x
                        .checked_add(NEW_NODE_X_GAP)
                        .ok_or_else(|| "root instance x coordinate overflowed".to_owned())
                });
                let x = match x {
                    Ok(x) => x,
                    Err(error) => {
                        self.component_status =
                            format!("hierarchy edit rejected without mutation: {error}");
                        return;
                    }
                };
                let label = format!("Component instance {}", document.root().next_node_id());
                let node = match document.add_root_instance(component, label, x, NEW_NODE_ORIGIN) {
                    Ok(node) => node,
                    Err(error) => {
                        self.component_status =
                            format!("hierarchy edit rejected without mutation: {error}");
                        return;
                    }
                };
                (
                    format!(
                        "added root instance #{} of exact ALGC {}…",
                        node.get(),
                        digest_prefix(component.0)
                    ),
                    Some(component),
                    Some(node),
                )
            }
            HierarchyUiAction::RemoveRoot(node) => {
                let removed_wires = match document.remove_root_instance(node) {
                    Ok(removed_wires) => removed_wires,
                    Err(error) => {
                        self.component_status =
                            format!("hierarchy edit rejected without mutation: {error}");
                        return;
                    }
                };
                (
                    format!(
                        "removed root instance #{} and {removed_wires} incident root wire(s)",
                        node.get()
                    ),
                    self.selected_hierarchy_component,
                    None,
                )
            }
            HierarchyUiAction::RemoveComponent(component) => {
                if component == current.encoding.digest() {
                    "hierarchy edit rejected without mutation: the selected ALGC is complete-session control authority"
                        .clone_into(&mut self.component_status);
                    return;
                }
                if let Err(error) = document.remove_component(component) {
                    self.component_status =
                        format!("hierarchy edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!(
                        "removed unreferenced library ALGC {}…",
                        digest_prefix(component.0)
                    ),
                    None,
                    self.selected_hierarchy_instance,
                )
            }
            HierarchyUiAction::MoveRoot { node, x, y } => {
                if let Err(error) = document.move_root_instance(node, x, y) {
                    self.component_status =
                        format!("hierarchy edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!(
                        "moved root instance #{} to canonical canvas ({x}, {y})",
                        node.get()
                    ),
                    self.selected_hierarchy_component,
                    Some(node),
                )
            }
            HierarchyUiAction::ConnectRoot { source, target } => {
                let wire = match document.connect_root_wire(source, target) {
                    Ok(wire) => wire,
                    Err(error) => {
                        self.component_status =
                            format!("hierarchy edit rejected without mutation: {error}");
                        return;
                    }
                };
                (
                    format!(
                        "connected root wire {} from #{}.{} to #{}.{}",
                        wire.get(),
                        source.node.get(),
                        source.port.get(),
                        target.node.get(),
                        target.port.get()
                    ),
                    self.selected_hierarchy_component,
                    Some(target.node),
                )
            }
            HierarchyUiAction::DisconnectRoot(wire) => {
                if let Err(error) = document.disconnect_root_wire(wire) {
                    self.component_status =
                        format!("hierarchy edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!("disconnected root wire {}", wire.get()),
                    self.selected_hierarchy_component,
                    self.selected_hierarchy_instance,
                )
            }
        };
        match self.commit_hierarchy_document(
            current,
            document,
            &status,
            selected_component,
            selected_instance,
        ) {
            Ok(_) => {
                if clears_pending_source {
                    self.pending_hierarchy_source = None;
                }
                if clears_new_component_name {
                    self.new_library_component_name.clear();
                }
            }
            Err(error) => {
                self.component_status =
                    format!("hierarchy edit rejected without mutation: {error}");
            }
        }
    }

    fn commit_hierarchy_document(
        &mut self,
        current: ComponentPackage,
        document: GraphHierarchyDocument,
        status: &str,
        selected_component: Option<Digest>,
        selected_instance: Option<GraphNodeId>,
    ) -> Result<bool, String> {
        if document == current.hierarchy.document {
            self.selected_hierarchy_component = selected_component;
            self.selected_hierarchy_instance = selected_instance;
            self.reconcile_hierarchy_selection(None);
            self.component_status = format!("{status}; canonical hierarchy already matched");
            self.edit_status.clone_from(&self.component_status);
            return Ok(false);
        }
        let hierarchy = hierarchy_package(document, &self.fixture)?;
        let candidate = ComponentPackage {
            document: current.document,
            encoding: current.encoding,
            hierarchy,
        };
        let Some(probes) = self.probes.as_ref() else {
            return Err("canonical ALGP sidecar is unavailable".to_owned());
        };
        Self::authoring_session_document_from_parts(
            &self.workspace,
            &probes.document,
            &self.cached_jobs.workspace,
            Some(&candidate),
        )
        .map_err(|error| format!("complete-session candidate rejected: {error}"))?;
        let history = self.history_with_current_recorded()?;
        self.component = Some(candidate);
        self.history = history;
        self.selected_hierarchy_component = selected_component;
        self.selected_hierarchy_instance = selected_instance;
        self.reconcile_hierarchy_selection(None);
        self.reconcile_component_definition_editor();
        self.persistence_dirty = true;
        self.persistence_attempted = false;
        self.component_status = format!("{status}; complete ALGS history recorded");
        self.edit_status.clone_from(&self.component_status);
        Ok(true)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "six connector CRUD variants converge on one atomic ALGC/ALGH/ALGM/ALGS commit boundary"
    )]
    fn apply_component_connector_action(&mut self, action: ComponentConnectorUiAction) {
        let Some(current) = self.component.clone() else {
            "connector-pane edit rejected without mutation: no selected component is attached"
                .clone_into(&mut self.component_status);
            return;
        };
        let Some(original) = self.component_connector_scope else {
            "connector-pane edit rejected without mutation: no hierarchy library component is selected"
                .clone_into(&mut self.component_status);
            return;
        };
        let Some(dependency) = current.hierarchy.document.dependency(original) else {
            "connector-pane edit rejected without mutation: selected hierarchy library component is unavailable"
                .clone_into(&mut self.component_status);
            return;
        };
        let mut document = dependency.document().clone();
        let (status, preferred, clears_input, clears_output) = match action {
            ComponentConnectorUiAction::AddInput { name, target } => {
                let input = match document.add_input(name, target) {
                    Ok(input) => input,
                    Err(error) => {
                        self.component_status =
                            format!("connector-pane edit rejected without mutation: {error}");
                        return;
                    }
                };
                (
                    format!(
                        "added stable public input #{} at internal endpoint #{}.{}",
                        input.get(),
                        target.node.get(),
                        target.port.get()
                    ),
                    Some(ComponentConnectorSelection::Input(input)),
                    true,
                    false,
                )
            }
            ComponentConnectorUiAction::UpdateInput { input, draft } => {
                if let Err(error) = document.update_input(input, draft.name, draft.endpoint) {
                    self.component_status =
                        format!("connector-pane edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!("updated stable public input #{}", input.get()),
                    Some(ComponentConnectorSelection::Input(input)),
                    false,
                    false,
                )
            }
            ComponentConnectorUiAction::RemoveInput(input) => {
                if let Err(error) = document.remove_input(input) {
                    self.component_status =
                        format!("connector-pane edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!("removed stable public input #{}", input.get()),
                    None,
                    false,
                    false,
                )
            }
            ComponentConnectorUiAction::AddOutput { name, source } => {
                let output = match document.add_output(name, source) {
                    Ok(output) => output,
                    Err(error) => {
                        self.component_status =
                            format!("connector-pane edit rejected without mutation: {error}");
                        return;
                    }
                };
                (
                    format!(
                        "added stable public output #{} at internal endpoint #{}.{}",
                        output.get(),
                        source.node.get(),
                        source.port.get()
                    ),
                    Some(ComponentConnectorSelection::Output(output)),
                    false,
                    true,
                )
            }
            ComponentConnectorUiAction::UpdateOutput { output, draft } => {
                if let Err(error) = document.update_output(output, draft.name, draft.endpoint) {
                    self.component_status =
                        format!("connector-pane edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!("updated stable public output #{}", output.get()),
                    Some(ComponentConnectorSelection::Output(output)),
                    false,
                    false,
                )
            }
            ComponentConnectorUiAction::RemoveOutput(output) => {
                if let Err(error) = document.remove_output(output) {
                    self.component_status =
                        format!("connector-pane edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!("removed stable public output #{}", output.get()),
                    None,
                    false,
                    false,
                )
            }
        };
        match self.commit_component_dependency_document(
            &current,
            original,
            document,
            &status,
            self.selected_panel_item,
        ) {
            Ok(_) => {
                if clears_input {
                    self.new_component_input_name.clear();
                    self.new_component_input_target = None;
                }
                if clears_output {
                    self.new_component_output_name.clear();
                    self.new_component_output_source = None;
                }
                self.reconcile_component_connector_selection(preferred);
            }
            Err(error) => {
                self.component_status =
                    format!("connector-pane edit rejected without mutation: {error}");
            }
        }
    }

    fn apply_panel_action(&mut self, action: PanelUiAction) {
        let Some(current) = self.component.clone() else {
            "front-panel edit rejected without mutation: no selected component is attached"
                .clone_into(&mut self.component_status);
            return;
        };
        let mut document = current.document.clone();
        let (status, selected, clears_new_item) = match action {
            PanelUiAction::Add {
                name,
                binding,
                rect,
            } => {
                let item = match document.add_panel_item(name, binding, rect) {
                    Ok(item) => item,
                    Err(error) => {
                        self.component_status =
                            format!("front-panel edit rejected without mutation: {error}");
                        return;
                    }
                };
                (
                    format!(
                        "added front-panel item #{} bound to {}",
                        item.get(),
                        panel_binding_label(&document, binding)
                    ),
                    Some(item),
                    true,
                )
            }
            PanelUiAction::Update { item, draft } => {
                let rect = draft.rect();
                if let Err(error) =
                    document.update_panel_item(item, draft.name, draft.binding, rect)
                {
                    self.component_status =
                        format!("front-panel edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!("updated exact front-panel item #{}", item.get()),
                    Some(item),
                    false,
                )
            }
            PanelUiAction::Remove(item) => {
                if let Err(error) = document.remove_panel_item(item) {
                    self.component_status =
                        format!("front-panel edit rejected without mutation: {error}");
                    return;
                }
                (
                    format!("removed front-panel item #{}", item.get()),
                    None,
                    false,
                )
            }
        };
        match self.commit_component_document(&current, document, &status, selected) {
            Ok(_) if clears_new_item => {
                self.new_panel_item_name.clear();
                self.new_panel_binding = None;
            }
            Ok(_) => {}
            Err(error) => {
                self.component_status =
                    format!("front-panel edit rejected without mutation: {error}");
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "ALGC replacement, recursive hierarchy remap, flattening, complete-session admission, history, and UI reconciliation are one transaction"
    )]
    fn commit_component_document(
        &mut self,
        current: &ComponentPackage,
        document: GraphComponentDocument,
        status: &str,
        selected_panel_item: Option<GraphFrontPanelItemId>,
    ) -> Result<bool, String> {
        self.commit_component_dependency_document(
            current,
            current.encoding.digest(),
            document,
            status,
            selected_panel_item,
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "arbitrary ALGC replacement, recursive hierarchy remap, control-authority preservation, flattening, complete-session admission, history, and UI reconciliation are one transaction"
    )]
    fn commit_component_dependency_document(
        &mut self,
        current: &ComponentPackage,
        original: Digest,
        document: GraphComponentDocument,
        status: &str,
        selected_panel_item: Option<GraphFrontPanelItemId>,
    ) -> Result<bool, String> {
        let dependency = current
            .hierarchy
            .document
            .dependency(original)
            .ok_or_else(|| "selected hierarchy library component is unavailable".to_owned())?;
        if &document == dependency.document() {
            self.reconcile_hierarchy_selection(None);
            self.reconcile_component_connector_selection(self.selected_component_connector);
            self.reconcile_panel_selection(selected_panel_item);
            self.component_status =
                format!("{status}; canonical hierarchy library component already matched");
            self.edit_status.clone_from(&self.component_status);
            return Ok(false);
        }
        let encoding = encode_graph_component(&document).map_err(|error| error.to_string())?;
        let replacement = encoding.digest();
        let mut hierarchy_document = current.hierarchy.document.clone();
        let report = hierarchy_document
            .replace_component_with_report(original, document.clone())
            .map_err(|error| error.to_string())?;
        if report.requested_replacement() != replacement {
            return Err("hierarchy replacement returned a foreign ALGC identity".to_owned());
        }
        let authoritative_original = current.encoding.digest();
        let authoritative_replacement = report.resolve(authoritative_original);
        if original != authoritative_original && authoritative_replacement != authoritative_original
        {
            return Err(
                "library edit would recursively rewrite the complete-session control ALGC; coordinated control-workspace replacement is not available"
                    .to_owned(),
            );
        }
        let hierarchy = hierarchy_package(hierarchy_document, &self.fixture)?;
        let (selected_document, selected_encoding) = if original == authoritative_original {
            (document, encoding)
        } else {
            (current.document.clone(), current.encoding.clone())
        };
        let candidate = ComponentPackage {
            document: selected_document,
            encoding: selected_encoding,
            hierarchy,
        };
        let Some(probes) = self.probes.as_ref() else {
            return Err("canonical ALGP sidecar is unavailable".to_owned());
        };
        Self::authoring_session_document_from_parts(
            &self.workspace,
            &probes.document,
            &self.cached_jobs.workspace,
            Some(&candidate),
        )
        .map_err(|error| format!("complete-session candidate rejected: {error}"))?;
        let history = self.history_with_current_recorded()?;
        self.component = Some(candidate);
        self.history = history;
        self.panel_item_drafts.clear();
        self.component_connector_drafts.clear();
        self.component_definition.scope = self
            .component_definition
            .scope
            .map(|scope| report.resolve(scope));
        self.selected_hierarchy_component = self
            .selected_hierarchy_component
            .map(|selected| report.resolve(selected));
        self.reconcile_hierarchy_selection(None);
        self.reconcile_component_definition_editor();
        self.reconcile_component_connector_selection(None);
        self.reconcile_panel_selection(selected_panel_item);
        self.persistence_dirty = true;
        self.persistence_attempted = false;
        self.component_status = format!(
            "{status}; exact selected library ALGC {}… and complete ALGS history recorded across {} recursive identity remap(s)",
            digest_prefix(replacement.0),
            report.remaps().len()
        );
        self.edit_status.clone_from(&self.component_status);
        Ok(true)
    }

    fn handle_component_file_event(&mut self, event: BoundedFileEvent) {
        match event {
            BoundedFileEvent::Import(Ok(bytes)) => match self.import_component_dependency(&bytes) {
                Ok(_) => self.file_status.clone_from(&self.component_status),
                Err(error) => {
                    self.file_status =
                        format!("ALGC import rejected without authoring mutation: {error}");
                    self.component_status.clone_from(&self.file_status);
                }
            },
            BoundedFileEvent::Import(Err(error)) => {
                self.file_status = format!("ALGC file read rejected: {error}");
            }
            BoundedFileEvent::Export(Ok(bytes)) => {
                self.file_status = format!("exported {bytes} exact selected ALGC dependency bytes");
            }
            BoundedFileEvent::Export(Err(error)) => {
                self.file_status = format!("ALGC export failed: {error}");
            }
        }
    }

    fn import_component_dependency(&mut self, bytes: &[u8]) -> Result<bool, String> {
        let replay = replay_graph_component(
            bytes,
            GraphComponentLimits::interactive(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .map_err(|error| error.to_string())?;
        analyze_graph_draft(
            replay.document().workspace().graph(),
            self.fixture.registry().semantic_registry(),
        )
        .map_err(|error| format!("imported ALGC graph semantics rejected: {error}"))?;
        let digest = replay.encoding().digest();
        let Some(current) = self.component.clone() else {
            return Err("no selected component hierarchy is attached".to_owned());
        };
        let mut document = current.hierarchy.document.clone();
        let admitted = document
            .add_component(replay.into_document())
            .map_err(|error| error.to_string())?;
        if admitted != digest {
            return Err("component-library admission returned a foreign ALGC identity".to_owned());
        }
        let status = format!(
            "imported {} canonical ALGC bytes as library dependency {}…",
            bytes.len(),
            digest_prefix(digest.0)
        );
        self.commit_hierarchy_document(
            current,
            document,
            &status,
            Some(digest),
            self.selected_hierarchy_instance,
        )
    }

    fn reconcile_hierarchy_selection(&mut self, component_remap: Option<(Digest, Digest)>) {
        let Some(component) = self.component.as_ref() else {
            self.selected_hierarchy_component = None;
            self.selected_hierarchy_instance = None;
            self.pending_hierarchy_source = None;
            self.hierarchy_drag = None;
            self.hierarchy_source_browser
                .reset("no complete ALGH/ALGM source authority is attached");
            return;
        };
        let mut selected_component = self.selected_hierarchy_component;
        if let Some((original, replacement)) = component_remap
            && selected_component == Some(original)
        {
            selected_component = Some(replacement);
        }
        if selected_component
            .is_none_or(|digest| component.hierarchy.document.dependency(digest).is_none())
        {
            selected_component = component
                .hierarchy
                .document
                .dependency(component.encoding.digest())
                .map(GraphHierarchyDependency::digest)
                .or_else(|| {
                    component
                        .hierarchy
                        .document
                        .dependencies()
                        .first()
                        .map(GraphHierarchyDependency::digest)
                });
        }
        let mut selected_instance = self.selected_hierarchy_instance;
        if selected_instance.is_none_or(|node| {
            !component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Root && instance.node() == node
                })
        }) {
            selected_instance = component
                .hierarchy
                .document
                .instances()
                .iter()
                .find(|instance| instance.scope() == GraphInstanceScope::Root)
                .map(|instance| instance.node());
        }
        self.selected_hierarchy_component = selected_component;
        self.selected_hierarchy_instance = selected_instance;
        if self.pending_hierarchy_source.is_some_and(|source| {
            component
                .hierarchy
                .document
                .root()
                .graph()
                .node(source.node)
                .is_none_or(|node| node.outputs().iter().all(|port| port.id() != source.port))
        }) {
            self.pending_hierarchy_source = None;
        }
        if self.hierarchy_drag.is_some_and(|drag| {
            !component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Root && instance.node() == drag.node
                })
        }) {
            self.hierarchy_drag = None;
        }
        self.reconcile_hierarchy_source_browser();
    }

    fn reconcile_hierarchy_source_browser(&mut self) {
        let Some(component) = self.component.as_ref() else {
            self.hierarchy_source_browser
                .reset("no complete ALGH/ALGM source authority is attached");
            return;
        };
        let nodes = component
            .hierarchy
            .flattening
            .node_provenance()
            .iter()
            .map(|mapping| (mapping.flattened_node(), mapping.origin().clone()))
            .collect::<Vec<_>>();
        let wires = component
            .hierarchy
            .flattening
            .wire_provenance()
            .iter()
            .map(|mapping| (mapping.flattened_wire(), mapping.origin().clone()))
            .collect::<Vec<_>>();
        self.hierarchy_source_browser.selected_node = self
            .hierarchy_source_browser
            .selected_node
            .filter(|selected| nodes.iter().any(|(node, _)| node == selected))
            .or_else(|| nodes.first().map(|(node, _)| *node));
        self.hierarchy_source_browser.selected_wire = self
            .hierarchy_source_browser
            .selected_wire
            .filter(|selected| wires.iter().any(|(wire, _)| wire == selected))
            .or_else(|| wires.first().map(|(wire, _)| *wire));
        let had_opened = self.hierarchy_source_browser.last_opened.is_some();
        let retained = self
            .hierarchy_source_browser
            .last_opened
            .take()
            .filter(|selection| match selection {
                HierarchySourceSelection::Node { flattened, origin } => nodes
                    .iter()
                    .any(|(node, retained)| node == flattened && retained == origin),
                HierarchySourceSelection::Wire { flattened, origin } => wires
                    .iter()
                    .any(|(wire, retained)| wire == flattened && retained == origin),
            });
        if retained.is_none() && had_opened {
            "the previously opened ALGM source is absent from the current exact hierarchy"
                .clone_into(&mut self.hierarchy_source_browser.status);
            self.hierarchy_source_browser.scroll_pending = false;
        }
        self.hierarchy_source_browser.last_opened = retained;
    }

    fn reconcile_component_definition_editor(&mut self) {
        let snapshot = self.component.as_ref().and_then(|component| {
            let scope = self.selected_hierarchy_component?;
            let dependency = component.hierarchy.document.dependency(scope)?;
            Some((
                scope,
                scope == component.encoding.digest(),
                dependency.document().name().to_owned(),
                dependency.document().workspace().clone(),
                component
                    .hierarchy
                    .document
                    .dependencies()
                    .iter()
                    .map(GraphHierarchyDependency::digest)
                    .filter(|digest| *digest != scope)
                    .collect::<Vec<_>>(),
            ))
        });
        let Some((scope, authoritative, name, workspace, child_components)) = snapshot else {
            if self.component_definition.scope.is_some() {
                self.component_definition.reset_scope(
                    None,
                    "no exact ALGH library dependency is selected for definition authoring"
                        .to_owned(),
                );
            }
            return;
        };
        if self.component_definition.scope != Some(scope) {
            let status = if authoritative {
                format!(
                    "{name} is the complete-session control authority; edit it on the main exact-control canvas"
                )
            } else {
                format!(
                    "selected exact {name} definition {}…; no definition edit has been proposed",
                    digest_prefix(scope.0)
                )
            };
            self.component_definition.reset_scope(Some(scope), status);
        }
        self.component_definition.child_component = self
            .component_definition
            .child_component
            .filter(|selected| child_components.contains(selected))
            .or_else(|| child_components.first().copied());
        let graph = workspace.graph();
        self.component_definition.selected_node = self
            .component_definition
            .selected_node
            .filter(|node| graph.node(*node).is_some());
        self.component_definition.pending_source =
            self.component_definition.pending_source.filter(|source| {
                graph
                    .node(source.node)
                    .is_some_and(|node| node.outputs().iter().any(|port| port.id() == source.port))
            });
        if self
            .component_definition
            .drag
            .is_some_and(|drag| graph.node(drag.node).is_none())
        {
            self.component_definition.drag = None;
        }
        self.component_definition
            .parameter_drafts
            .retain(|(node, _), _| graph.node(*node).is_some());
        self.component_definition
            .node_label_drafts
            .retain(|node, _| graph.node(*node).is_some());
    }

    fn reconcile_panel_selection(&mut self, preferred: Option<GraphFrontPanelItemId>) {
        let Some(component) = self.component.as_ref() else {
            self.selected_panel_item = None;
            self.panel_item_drafts.clear();
            self.panel_drag = None;
            self.new_panel_binding = None;
            return;
        };
        let items = component.document.panel_items();
        self.panel_item_drafts.retain(|item, _| {
            items
                .binary_search_by_key(item, GraphFrontPanelItem::id)
                .is_ok()
        });
        let preferred = preferred.filter(|item| {
            items
                .binary_search_by_key(item, GraphFrontPanelItem::id)
                .is_ok()
        });
        self.selected_panel_item = preferred
            .or_else(|| {
                self.selected_panel_item.filter(|item| {
                    items
                        .binary_search_by_key(item, GraphFrontPanelItem::id)
                        .is_ok()
                })
            })
            .or_else(|| items.first().map(GraphFrontPanelItem::id));
        if self.panel_drag.is_some_and(|drag| {
            items
                .binary_search_by_key(&drag.item, GraphFrontPanelItem::id)
                .is_err()
        }) {
            self.panel_drag = None;
        }
    }

    fn reconcile_component_connector_selection(
        &mut self,
        preferred: Option<ComponentConnectorSelection>,
    ) {
        let Some((scope, document)) = self.component.as_ref().and_then(|component| {
            let scope = self
                .selected_hierarchy_component
                .filter(|digest| component.hierarchy.document.dependency(*digest).is_some())
                .unwrap_or_else(|| component.encoding.digest());
            component
                .hierarchy
                .document
                .dependency(scope)
                .map(|dependency| (scope, dependency.document().clone()))
        }) else {
            self.component_connector_scope = None;
            self.selected_component_connector = None;
            self.component_connector_drafts.clear();
            self.new_component_input_target = None;
            self.new_component_output_source = None;
            return;
        };
        if self.component_connector_scope != Some(scope) {
            self.component_connector_scope = Some(scope);
            self.selected_component_connector = None;
            self.component_connector_drafts.clear();
            self.new_component_input_name.clear();
            self.new_component_input_target = None;
            self.new_component_output_name.clear();
            self.new_component_output_source = None;
        }
        let connectors = document
            .inputs()
            .iter()
            .map(|input| ComponentConnectorSelection::Input(input.id()))
            .chain(
                document
                    .outputs()
                    .iter()
                    .map(|output| ComponentConnectorSelection::Output(output.id())),
            )
            .collect::<BTreeSet<_>>();
        self.component_connector_drafts
            .retain(|connector, _| connectors.contains(connector));
        self.selected_component_connector = preferred
            .filter(|connector| connectors.contains(connector))
            .or_else(|| {
                self.selected_component_connector
                    .filter(|connector| connectors.contains(connector))
            })
            .or_else(|| connectors.first().copied());

        let input_choices = component_input_endpoint_choices(&document, None);
        if self.new_component_input_target.is_none_or(|target| {
            input_choices
                .iter()
                .all(|(candidate, _)| *candidate != target)
        }) {
            self.new_component_input_target = input_choices.first().map(|(endpoint, _)| *endpoint);
        }
        let output_choices = component_output_endpoint_choices(&document, None);
        if self.new_component_output_source.is_none_or(|source| {
            output_choices
                .iter()
                .all(|(candidate, _)| *candidate != source)
        }) {
            self.new_component_output_source =
                output_choices.first().map(|(endpoint, _)| *endpoint);
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "input/output creation, stable selection, endpoint drafts, and one deferred canonical action form one connector-pane editor"
    )]
    fn show_component_connector_authoring(
        &mut self,
        ui: &mut egui::Ui,
        component: &ComponentPackage,
    ) -> Option<ComponentConnectorUiAction> {
        let scope = self.component_connector_scope?;
        let dependency = component.hierarchy.document.dependency(scope)?;
        let document = dependency.document().clone();
        ui.separator();
        ui.strong("Canonical public connector authoring");
        ui.weak(
            "Edit the selected ALGH library dependency. Expose an unowned internal input or any unexposed internal output under a fresh monotonic identity. Stable connector IDs, never positional ports, govern recursive ALGC placeholder and wire remapping through ALGH, ALGM, and complete ALGS history.",
        );
        ui.monospace(format!(
            "{} · {}… · {} inputs / {} outputs · next IDs {} / {}{}",
            document.name(),
            digest_prefix(scope.0),
            document.inputs().len(),
            document.outputs().len(),
            document.next_input_id(),
            document.next_output_id(),
            if scope == component.encoding.digest() {
                " · control authority"
            } else {
                " · library dependency"
            },
        ));

        let input_choices = component_input_endpoint_choices(&document, None);
        if self.new_component_input_target.is_none_or(|target| {
            input_choices
                .iter()
                .all(|(candidate, _)| *candidate != target)
        }) {
            self.new_component_input_target = input_choices.first().map(|(endpoint, _)| *endpoint);
        }
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("new public input name");
            ui.add(
                egui::TextEdit::singleline(&mut self.new_component_input_name)
                    .char_limit(64)
                    .desired_width(180.0),
            );
            let selected = endpoint_choice_selected_label(
                self.new_component_input_target,
                &input_choices,
                "no unowned internal input",
            );
            egui::ComboBox::from_id_salt("new_component_input_target")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (endpoint, label) in &input_choices {
                        ui.selectable_value(
                            &mut self.new_component_input_target,
                            Some(*endpoint),
                            label,
                        );
                    }
                });
            if ui
                .add_enabled(
                    !self.new_component_input_name.is_empty()
                        && self.new_component_input_target.is_some(),
                    egui::Button::new("add public input"),
                )
                .clicked()
                && let Some(target) = self.new_component_input_target
            {
                action = Some(ComponentConnectorUiAction::AddInput {
                    name: self.new_component_input_name.clone(),
                    target,
                });
            }
        });

        let output_choices = component_output_endpoint_choices(&document, None);
        if self.new_component_output_source.is_none_or(|source| {
            output_choices
                .iter()
                .all(|(candidate, _)| *candidate != source)
        }) {
            self.new_component_output_source =
                output_choices.first().map(|(endpoint, _)| *endpoint);
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("new public output name");
            ui.add(
                egui::TextEdit::singleline(&mut self.new_component_output_name)
                    .char_limit(64)
                    .desired_width(180.0),
            );
            let selected = endpoint_choice_selected_label(
                self.new_component_output_source,
                &output_choices,
                "no unexposed internal output",
            );
            egui::ComboBox::from_id_salt("new_component_output_source")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (endpoint, label) in &output_choices {
                        ui.selectable_value(
                            &mut self.new_component_output_source,
                            Some(*endpoint),
                            label,
                        );
                    }
                });
            if ui
                .add_enabled(
                    !self.new_component_output_name.is_empty()
                        && self.new_component_output_source.is_some(),
                    egui::Button::new("add public output"),
                )
                .clicked()
                && let Some(source) = self.new_component_output_source
            {
                action = Some(ComponentConnectorUiAction::AddOutput {
                    name: self.new_component_output_name.clone(),
                    source,
                });
            }
        });

        let mut selected = self.selected_component_connector;
        let selected_label = selected.map_or_else(
            || "no public connector".to_owned(),
            |connector| component_connector_label(&document, connector),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("selected public connector");
            egui::ComboBox::from_id_salt("selected_component_connector")
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    for input in document.inputs() {
                        let connector = ComponentConnectorSelection::Input(input.id());
                        ui.selectable_value(
                            &mut selected,
                            Some(connector),
                            component_connector_label(&document, connector),
                        );
                    }
                    for output in document.outputs() {
                        let connector = ComponentConnectorSelection::Output(output.id());
                        ui.selectable_value(
                            &mut selected,
                            Some(connector),
                            component_connector_label(&document, connector),
                        );
                    }
                });
            if ui
                .add_enabled(
                    selected.is_some(),
                    egui::Button::new("remove public connector"),
                )
                .clicked()
                && let Some(connector) = selected
            {
                action = Some(match connector {
                    ComponentConnectorSelection::Input(input) => {
                        ComponentConnectorUiAction::RemoveInput(input)
                    }
                    ComponentConnectorSelection::Output(output) => {
                        ComponentConnectorUiAction::RemoveOutput(output)
                    }
                });
            }
        });
        self.selected_component_connector = selected;

        if let Some(connector) = selected {
            let (canonical_name, canonical_endpoint, choices) = match connector {
                ComponentConnectorSelection::Input(input) => {
                    let input = document.input(input)?;
                    (
                        input.name(),
                        input.target(),
                        component_input_endpoint_choices(&document, Some(input.id())),
                    )
                }
                ComponentConnectorSelection::Output(output) => {
                    let output = document.output(output)?;
                    (
                        output.name(),
                        output.source(),
                        component_output_endpoint_choices(&document, Some(output.id())),
                    )
                }
            };
            let mut draft = self
                .component_connector_drafts
                .get(&connector)
                .cloned()
                .unwrap_or_else(|| ComponentConnectorDraft {
                    name: canonical_name.to_owned(),
                    endpoint: canonical_endpoint,
                });
            if choices
                .iter()
                .all(|(endpoint, _)| *endpoint != draft.endpoint)
            {
                draft = ComponentConnectorDraft {
                    name: canonical_name.to_owned(),
                    endpoint: canonical_endpoint,
                };
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("stable name");
                ui.add(
                    egui::TextEdit::singleline(&mut draft.name)
                        .char_limit(64)
                        .desired_width(180.0),
                );
                let selected_endpoint = endpoint_choice_selected_label(
                    Some(draft.endpoint),
                    &choices,
                    "missing canonical endpoint",
                );
                egui::ComboBox::from_id_salt("selected_component_connector_endpoint")
                    .selected_text(selected_endpoint)
                    .show_ui(ui, |ui| {
                        for (endpoint, label) in &choices {
                            ui.selectable_value(&mut draft.endpoint, *endpoint, label);
                        }
                    });
                if ui.button("apply connector metadata").clicked() {
                    action = Some(match connector {
                        ComponentConnectorSelection::Input(input) => {
                            ComponentConnectorUiAction::UpdateInput {
                                input,
                                draft: draft.clone(),
                            }
                        }
                        ComponentConnectorSelection::Output(output) => {
                            ComponentConnectorUiAction::UpdateOutput {
                                output,
                                draft: draft.clone(),
                            }
                        }
                    });
                }
                if ui.button("reset connector draft").clicked() {
                    draft = ComponentConnectorDraft {
                        name: canonical_name.to_owned(),
                        endpoint: canonical_endpoint,
                    };
                }
            });
            self.component_connector_drafts.insert(connector, draft);
        }
        action
    }

    #[allow(
        clippy::too_many_lines,
        reason = "canonical panel layout, exact control editing, and replay-only indicators remain one auditable egui frame operation"
    )]
    fn show_front_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Reusable component front panel");
            if let Some(component) = &self.component {
                ui.monospace(format!(
                    "ALGC {}… · {} exact bindings",
                    digest_prefix(component.encoding.digest().0),
                    component.document.panel_items().len()
                ));
            }
        });
        ui.label(
            "This first canonical component wraps the autonomous reference controller: exact parameter controls write ALGW through the normal transactional boundary, and output indicators read only the attached exact replay.",
        );
        let Some(component) = self.component.clone() else {
            ui.colored_label(egui::Color32::YELLOW, &self.component_status);
            return;
        };
        let hierarchy_depth = hierarchy_depth(&component.hierarchy.flattening);
        ui.horizontal_wrapped(|ui| {
            ui.strong("Reusable instance proof");
            ui.monospace(format!(
                "ALGH {}… · {} collapsed instances / depth {} → {} ordinary ALGR nodes / {} wires · flattened ALGW {}…",
                digest_prefix(component.hierarchy.encoding.digest().0),
                component.hierarchy.document.flattened_instance_count(),
                hierarchy_depth,
                component.hierarchy.document.flattened_node_count(),
                component.hierarchy.document.flattened_wire_count(),
                digest_prefix(component.hierarchy.flattening.encoding().digest().0)
            ));
        });
        ui.monospace(format!(
            "ALGM {}… · {} total node origins / {} total wire origins",
            digest_prefix(component.hierarchy.source_map.digest().0),
            component.hierarchy.flattening.node_provenance().len(),
            component.hierarchy.flattening.wire_provenance().len(),
        ));
        let source_map_name = format!(
            "alumina-{}.algm",
            digest_prefix(component.hierarchy.source_map.digest().0)
        );
        let source_map_events = self.hierarchy_source_file_bridge.show(
            ui,
            component.hierarchy.source_map.bytes(),
            MAX_GRAPH_HIERARCHY_SOURCE_MAP_BYTES,
            &source_map_name,
            ALGM_FILE,
        );
        for event in source_map_events {
            match event {
                BoundedFileEvent::Import(Ok(bytes)) => {
                    match replay_graph_hierarchy_source_map(
                        &bytes,
                        &component.hierarchy.document,
                        GraphHierarchySourceMapLimits::interactive(),
                    ) {
                        Ok(replay) if replay.encoding() == &component.hierarchy.source_map => {
                            self.file_status = format!(
                                "verified {} exact ALGM bytes by fresh hierarchy flattening",
                                bytes.len()
                            );
                        }
                        Ok(_) => {
                            "ALGM import regenerated a different embedded-policy map without mutation"
                                .clone_into(&mut self.file_status);
                        }
                        Err(error) => {
                            self.file_status = format!(
                                "ALGM import rejected without graph or hierarchy mutation: {error}"
                            );
                        }
                    }
                }
                BoundedFileEvent::Import(Err(error)) => {
                    self.file_status = format!("ALGM file read rejected: {error}");
                }
                BoundedFileEvent::Export(Ok(bytes)) => {
                    self.file_status = format!(
                        "exported {bytes} exact ALGM bytes bound to the current ALGH and ALGW"
                    );
                }
                BoundedFileEvent::Export(Err(error)) => {
                    self.file_status = format!("ALGM export failed: {error}");
                }
            }
        }
        ui.separator();
        self.show_hierarchy_source_browser(ui, &component);
        ui.separator();
        self.reconcile_component_connector_selection(None);
        if let Some(action) = self.show_component_connector_authoring(ui, &component) {
            self.apply_component_connector_action(action);
            ui.label(&self.component_status);
            return;
        }
        self.reconcile_panel_selection(None);
        let binding_choices = panel_binding_choices(&component.document);
        let used_bindings = component
            .document
            .panel_items()
            .iter()
            .map(GraphFrontPanelItem::binding)
            .collect::<BTreeSet<_>>();
        let available_bindings = binding_choices
            .iter()
            .filter(|(binding, _)| !used_bindings.contains(binding))
            .cloned()
            .collect::<Vec<_>>();
        if self
            .new_panel_binding
            .is_none_or(|binding| used_bindings.contains(&binding))
        {
            self.new_panel_binding = available_bindings.first().map(|(binding, _)| *binding);
        }
        let mut panel_action = None;
        ui.separator();
        ui.strong("Canonical front-panel authoring");
        ui.weak(
            "Choose one unbound exact input, parameter, or output; add it with a fresh monotonic identity. Select an item to edit its stable metadata, or drag its header to an integer position. All changes replace the selected ALGC through complete ALGH/ALGM/ALGS admission.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("new stable name");
            ui.add(
                egui::TextEdit::singleline(&mut self.new_panel_item_name)
                    .char_limit(64)
                    .desired_width(180.0),
            );
            let selected_binding_label = self.new_panel_binding.map_or_else(
                || "no unbound exact binding".to_owned(),
                |binding| panel_binding_label(&component.document, binding),
            );
            egui::ComboBox::from_id_salt("new_component_panel_binding")
                .selected_text(selected_binding_label)
                .show_ui(ui, |ui| {
                    for (binding, label) in &available_bindings {
                        ui.selectable_value(&mut self.new_panel_binding, Some(*binding), label);
                    }
                });
            if ui
                .add_enabled(
                    !self.new_panel_item_name.is_empty() && self.new_panel_binding.is_some(),
                    egui::Button::new("add bound item"),
                )
                .clicked()
                && let Some(binding) = self.new_panel_binding
            {
                match next_panel_item_rect(&component.document) {
                    Ok(rect) => {
                        panel_action = Some(PanelUiAction::Add {
                            name: self.new_panel_item_name.clone(),
                            binding,
                            rect,
                        });
                    }
                    Err(error) => {
                        self.component_status =
                            format!("front-panel edit rejected without mutation: {error}");
                    }
                }
            }
        });

        let mut selected_panel_item = self.selected_panel_item;
        let selected_panel_label = selected_panel_item
            .and_then(|id| component.document.panel_item(id))
            .map_or_else(
                || "no panel item".to_owned(),
                |item| format!("#{} {}", item.id().get(), item.name()),
            );
        ui.horizontal_wrapped(|ui| {
            ui.label("selected item");
            egui::ComboBox::from_id_salt("selected_component_panel_item")
                .selected_text(selected_panel_label)
                .show_ui(ui, |ui| {
                    for item in component.document.panel_items() {
                        ui.selectable_value(
                            &mut selected_panel_item,
                            Some(item.id()),
                            format!("#{} {}", item.id().get(), item.name()),
                        );
                    }
                });
            if ui
                .add_enabled(
                    selected_panel_item.is_some(),
                    egui::Button::new("remove selected item"),
                )
                .clicked()
                && let Some(item) = selected_panel_item
            {
                panel_action = Some(PanelUiAction::Remove(item));
            }
        });
        self.selected_panel_item = selected_panel_item;

        if let Some(item_id) = selected_panel_item
            && let Some(item) = component.document.panel_item(item_id)
        {
            let mut draft = self
                .panel_item_drafts
                .get(&item_id)
                .cloned()
                .unwrap_or_else(|| PanelItemDraft::from_item(item));
            let selectable_bindings = binding_choices
                .iter()
                .filter(|(binding, _)| {
                    *binding == draft.binding
                        || !component.document.panel_items().iter().any(|candidate| {
                            candidate.id() != item_id && candidate.binding() == *binding
                        })
                })
                .cloned()
                .collect::<Vec<_>>();
            ui.horizontal_wrapped(|ui| {
                ui.label("name");
                ui.add(
                    egui::TextEdit::singleline(&mut draft.name)
                        .char_limit(64)
                        .desired_width(170.0),
                );
                egui::ComboBox::from_id_salt("selected_component_panel_binding")
                    .selected_text(panel_binding_label(&component.document, draft.binding))
                    .show_ui(ui, |ui| {
                        for (binding, label) in &selectable_bindings {
                            ui.selectable_value(&mut draft.binding, *binding, label);
                        }
                    });
            });
            ui.horizontal_wrapped(|ui| {
                let maximum = i32::try_from(component.document.limits().maximum_panel_coordinate)
                    .unwrap_or(i32::MAX);
                ui.label("x");
                ui.add(egui::DragValue::new(&mut draft.x).range(0..=maximum));
                ui.label("y");
                ui.add(egui::DragValue::new(&mut draft.y).range(0..=maximum));
                ui.label("width");
                ui.add(
                    egui::DragValue::new(&mut draft.width)
                        .range(1..=component.document.limits().maximum_panel_coordinate),
                );
                ui.label("height");
                ui.add(
                    egui::DragValue::new(&mut draft.height)
                        .range(1..=component.document.limits().maximum_panel_coordinate),
                );
                if ui.button("apply exact metadata").clicked() {
                    panel_action = Some(PanelUiAction::Update {
                        item: item_id,
                        draft: draft.clone(),
                    });
                }
                if ui.button("reset draft").clicked() {
                    draft = PanelItemDraft::from_item(item);
                }
            });
            self.panel_item_drafts.insert(item_id, draft);
        }
        let items = component
            .document
            .panel_items()
            .iter()
            .filter_map(|item| {
                let value_type = component.document.panel_item_value_type(item.id())?;
                let output_text = match item.binding() {
                    GraphFrontPanelBinding::OutputIndicator(output) => {
                        Some(self.component_output_text(&component.document, output))
                    }
                    GraphFrontPanelBinding::InputControl(_)
                    | GraphFrontPanelBinding::ParameterControl { .. } => None,
                };
                Some(ComponentPanelItem {
                    id: item.id(),
                    name: item.name().to_owned(),
                    binding: item.binding(),
                    rect: item.rect(),
                    value_type,
                    output_text,
                })
            })
            .collect::<Vec<_>>();
        let maximum_right = items
            .iter()
            .map(|item| {
                u32::try_from(item.rect.x()).expect("validated ALGC panel x is nonnegative")
                    + item.rect.width()
            })
            .max()
            .unwrap_or(0);
        let maximum_bottom = items
            .iter()
            .map(|item| {
                u32::try_from(item.rect.y()).expect("validated ALGC panel y is nonnegative")
                    + item.rect.height()
            })
            .max()
            .unwrap_or(0);
        let panel_size = egui::vec2(
            display_panel_coordinate(maximum_right.saturating_add(20)).max(360.0),
            display_panel_coordinate(maximum_bottom.saturating_add(20)).max(120.0),
        );
        let mut parameter_request = None;
        let mut clicked_panel_item = None;
        egui::ScrollArea::both()
            .id_salt("exact_control_component_front_panel")
            .auto_shrink([false, false])
            .max_height(285.0)
            .show(ui, |ui| {
                let (surface, painter) = ui.allocate_painter(panel_size, egui::Sense::hover());
                painter.rect_filled(surface.rect, 5.0, egui::Color32::from_rgb(21, 25, 34));
                paint_grid(&painter, surface.rect);
                for item in &items {
                    let canonical_rect = egui::Rect::from_min_size(
                        surface.rect.min
                            + egui::vec2(
                                display_coordinate(item.rect.x()),
                                display_coordinate(item.rect.y()),
                            ),
                        egui::vec2(
                            display_panel_coordinate(item.rect.width()),
                            display_panel_coordinate(item.rect.height()),
                        ),
                    );
                    let header = egui::Rect::from_min_max(
                        canonical_rect.min,
                        egui::pos2(canonical_rect.right(), canonical_rect.top() + 22.0),
                    );
                    let response = ui.interact(
                        header,
                        egui::Id::new(("component_panel_item", item.id.get())),
                        egui::Sense::click_and_drag(),
                    );
                    if response.drag_started() {
                        self.panel_drag = Some(PanelItemDrag {
                            item: item.id,
                            origin: item.rect,
                            delta: egui::Vec2::ZERO,
                        });
                    }
                    if response.dragged()
                        && let Some(drag) = self.panel_drag.as_mut()
                        && drag.item == item.id
                    {
                        drag.delta += response.drag_delta();
                    }
                    let dragging = self
                        .panel_drag
                        .filter(|drag| drag.item == item.id)
                        .filter(|_| response.dragged() || response.drag_stopped());
                    let rect = dragging
                        .map_or(canonical_rect, |drag| canonical_rect.translate(drag.delta));
                    if response.drag_stopped()
                        && let Some(drag) = dragging
                    {
                        match (
                            quantized_canvas_coordinate(drag.origin.x(), drag.delta.x),
                            quantized_canvas_coordinate(drag.origin.y(), drag.delta.y),
                        ) {
                            (Ok(x), Ok(y)) => {
                                panel_action = Some(PanelUiAction::Update {
                                    item: item.id,
                                    draft: PanelItemDraft {
                                        name: item.name.clone(),
                                        binding: item.binding,
                                        x,
                                        y,
                                        width: item.rect.width(),
                                        height: item.rect.height(),
                                    },
                                });
                            }
                            (Err(error), _) | (_, Err(error)) => {
                                self.component_status =
                                    format!("front-panel move rejected without mutation: {error}");
                            }
                        }
                        self.panel_drag = None;
                    }
                    if response.clicked() {
                        clicked_panel_item = Some(item.id);
                    }
                    painter.rect_filled(rect, 5.0, egui::Color32::from_rgb(35, 44, 57));
                    painter.rect_stroke(
                        rect,
                        5.0,
                        egui::Stroke::new(
                            if self.selected_panel_item == Some(item.id) {
                                2.0_f32
                            } else {
                                1.0_f32
                            },
                            if self.selected_panel_item == Some(item.id) {
                                egui::Color32::from_rgb(122, 211, 185)
                            } else {
                                egui::Color32::from_rgb(90, 119, 151)
                            },
                        ),
                    );
                    painter.text(
                        rect.left_top() + egui::vec2(8.0, 6.0),
                        egui::Align2::LEFT_TOP,
                        panel_item_label(&item.name),
                        egui::FontId::proportional(12.0),
                        egui::Color32::WHITE,
                    );
                    match item.binding {
                        GraphFrontPanelBinding::ParameterControl { node, parameter } => {
                            let Some(parameter_definition) = self
                                .workspace
                                .graph()
                                .node(node)
                                .and_then(|node| {
                                    node.parameters()
                                        .iter()
                                        .find(|candidate| candidate.id() == parameter)
                                })
                                .cloned()
                            else {
                                continue;
                            };
                            let Some(initial) = parameter_edit_text(
                                self.workspace.graph(),
                                parameter_definition.value(),
                            ) else {
                                painter.text(
                                    rect.center_bottom() - egui::vec2(0.0, 7.0),
                                    egui::Align2::CENTER_BOTTOM,
                                    "selector-bound or oversized literal is read-only",
                                    egui::FontId::monospace(9.5),
                                    egui::Color32::YELLOW,
                                );
                                continue;
                            };
                            let maximum = parameter_text_limit(
                                self.workspace.graph(),
                                parameter_definition.value().value_type(),
                            );
                            let draft = self
                                .parameter_drafts
                                .entry((node, parameter))
                                .or_insert(initial);
                            let input_rect = egui::Rect::from_min_max(
                                rect.left_top() + egui::vec2(8.0, 25.0),
                                rect.right_bottom() - egui::vec2(57.0, 7.0),
                            );
                            let button_rect = egui::Rect::from_min_max(
                                egui::pos2(input_rect.right() + 5.0, input_rect.top()),
                                rect.right_bottom() - egui::vec2(7.0, 7.0),
                            );
                            let response = ui.put(
                                input_rect,
                                egui::TextEdit::singleline(draft).char_limit(maximum),
                            );
                            let apply = ui.put(button_rect, egui::Button::new("apply")).clicked()
                                || (response.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                            if apply {
                                parameter_request = Some((node, parameter, draft.clone()));
                            }
                        }
                        GraphFrontPanelBinding::OutputIndicator(_) => {
                            painter.text(
                                rect.left_bottom() + egui::vec2(8.0, -8.0),
                                egui::Align2::LEFT_BOTTOM,
                                item.output_text.as_deref().unwrap_or("no exact sample"),
                                egui::FontId::monospace(11.0),
                                if self.reference_trace_is_current() {
                                    egui::Color32::from_rgb(122, 211, 185)
                                } else {
                                    egui::Color32::YELLOW
                                },
                            );
                        }
                        GraphFrontPanelBinding::InputControl(_) => {
                            painter.text(
                                rect.left_bottom() + egui::vec2(8.0, -8.0),
                                egui::Align2::LEFT_BOTTOM,
                                format!("runtime input · t{}", item.value_type.get()),
                                egui::FontId::monospace(10.0),
                                egui::Color32::GRAY,
                            );
                        }
                    }
                }
            });
        if let Some(item) = clicked_panel_item {
            self.selected_panel_item = Some(item);
        }
        ui.weak(&self.component_status);
        if let Some(action) = panel_action {
            self.apply_panel_action(action);
        } else if let Some((node, parameter, text)) = parameter_request {
            self.commit_parameter_text(node, parameter, &text);
        }
    }

    fn component_output_text(
        &self,
        component: &GraphComponentDocument,
        output: GraphComponentOutputId,
    ) -> String {
        if !self.reference_trace_is_current() {
            return "exact replay detached after draft edit".to_owned();
        }
        let Some(endpoint) = component.output(output).map(GraphComponentOutput::source) else {
            return "unresolved output binding".to_owned();
        };
        let Some(signal) = SIGNALS
            .iter()
            .copied()
            .find(|signal| signal.endpoint() == endpoint)
        else {
            return format!(
                "output #{}.{} has no replay probe",
                endpoint.node.get(),
                endpoint.port.get()
            );
        };
        let Some(series) = self
            .traces
            .iter()
            .find(|series| series.signal.representative == Some(signal))
        else {
            return format!("no replay series for {}", signal.label());
        };
        let Some(point) = trace_point_at_or_before(series, &self.cursor_root_tick) else {
            return format!("no sample at or before root tick {}", self.cursor_root_tick);
        };
        match &point.value {
            TracePointValue::Analog { exact, .. } => format!(
                "{exact}{} · c{}:t{}:s{} · root {}",
                series
                    .signal
                    .unit_symbol
                    .as_deref()
                    .map_or_else(String::new, |unit| format!(" {unit}")),
                point.clock.get(),
                point.tick,
                point.sequence,
                point.root_tick
            ),
            TracePointValue::Boolean(value) => format!(
                "{value} · c{}:t{}:s{} · root {}",
                point.clock.get(),
                point.tick,
                point.sequence,
                point.root_tick
            ),
            TracePointValue::State { summary, encoding } => format!(
                "{} · c{}:t{}:s{} · root {}",
                state_identity_label(summary, encoding),
                point.clock.get(),
                point.tick,
                point.sequence,
                point.root_tick
            ),
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "canvas allocation, layered node/port interaction, preview, and deferred transactional edits remain one egui frame operation"
    )]
    fn show_graph(&mut self, ui: &mut egui::Ui, maximum_height: f32) {
        if let Some(authoritative) = self
            .component
            .as_ref()
            .map(|component| component.encoding.digest())
        {
            self.scroll_to_open_hierarchy_source(
                ui,
                HierarchySourceDestination::Component(authoritative),
            );
        }
        let nodes = self.workspace.graph().nodes().to_vec();
        let wires = self.workspace.graph().wires().to_vec();
        let mut clicked_node = None;
        let mut canvas_clicked = false;
        let mut move_request = None;
        let mut port_edit = None;
        egui::ScrollArea::both()
            .id_salt("exact_control_graph_canvas")
            .auto_shrink([false, false])
            .max_height(maximum_height)
            .show(ui, |ui| {
                let (canvas, painter) =
                    ui.allocate_painter(self.presentation.size, egui::Sense::click());
                canvas_clicked = canvas.clicked();
                painter.rect_filled(canvas.rect, 0.0, egui::Color32::from_rgb(17, 21, 29));
                paint_grid(&painter, canvas.rect);
                let origin = canvas.rect.min.to_vec2();

                for wire in &wires {
                    Self::paint_wire(
                        &painter,
                        origin,
                        self.workspace.graph(),
                        &self.presentation,
                        self.selected_node,
                        wire,
                    );
                }
                for node in &nodes {
                    let Some(presentation) = self.presentation.nodes.get(&node.id()) else {
                        continue;
                    };
                    let rect = presentation.rect.translate(origin);
                    let header_rect = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.right(), rect.top() + NODE_HEADER_HEIGHT),
                    );
                    let response = ui.interact(
                        header_rect,
                        egui::Id::new(("exact_control_node", node.id().get())),
                        egui::Sense::click_and_drag(),
                    );
                    if response.drag_started()
                        && let Some(placement) = self.workspace.placement(node.id())
                    {
                        self.drag = Some(NodeDrag {
                            node: node.id(),
                            origin: placement,
                            delta: egui::Vec2::ZERO,
                        });
                    }
                    if response.dragged()
                        && let Some(drag) = self.drag.as_mut()
                        && drag.node == node.id()
                    {
                        drag.delta += response.drag_delta();
                    }
                    let dragging = self
                        .drag
                        .filter(|drag| drag.node == node.id())
                        .filter(|_| response.dragged() || response.drag_stopped());
                    let painted_rect = dragging.map_or(rect, |drag| rect.translate(drag.delta));
                    if response.drag_stopped()
                        && let Some(drag) = dragging
                    {
                        move_request = Some((drag, drag.delta));
                        self.drag = None;
                    }
                    if response.clicked() {
                        clicked_node = Some(node.id());
                    }
                    for (index, port) in node.inputs().iter().enumerate() {
                        let anchor = port_anchor_for_rect(painted_rect, index, false);
                        let port_response = ui.interact(
                            egui::Rect::from_center_size(anchor, egui::vec2(18.0, 18.0)),
                            egui::Id::new((
                                "exact_control_input",
                                node.id().get(),
                                port.id().get(),
                            )),
                            egui::Sense::click(),
                        );
                        let endpoint = WireEndpoint {
                            node: node.id(),
                            port: port.id(),
                        };
                        if port_response.secondary_clicked() {
                            port_edit = Some(PortEdit::DisconnectInput(endpoint));
                            clicked_node = Some(node.id());
                        } else if port_response.clicked() {
                            port_edit = Some(PortEdit::ConnectInput(endpoint));
                            clicked_node = Some(node.id());
                        }
                    }
                    for (index, port) in node.outputs().iter().enumerate() {
                        let anchor = port_anchor_for_rect(painted_rect, index, true);
                        let port_response = ui.interact(
                            egui::Rect::from_center_size(anchor, egui::vec2(18.0, 18.0)),
                            egui::Id::new((
                                "exact_control_output",
                                node.id().get(),
                                port.id().get(),
                            )),
                            egui::Sense::click(),
                        );
                        if port_response.clicked() {
                            port_edit = Some(PortEdit::SelectOutput(WireEndpoint {
                                node: node.id(),
                                port: port.id(),
                            }));
                            clicked_node = Some(node.id());
                        }
                    }
                    self.paint_node(
                        &painter,
                        painted_rect,
                        node,
                        presentation.rank,
                        self.selected_node,
                    );
                }
                if let Some(source) = self.pending_source
                    && let Some(source_anchor) =
                        port_anchor(self.workspace.graph(), &self.presentation, source, true)
                    && let Some(pointer) = ui.ctx().pointer_hover_pos()
                {
                    painter.line_segment(
                        [source_anchor + origin, pointer],
                        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    );
                }
            });
        let interaction_consumed =
            clicked_node.is_some() || move_request.is_some() || port_edit.is_some();
        if let Some((drag, delta)) = move_request {
            self.commit_node_drag(drag, delta);
        }
        if let Some(edit) = port_edit {
            self.handle_port_edit(edit);
        }
        if let Some(node) = clicked_node {
            self.selected_node = Some(node);
        } else if canvas_clicked && !interaction_consumed {
            self.selected_node = None;
        }
    }

    fn paint_wire(
        painter: &egui::Painter,
        origin: egui::Vec2,
        document: &GraphDocument,
        presentation: &GraphPresentation,
        selected_node: Option<GraphNodeId>,
        wire: &WireDefinition,
    ) {
        let (Some(source_anchor), Some(target_anchor)) = (
            port_anchor(document, presentation, wire.source(), true),
            port_anchor(document, presentation, wire.target(), false),
        ) else {
            return;
        };
        let source_anchor = source_anchor + origin;
        let target_anchor = target_anchor + origin;
        let selected = selected_node
            .is_some_and(|node| node == wire.source().node || node == wire.target().node);
        let color = if selected {
            egui::Color32::WHITE
        } else {
            wire_color(document, wire.source())
        };
        let stroke = egui::Stroke::new(if selected { 2.4_f32 } else { 1.5_f32 }, color);
        let feedback_lane = presentation
            .wires
            .get(&wire.id())
            .and_then(|wire| wire.feedback_lane);
        let points = if let Some(lane) = feedback_lane {
            let route_y = origin.y + presentation.size.y - 18.0 - display_index(lane) * 13.0;
            vec![
                source_anchor,
                egui::pos2(source_anchor.x + 22.0, source_anchor.y),
                egui::pos2(source_anchor.x + 22.0, route_y),
                egui::pos2(target_anchor.x - 22.0, route_y),
                egui::pos2(target_anchor.x - 22.0, target_anchor.y),
                target_anchor,
            ]
        } else {
            let middle_x = (source_anchor.x + target_anchor.x) * 0.5;
            vec![
                source_anchor,
                egui::pos2(middle_x, source_anchor.y),
                egui::pos2(middle_x, target_anchor.y),
                target_anchor,
            ]
        };
        painter.add(egui::Shape::line(points, stroke));
        painter.add(egui::Shape::convex_polygon(
            vec![
                target_anchor,
                target_anchor + egui::vec2(-7.0, -4.0),
                target_anchor + egui::vec2(-7.0, 4.0),
            ],
            color,
            egui::Stroke::NONE,
        ));
    }

    fn paint_node(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        node: &NodeDefinition,
        rank: usize,
        selected_node: Option<GraphNodeId>,
    ) {
        let selected = selected_node == Some(node.id());
        let stateful = self
            .fixture
            .registry()
            .semantic_registry()
            .schema(node.kind())
            .is_some_and(|schema| schema.state().is_some());
        let fill = if selected {
            egui::Color32::from_rgb(46, 73, 105)
        } else if stateful {
            egui::Color32::from_rgb(49, 47, 67)
        } else {
            egui::Color32::from_rgb(34, 43, 56)
        };
        let border = if selected {
            egui::Color32::from_rgb(126, 195, 255)
        } else if stateful {
            egui::Color32::from_rgb(209, 158, 255)
        } else {
            egui::Color32::from_rgb(89, 111, 139)
        };
        painter.rect_filled(rect, 6.0, fill);
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, border),
        );
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.top() + NODE_HEADER_HEIGHT),
                egui::pos2(rect.right(), rect.top() + NODE_HEADER_HEIGHT),
            ],
            egui::Stroke::new(1.0_f32, border.gamma_multiply(0.65)),
        );
        painter.text(
            rect.left_top() + egui::vec2(10.0, 9.0),
            egui::Align2::LEFT_TOP,
            node.label(),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );
        painter.text(
            rect.left_top() + egui::vec2(10.0, 28.0),
            egui::Align2::LEFT_TOP,
            format!(
                "L{rank} · {} · v{}",
                short_kind(node.kind().name()),
                node.kind().version()
            ),
            egui::FontId::monospace(10.5),
            egui::Color32::GRAY,
        );

        for (index, port) in node.inputs().iter().enumerate() {
            let anchor = port_anchor_for_rect(rect, index, false);
            painter.circle_filled(anchor, 4.0, egui::Color32::from_rgb(136, 171, 211));
            painter.text(
                anchor + egui::vec2(9.0, 0.0),
                egui::Align2::LEFT_CENTER,
                port.name(),
                egui::FontId::proportional(11.0),
                egui::Color32::LIGHT_GRAY,
            );
        }
        for (index, port) in node.outputs().iter().enumerate() {
            let anchor = port_anchor_for_rect(rect, index, true);
            painter.circle_filled(anchor, 4.0, egui::Color32::from_rgb(122, 211, 185));
            painter.text(
                anchor + egui::vec2(-9.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                port.name(),
                egui::FontId::proportional(11.0),
                egui::Color32::LIGHT_GRAY,
            );
        }
        let footer = match (stateful, node.parameters().len()) {
            (true, count) => format!("unit delay · {count} exact parameter(s)"),
            (false, 0) => "HostExact".to_owned(),
            (false, count) => format!("HostExact · {count} exact parameter(s)"),
        };
        painter.text(
            rect.left_bottom() + egui::vec2(10.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            footer,
            egui::FontId::monospace(9.5),
            egui::Color32::from_rgb(170, 180, 194),
        );
    }

    fn reference_trace_is_current(&self) -> bool {
        self.workspace.graph_digest() == self.fixture.simulation().graph_digest()
    }

    fn reset_draft(&mut self) {
        match initial_workspace(&self.fixture) {
            Ok((workspace, _, _)) => {
                if self.commit_candidate(
                    workspace,
                    "reset draft to the canonical reference graph and layout",
                ) {
                    self.selected_node = None;
                    self.pending_source = None;
                    self.drag = None;
                    self.parameter_drafts.clear();
                    self.node_label_drafts.clear();
                }
            }
            Err(error) => {
                self.edit_status = format!("draft reset failed without mutation: {error}");
            }
        }
    }

    fn add_palette_node(&mut self) {
        let Some(entry) = self.palette.get(self.palette_index).cloned() else {
            "node creation rejected without mutation: palette selection is unavailable"
                .clone_into(&mut self.edit_status);
            return;
        };
        let (x, y) = match new_node_position(&self.workspace) {
            Ok(position) => position,
            Err(error) => {
                self.edit_status = format!("node creation rejected without mutation: {error}");
                return;
            }
        };
        let mut candidate = self.workspace.clone();
        let id = match candidate.create_node(entry.prototype, x, y) {
            Ok(id) => id,
            Err(error) => {
                self.edit_status = format!("node creation rejected without mutation: {error}");
                return;
            }
        };
        if self.commit_candidate(
            candidate,
            &format!(
                "created {} as node {} at canonical canvas ({x}, {y})",
                entry.display_name,
                id.get()
            ),
        ) {
            self.selected_node = Some(id);
            self.pending_source = None;
        }
    }

    fn delete_selected_node(&mut self, id: GraphNodeId) {
        let mut candidate = self.workspace.clone();
        let removed_wires = match candidate.delete_node(id) {
            Ok(count) => count,
            Err(error) => {
                self.edit_status = format!("node deletion rejected without mutation: {error}");
                return;
            }
        };
        if self.commit_candidate(
            candidate,
            &format!(
                "deleted node {} and {removed_wires} incident wire(s) without reusing identities",
                id.get()
            ),
        ) {
            self.selected_node = None;
            self.pending_source = self.pending_source.filter(|source| source.node != id);
            self.parameter_drafts.retain(|(node, _), _| *node != id);
            self.node_label_drafts.remove(&id);
        }
    }

    fn commit_node_label(&mut self, node_id: GraphNodeId, label: &str) {
        if self.workspace.graph().node(node_id).is_none() {
            self.edit_status = format!(
                "node label edit rejected without mutation: node {} is unavailable",
                node_id.get()
            );
            return;
        }
        let mut candidate = self.workspace.clone();
        if let Err(error) = candidate.set_node_label(node_id, label) {
            self.edit_status = format!("node label edit rejected without mutation: {error}");
            return;
        }
        if self.commit_candidate(
            candidate,
            &format!("set node {} label to exact UTF-8 {label:?}", node_id.get()),
        ) {
            self.node_label_drafts.insert(node_id, label.to_owned());
        }
    }

    fn commit_node_domain(&mut self, node_id: GraphNodeId, domain: ExecutionDomain) {
        if self.workspace.graph().node(node_id).is_none() {
            self.edit_status = format!(
                "node domain edit rejected without mutation: node {} is unavailable",
                node_id.get()
            );
            return;
        }
        let mut candidate = self.workspace.clone();
        if let Err(error) = candidate.set_node_domain(node_id, domain) {
            self.edit_status = format!("node domain edit rejected without mutation: {error}");
            return;
        }
        self.commit_candidate(
            candidate,
            &format!(
                "set node {} execution placement to {}",
                node_id.get(),
                domain_choice_label(domain)
            ),
        );
    }

    fn commit_parameter_text(&mut self, node_id: GraphNodeId, parameter_id: u32, text: &str) {
        let Some(parameter) = self
            .workspace
            .graph()
            .node(node_id)
            .and_then(|node| {
                node.parameters()
                    .iter()
                    .find(|parameter| parameter.id() == parameter_id)
            })
            .cloned()
        else {
            self.edit_status = format!(
                "parameter edit rejected without mutation: node {} parameter {parameter_id} is unavailable",
                node_id.get()
            );
            return;
        };
        let value = match parse_parameter_text(
            self.workspace.graph(),
            parameter.value().value_type(),
            text,
        ) {
            Ok(value) => value,
            Err(error) => {
                self.edit_status = format!("parameter edit rejected without mutation: {error}");
                return;
            }
        };
        let canonical =
            parameter_edit_text(self.workspace.graph(), &value).unwrap_or_else(|| text.to_owned());
        let mut candidate = self.workspace.clone();
        if let Err(error) = candidate.set_parameter(node_id, parameter_id, value) {
            self.edit_status = format!("parameter edit rejected without mutation: {error}");
            return;
        }
        if self.commit_candidate(
            candidate,
            &format!(
                "set node {} parameter {} to exact canonical {canonical}",
                node_id.get(),
                parameter.name()
            ),
        ) {
            self.parameter_drafts
                .insert((node_id, parameter_id), canonical);
        }
    }

    fn commit_node_drag(&mut self, drag: NodeDrag, delta: egui::Vec2) {
        let (x, y) = match (
            quantized_canvas_coordinate(drag.origin.x(), delta.x),
            quantized_canvas_coordinate(drag.origin.y(), delta.y),
        ) {
            (Ok(x), Ok(y)) => (x, y),
            (Err(error), _) | (_, Err(error)) => {
                self.edit_status = format!("node move rejected without mutation: {error}");
                return;
            }
        };
        let mut candidate = self.workspace.clone();
        if let Err(error) = candidate.move_node(drag.node, x, y) {
            self.edit_status = format!("node move rejected without mutation: {error}");
            return;
        }
        self.commit_candidate(
            candidate,
            &format!(
                "moved node {} to canonical canvas ({x}, {y})",
                drag.node.get()
            ),
        );
    }

    fn handle_port_edit(&mut self, edit: PortEdit) {
        match edit {
            PortEdit::SelectOutput(source) => {
                if self.pending_source == Some(source) {
                    self.pending_source = None;
                    "pending wire cancelled".clone_into(&mut self.edit_status);
                } else {
                    self.pending_source = Some(source);
                    self.edit_status = format!(
                        "selected output #{}.{}; choose one typed input",
                        source.node.get(),
                        source.port.get()
                    );
                }
            }
            PortEdit::ConnectInput(target) => {
                let Some(source) = self.pending_source else {
                    self.edit_status = format!(
                        "input #{}.{} selected; choose an output first",
                        target.node.get(),
                        target.port.get()
                    );
                    return;
                };
                let mut candidate = self.workspace.clone();
                let id = match candidate.connect(source, target) {
                    Ok(id) => id,
                    Err(error) => {
                        self.edit_status = format!("wire edit rejected without mutation: {error}");
                        return;
                    }
                };
                if self.commit_candidate(
                    candidate,
                    &format!(
                        "connected wire {} from #{}.{} to #{}.{}",
                        id.get(),
                        source.node.get(),
                        source.port.get(),
                        target.node.get(),
                        target.port.get()
                    ),
                ) {
                    self.pending_source = None;
                }
            }
            PortEdit::DisconnectInput(target) => {
                let Some(id) = self
                    .workspace
                    .graph()
                    .wires()
                    .iter()
                    .find(|wire| wire.target() == target)
                    .map(|wire| wire.id())
                else {
                    self.edit_status = format!(
                        "input #{}.{} is already disconnected",
                        target.node.get(),
                        target.port.get()
                    );
                    return;
                };
                let mut candidate = self.workspace.clone();
                if let Err(error) = candidate.disconnect(id) {
                    self.edit_status = format!("wire removal rejected without mutation: {error}");
                    return;
                }
                if self.commit_candidate(
                    candidate,
                    &format!(
                        "disconnected wire {} from input #{}.{}",
                        id.get(),
                        target.node.get(),
                        target.port.get()
                    ),
                ) {
                    self.pending_source = None;
                }
            }
        }
    }

    fn commit_candidate(&mut self, candidate: GraphWorkspaceDocument, success: &str) -> bool {
        if candidate == self.workspace {
            self.edit_status = format!("{success}; canonical workspace already matched");
            return true;
        }
        let (encoding, presentation, semantic) = match self.prepare_candidate(&candidate) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.edit_status = format!("edit rejected without mutation: {error}");
                return false;
            }
        };
        let (component, component_status) = self.prepare_component_for_workspace(&candidate);
        let (probes, probe_status) = match self.prepare_probes_for_workspace(&candidate) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.edit_status = format!(
                    "edit rejected without mutation because no exact ALGP sidecar could be prepared: {error}"
                );
                return false;
            }
        };
        if let Err(error) = Self::authoring_session_document_from_parts(
            &candidate,
            &probes.document,
            &self.cached_jobs.workspace,
            component.as_ref(),
        ) {
            self.edit_status =
                format!("complete-session candidate rejected without mutation: {error}");
            return false;
        }
        let history = match self.history_with_current_recorded() {
            Ok(history) => history,
            Err(error) => {
                self.edit_status = format!("edit history rejected without mutation: {error}");
                return false;
            }
        };
        let prior_component_digest = self
            .component
            .as_ref()
            .map(|component| component.encoding.digest());
        let replacement_component_digest = component
            .as_ref()
            .map(|component| component.encoding.digest());
        self.workspace = candidate;
        self.workspace_encoding = encoding;
        self.presentation = presentation;
        self.component = component;
        self.component_status = component_status;
        self.reconcile_hierarchy_selection(
            prior_component_digest.zip(replacement_component_digest),
        );
        self.reconcile_component_definition_editor();
        self.reconcile_component_connector_selection(self.selected_component_connector);
        self.reconcile_panel_selection(self.selected_panel_item);
        self.replace_probe_package(probes);
        self.probe_status = probe_status;
        self.history = history;
        self.selected_node = self
            .selected_node
            .filter(|node| self.workspace.graph().node(*node).is_some());
        self.pending_source = self
            .pending_source
            .filter(|source| self.workspace.graph().node(source.node).is_some());
        self.parameter_drafts
            .retain(|(node, _), _| self.workspace.graph().node(*node).is_some());
        self.node_label_drafts
            .retain(|node, _| self.workspace.graph().node(*node).is_some());
        self.persistence_dirty = true;
        self.persistence_attempted = false;
        self.edit_status = format!("{success}; {semantic}");
        true
    }

    fn history_with_current_recorded(&self) -> Result<GraphAuthoringSessionHistory, String> {
        let current = self.authoring_session_encoding()?;
        let mut history = self.history.clone();
        history
            .record(current)
            .map_err(|error| format!("complete ALGS history rejected: {error}"))?;
        Ok(history)
    }

    fn prepare_candidate(
        &self,
        candidate: &GraphWorkspaceDocument,
    ) -> Result<(CanonicalGraphWorkspaceEncoding, GraphPresentation, String), String> {
        let presentation = graph_presentation(
            candidate.graph(),
            self.fixture.registry(),
            Some(candidate.placements()),
        )?;
        let encoding = encode_graph_workspace(candidate).map_err(|error| error.to_string())?;
        let draft_analysis = analyze_graph_draft(
            candidate.graph(),
            self.fixture.registry().semantic_registry(),
        )
        .map_err(|error| format!("audited semantics rejected: {error}"))?;
        let semantic = if let Some(first) = draft_analysis.required_unconnected_inputs().first() {
            format!(
                "draft semantic blocker: {} required input(s) unconnected; first {first:?}",
                draft_analysis.required_unconnected_inputs().len()
            )
        } else {
            "audited semantics valid".to_owned()
        };
        Ok((encoding, presentation, semantic))
    }

    fn authoring_session_document_from_parts(
        workspace: &GraphWorkspaceDocument,
        probes: &GraphProbeDocument,
        cached_job_workspace: &GraphWorkspaceDocument,
        component: Option<&ComponentPackage>,
    ) -> Result<GraphAuthoringSessionDocument, String> {
        let hierarchy = component.map(|component| {
            GraphAuthoringHierarchyInput::new(
                component.encoding.digest(),
                component.hierarchy.document.clone(),
                component.hierarchy.source_map.clone(),
            )
        });
        GraphAuthoringSessionDocument::try_new(
            GraphAuthoringSessionLimits::interactive(),
            GraphHierarchySourceMapLimits::interactive(),
            workspace.clone(),
            probes.clone(),
            cached_job_workspace.clone(),
            hierarchy,
        )
        .map_err(|error| error.to_string())
    }

    fn authoring_session_document(&self) -> Result<GraphAuthoringSessionDocument, String> {
        let probes = self
            .probes
            .as_ref()
            .ok_or_else(|| "canonical ALGP sidecar is unavailable".to_owned())?;
        Self::authoring_session_document_from_parts(
            &self.workspace,
            &probes.document,
            &self.cached_jobs.workspace,
            self.component.as_ref(),
        )
    }

    fn authoring_session_encoding(&self) -> Result<CanonicalGraphAuthoringSessionEncoding, String> {
        let document = self.authoring_session_document()?;
        encode_graph_authoring_session(&document).map_err(|error| error.to_string())
    }

    fn prepare_authoring_session_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PreparedAuthoringSession, String> {
        let replay =
            replay_graph_authoring_session(bytes, GraphAuthoringSessionReplayLimits::interactive())
                .map_err(|error| error.to_string())?;
        let session = replay.document();
        let candidate = session.control_workspace().clone();
        let (encoding, presentation, _) = self.prepare_candidate(&candidate)?;
        if session.control_encoding() != &encoding {
            return Err("ALGS control ALGW identity changed during UI admission".to_owned());
        }
        let probes = ProbePackage {
            document: session.probes().clone(),
            encoding: session.probe_encoding().clone(),
        };
        let (cached_job_workspace, cached_job_encoding) = self
            .cached_jobs
            .replay_persisted_workspace(session.cached_job_encoding().bytes())?;
        if &cached_job_encoding != session.cached_job_encoding() {
            return Err(
                "ALGS cached-job ALGW identity changed during catalog admission".to_owned(),
            );
        }
        let component = session
            .hierarchy()
            .map(|hierarchy| {
                analyze_component_library_drafts(hierarchy.document(), &self.fixture)?;
                analyze_graph_draft(
                    hierarchy.flattening().workspace().graph(),
                    self.fixture.registry().semantic_registry(),
                )
                .map_err(|error| {
                    format!("ALGS flattened component draft semantics rejected: {error}")
                })?;
                if hierarchy.selected_component_encoding().digest()
                    != hierarchy.selected_component()
                {
                    return Err(
                        "ALGS selected component identity changed during UI admission".to_owned(),
                    );
                }
                Ok(ComponentPackage {
                    document: hierarchy.selected_component_document().clone(),
                    encoding: hierarchy.selected_component_encoding().clone(),
                    hierarchy: HierarchyPackage {
                        document: hierarchy.document().clone(),
                        encoding: hierarchy.encoding().clone(),
                        flattening: hierarchy.flattening().clone(),
                        source_map: hierarchy.source_map().clone(),
                    },
                })
            })
            .transpose()?;

        Ok(PreparedAuthoringSession {
            workspace: candidate,
            workspace_encoding: encoding,
            presentation,
            component,
            probes,
            cached_job_workspace,
            cached_job_encoding,
        })
    }

    fn commit_prepared_authoring_session(&mut self, prepared: PreparedAuthoringSession) {
        self.workspace = prepared.workspace;
        self.workspace_encoding = prepared.workspace_encoding;
        self.presentation = prepared.presentation;
        self.component = prepared.component;
        self.component_status = if self.component.is_some() {
            "restored exact selected ALGC and complete ALGH/ALGM from ALGS".to_owned()
        } else {
            "ALGS contains no selected component hierarchy".to_owned()
        };
        self.selected_hierarchy_component = self
            .component
            .as_ref()
            .map(|component| component.encoding.digest());
        self.selected_hierarchy_instance = None;
        self.reconcile_hierarchy_selection(None);
        self.selected_node = None;
        self.pending_hierarchy_source = None;
        self.hierarchy_drag = None;
        self.pending_source = None;
        self.drag = None;
        self.parameter_drafts.clear();
        self.node_label_drafts.clear();
        self.probe_drafts.clear();
        self.panel_item_drafts.clear();
        self.component_connector_drafts.clear();
        self.component_definition.reset_scope(
            self.selected_hierarchy_component,
            "restored selected component-definition navigation from canonical ALGS authority"
                .to_owned(),
        );
        self.component_connector_scope = self.selected_hierarchy_component;
        self.selected_component_connector = self.component.as_ref().and_then(|component| {
            component
                .document
                .inputs()
                .first()
                .map(|input| ComponentConnectorSelection::Input(input.id()))
                .or_else(|| {
                    component
                        .document
                        .outputs()
                        .first()
                        .map(|output| ComponentConnectorSelection::Output(output.id()))
                })
        });
        self.new_component_input_name.clear();
        self.new_component_input_target = None;
        self.new_component_output_name.clear();
        self.new_component_output_source = None;
        self.reconcile_component_connector_selection(self.selected_component_connector);
        self.selected_panel_item = self
            .component
            .as_ref()
            .and_then(|component| component.document.panel_items().first())
            .map(GraphFrontPanelItem::id);
        self.new_panel_item_name.clear();
        self.new_panel_binding = None;
        self.panel_drag = None;
        self.replace_probe_package(prepared.probes);
        self.cached_jobs.restore_persisted_workspace(
            prepared.cached_job_workspace,
            prepared.cached_job_encoding,
        );
        self.reset_cursor_to_trigger();
        "restored canonical ALGP sidecar with exact ALGS/ALGW binding"
            .clone_into(&mut self.probe_status);
    }

    fn restore_authoring_session_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let prepared = self.prepare_authoring_session_bytes(bytes)?;
        // Commit only after all nested artifacts, exact hierarchy provenance,
        // UI semantics, and every catalog-bound cached-job leaf have passed.
        self.commit_prepared_authoring_session(prepared);
        self.history.clear();
        self.persistence_dirty = false;
        self.persistence_attempted = false;
        Ok(())
    }

    fn apply_cached_job_action(&mut self, action: CachedJobGraphAction) {
        let update = match self.cached_jobs.prepare_action(action) {
            Ok(update) => update,
            Err(error) => {
                self.cached_jobs.status =
                    format!("cached-job edit rejected without mutation: {error}");
                return;
            }
        };
        if update.encoding == self.cached_jobs.encoding {
            self.cached_jobs.status = format!(
                "{}; canonical cached-job workspace already matched",
                update.status
            );
            return;
        }
        let Some(probes) = self.probes.as_ref() else {
            "cached-job edit rejected without mutation: canonical ALGP sidecar is unavailable"
                .clone_into(&mut self.cached_jobs.status);
            return;
        };
        if let Err(error) = Self::authoring_session_document_from_parts(
            &self.workspace,
            &probes.document,
            &update.workspace,
            self.component.as_ref(),
        ) {
            self.cached_jobs.status =
                format!("cached-job complete-session candidate rejected without mutation: {error}");
            return;
        }
        let history = match self.history_with_current_recorded() {
            Ok(history) => history,
            Err(error) => {
                self.cached_jobs.status = format!(
                    "cached-job authoring-session history rejected without mutation: {error}"
                );
                return;
            }
        };
        self.cached_jobs.commit_update(update);
        self.history = history;
        self.persistence_dirty = true;
        self.persistence_attempted = false;
    }

    fn reset_cursor_to_trigger(&mut self) {
        self.cursor_root_tick = self
            .probes
            .as_ref()
            .and_then(|probes| {
                exact_probe_projection(&self.fixture, &self.workspace, &probes.document).ok()
            })
            .and_then(|projection| {
                projection
                    .trigger_root_window()
                    .map(|(_, trigger, _)| trigger.clone())
            })
            .unwrap_or_else(|| Rational::from(0));
    }

    fn prepare_component_for_workspace(
        &self,
        workspace: &GraphWorkspaceDocument,
    ) -> (Option<ComponentPackage>, String) {
        let prepared = if let Some(current) = &self.component {
            let mut document = current.document.clone();
            document
                .replace_workspace(workspace.clone())
                .map_err(|error| error.to_string())
                .and_then(|()| {
                    let encoding =
                        encode_graph_component(&document).map_err(|error| error.to_string())?;
                    let mut hierarchy = current.hierarchy.document.clone();
                    let remapped = hierarchy
                        .replace_component(current.encoding.digest(), document.clone())
                        .map_err(|error| error.to_string())?;
                    if remapped != encoding.digest() {
                        return Err(
                            "hierarchy dependency replacement returned a foreign ALGC identity"
                                .to_owned(),
                        );
                    }
                    let hierarchy = hierarchy_package(hierarchy, &self.fixture)?;
                    Ok(ComponentPackage {
                        document,
                        encoding,
                        hierarchy,
                    })
                })
        } else {
            representative_component_document(workspace).and_then(|(document, encoding)| {
                let hierarchy =
                    representative_hierarchy(&document, encoding.digest(), &self.fixture)?;
                Ok(ComponentPackage {
                    document,
                    encoding,
                    hierarchy,
                })
            })
        };
        match prepared {
            Ok(component) => {
                let status = if self.component.is_some() {
                    "canonical ALGC updated while authored ALGH root instances and stable library dependencies were preserved exactly"
                } else {
                    "canonical ALGC connector pane and fresh representative ALGH attached"
                };
                (Some(component), status.to_owned())
            }
            Err(error) => (
                None,
                format!(
                    "ALGC front panel detached from this draft without affecting ALGW: {error}"
                ),
            ),
        }
    }

    fn prepare_probes_for_workspace(
        &self,
        workspace: &GraphWorkspaceDocument,
    ) -> Result<(ProbePackage, String), String> {
        let result = if let Some(current) = &self.probes {
            let mut document = current.document.clone();
            document
                .replace_workspace(workspace)
                .and_then(|()| {
                    let encoding = encode_graph_probes(&document)?;
                    Ok(ProbePackage { document, encoding })
                })
                .map_err(|error| error.to_string())
        } else {
            representative_probes(workspace)
        };
        match result {
            Ok(probes) => Ok((
                probes,
                "canonical ALGP diagnostic probes attached".to_owned(),
            )),
            Err(error) => empty_probes(workspace)
                .map(|probes| {
                    (
                        probes,
                        format!(
                            "ALGP bindings incompatible with revised ALGW were atomically replaced by an empty canonical sidecar: {error}"
                        ),
                    )
                })
                .map_err(|empty_error| {
                    format!(
                        "ALGP probe rebinding failed: {error}; empty sidecar failed: {empty_error}"
                    )
                }),
        }
    }

    fn replace_probe_package(&mut self, probes: ProbePackage) -> bool {
        let changed = self
            .probes
            .as_ref()
            .is_none_or(|current| current.encoding != probes.encoding);
        if let Some(trigger) = probes.document.trigger() {
            self.trigger_pre_samples = trigger.pretrigger_samples();
            self.trigger_post_samples = trigger.posttrigger_samples();
        }
        self.reconcile_probe_drafts(&probes.document);
        self.probes = Some(probes);
        if changed {
            self.persistence_dirty = true;
            self.persistence_attempted = false;
        }
        self.reset_cursor_to_trigger();
        changed
    }

    fn reset_probe_drafts(&mut self) {
        self.probe_drafts.clear();
        if let Some(probes) = &self.probes {
            self.probe_drafts.extend(
                probes
                    .document
                    .probes()
                    .iter()
                    .map(|probe| (probe.id(), ProbeEditDraft::from_probe(probe))),
            );
        }
    }

    fn reconcile_probe_drafts(&mut self, next: &GraphProbeDocument) {
        let prior_metadata = self
            .probes
            .as_ref()
            .map(|probes| {
                probes
                    .document
                    .probes()
                    .iter()
                    .map(|probe| (probe.id(), ProbeEditDraft::from_probe(probe)))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let mut prior_drafts = std::mem::take(&mut self.probe_drafts);
        self.probe_drafts = next
            .probes()
            .iter()
            .map(|probe| {
                let canonical = ProbeEditDraft::from_probe(probe);
                let draft = if prior_metadata.get(&probe.id()) == Some(&canonical) {
                    prior_drafts.remove(&probe.id()).unwrap_or(canonical)
                } else {
                    canonical
                };
                (probe.id(), draft)
            })
            .collect();
    }

    fn commit_probe_package(&mut self, probes: ProbePackage) -> Result<bool, String> {
        if self
            .probes
            .as_ref()
            .is_some_and(|current| current.encoding == probes.encoding)
        {
            return Ok(false);
        }
        Self::authoring_session_document_from_parts(
            &self.workspace,
            &probes.document,
            &self.cached_jobs.workspace,
            self.component.as_ref(),
        )
        .map_err(|error| format!("probe complete-session candidate rejected: {error}"))?;
        let history = self.history_with_current_recorded()?;
        let changed = self.replace_probe_package(probes);
        self.history = history;
        Ok(changed)
    }

    fn import_workspace_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let replay = replay_graph_workspace(
            bytes,
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .map_err(|error| error.to_string())?;
        let digest = digest_prefix(replay.encoding().digest().0);
        if self.commit_candidate(
            replay.into_document(),
            &format!("imported canonical ALGW {digest}…"),
        ) {
            self.pending_source = None;
            self.drag = None;
            self.parameter_drafts.clear();
            self.node_label_drafts.clear();
            self.reset_probe_drafts();
            Ok(())
        } else {
            Err(self.edit_status.clone())
        }
    }

    fn import_probe_bytes(&mut self, bytes: &[u8]) -> Result<bool, String> {
        let replay = replay_graph_probes(bytes, &self.workspace, GraphProbeLimits::interactive())
            .map_err(|error| error.to_string())?;
        let probes = ProbePackage {
            document: replay.document().clone(),
            encoding: replay.encoding().clone(),
        };
        let changed = self.commit_probe_package(probes)?;
        self.reset_probe_drafts();
        if changed {
            "imported canonical ALGP after exact current-workspace replay"
                .clone_into(&mut self.probe_status);
        } else {
            "canonical ALGP import was an exact no-op".clone_into(&mut self.probe_status);
        }
        Ok(changed)
    }

    fn show_selected_node(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_node else {
            ui.weak("Select a node to inspect exact ports, parameters, and state authority.");
            return;
        };
        let Some(node) = self.workspace.graph().node(id).cloned() else {
            return;
        };
        let document = self.workspace.graph().clone();
        let placement = self.workspace.placement(id);
        let schema = self
            .fixture
            .registry()
            .semantic_registry()
            .schema(node.kind());
        let state = schema.and_then(alumina_interface_core::graph::NodeSchema::state);
        let hierarchy_correlation = self.component.as_ref().and_then(|component| {
            hierarchy_node_correlation(component, id).map(|correlation| {
                (
                    digest_prefix(component.hierarchy.source_map.digest().0),
                    correlation,
                )
            })
        });
        let domain_choices = schema.map_or_else(Vec::new, |schema| {
            audited_domain_choices(&document, schema.allowed_domains())
        });
        let maximum_label_bytes = document.schema().limits().maximum_label_bytes;
        let mut label_text = self
            .node_label_drafts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| node.label().to_owned());
        let mut delete_requested = false;
        let mut label_request = None;
        let mut domain_request = None;
        let mut parameter_request = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("#{} {}", id.get(), node.label()));
                ui.monospace(format!("{} v{}", node.kind().name(), node.kind().version()));
                if ui.small_button("clear").clicked() {
                    self.selected_node = None;
                }
                delete_requested = ui.small_button("delete node + wires").clicked();
            });
            (label_request, domain_request) = show_node_identity_editors(
                ui,
                &node,
                &domain_choices,
                maximum_label_bytes,
                &mut label_text,
            );
            if let Some(placement) = placement {
                ui.monospace(format!(
                    "canvas = ({}, {}) logical px · presentation only",
                    placement.x(),
                    placement.y()
                ));
            }
            if let Some((source_map, correlation)) = &hierarchy_correlation {
                ui.monospace(format!(
                    "ALGM {source_map}… · hierarchy origin {correlation}"
                ));
            }
            ui.horizontal_wrapped(|ui| {
                for port in node.inputs() {
                    ui.monospace(port_description(&document, "in", port));
                }
                for port in node.outputs() {
                    ui.monospace(port_description(&document, "out", port));
                }
            });
            parameter_request =
                show_node_parameter_editors(ui, &document, &node, &mut self.parameter_drafts);
            if let Some(state) = state {
                ui.label(format!(
                    "Explicit state: clock {}, t{}, read-before-write, ≤{} canonical bytes",
                    state.clock().get(),
                    state.value_type().get(),
                    state.declared_storage_bytes()
                ));
            }
        });
        self.node_label_drafts.insert(id, label_text);
        if delete_requested {
            self.delete_selected_node(id);
        } else if let Some(label) = label_request {
            self.commit_node_label(id, &label);
        } else if let Some(domain) = domain_request {
            self.commit_node_domain(id, domain);
        } else if let Some((parameter, text)) = parameter_request {
            self.commit_parameter_text(id, parameter, &text);
        }
    }

    fn show_probes(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Diagnostic probes");
            if let Some(probes) = &self.probes {
                ui.monospace(format!(
                    "ALGP {}… · {} / {} bounded bindings",
                    digest_prefix(probes.encoding.digest().0),
                    probes.document.probes().len(),
                    probes.document.limits().maximum_probes
                ));
            }
        });
        ui.label(
            "These saved bindings select exact graph outputs, bounded host retention, and one replay-only Boolean edge trigger. They do not grant GPIO access, telemetry bandwidth, device-trigger configuration, or firmware execution authority.",
        );

        let clear_trigger = self.show_probe_trigger_controls(ui);
        let row_action = self.show_probe_rows(ui);
        if self.probes.is_none() {
            ui.colored_label(egui::Color32::YELLOW, &self.probe_status);
        }

        let add = self.show_selected_probe_outputs(ui);
        ui.label(&self.probe_status);
        if clear_trigger {
            self.clear_probe_trigger();
        } else if let Some(action) = row_action {
            self.apply_probe_ui_action(action);
        } else if let Some(source) = add {
            self.add_probe(source);
        }
    }

    fn show_probe_rows(&mut self, ui: &mut egui::Ui) -> Option<ProbeUiAction> {
        let mut action = None;
        let probe_rows = self
            .probes
            .as_ref()
            .map(|probes| probes.document.probes().to_vec())
            .unwrap_or_default();
        for probe in probe_rows {
            if let Some(requested) = self.show_probe_row(ui, &probe) {
                action = Some(requested);
            }
        }
        action
    }

    fn show_probe_row(
        &mut self,
        ui: &mut egui::Ui,
        probe: &GraphProbeDefinition,
    ) -> Option<ProbeUiAction> {
        let mut action = None;
        let supports_edge_trigger = self.probes.as_ref().is_some_and(|probes| {
            probes
                .document
                .supports_edge_trigger(&self.workspace, probe.id())
                .unwrap_or(false)
        });
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.monospace(format!(
                    "p{} {} ← #{}.{} · t{} · ≤{} values / stride {}",
                    probe.id().get(),
                    probe.name(),
                    probe.source().node.get(),
                    probe.source().port.get(),
                    probe.value_type().get(),
                    probe.capture().maximum_samples(),
                    probe.capture().sample_stride()
                ));
                if ui.small_button("remove").clicked() {
                    action = Some(ProbeUiAction::Remove(probe.id()));
                }
                if supports_edge_trigger {
                    for (label, edge) in [
                        ("rise trigger", GraphProbeEdge::Rising),
                        ("fall trigger", GraphProbeEdge::Falling),
                        ("either trigger", GraphProbeEdge::Either),
                    ] {
                        if ui.small_button(label).clicked() {
                            action = Some(ProbeUiAction::SetTrigger(probe.id(), edge));
                        }
                    }
                }
            });
            let draft = self
                .probe_drafts
                .entry(probe.id())
                .or_insert_with(|| ProbeEditDraft::from_probe(probe));
            ui.horizontal_wrapped(|ui| {
                ui.label("name");
                let name_response = ui.add(
                    egui::TextEdit::singleline(&mut draft.name)
                        .desired_width(170.0)
                        .char_limit(GRAPH_PROBE_NAME_BYTES),
                );
                ui.label("retain");
                ui.add(
                    egui::DragValue::new(&mut draft.maximum_samples)
                        .range(1..=GraphProbeLimits::interactive().maximum_samples_per_probe)
                        .speed(1),
                );
                ui.label("stride");
                ui.add(
                    egui::DragValue::new(&mut draft.sample_stride)
                        .range(1..=GraphProbeLimits::interactive().maximum_sample_stride)
                        .speed(1),
                );
                let apply = ui.small_button("apply metadata").clicked()
                    || (name_response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                if apply {
                    action = Some(ProbeUiAction::ApplyMetadata(probe.id(), draft.clone()));
                }
                if ui.small_button("reset fields").clicked() {
                    action = Some(ProbeUiAction::ResetMetadata(probe.id()));
                }
            });
        });
        action
    }

    fn show_selected_probe_outputs(&self, ui: &mut egui::Ui) -> Option<WireEndpoint> {
        let mut add = None;
        if let Some(selected) = self.selected_node
            && let Some(node) = self.workspace.graph().node(selected)
        {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Selected outputs");
                for port in node.outputs() {
                    let source = WireEndpoint {
                        node: selected,
                        port: port.id(),
                    };
                    let observed = self
                        .probes
                        .as_ref()
                        .is_some_and(|probes| probes.document.observes(source));
                    if observed {
                        ui.weak(format!("{} already probed", port.name()));
                    } else if ui.small_button(format!("probe {}", port.name())).clicked() {
                        add = Some(source);
                    }
                }
            });
        }
        add
    }

    fn apply_probe_ui_action(&mut self, action: ProbeUiAction) {
        match action {
            ProbeUiAction::Remove(id) => self.remove_probe(id),
            ProbeUiAction::SetTrigger(id, edge) => self.set_probe_trigger(id, edge),
            ProbeUiAction::ApplyMetadata(id, draft) => self.apply_probe_metadata(id, &draft),
            ProbeUiAction::ResetMetadata(id) => self.reset_probe_draft(id),
        }
    }

    fn show_probe_trigger_controls(&mut self, ui: &mut egui::Ui) -> bool {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Trigger window");
            ui.label("pre");
            ui.add(
                egui::DragValue::new(&mut self.trigger_pre_samples)
                    .range(0..=1_000_000_u32)
                    .speed(1),
            );
            ui.label("post");
            ui.add(
                egui::DragValue::new(&mut self.trigger_post_samples)
                    .range(0..=1_000_000_u32)
                    .speed(1),
            );
            ui.weak("retained samples; apply with an edge button");
        });
        let Some(trigger) = self
            .probes
            .as_ref()
            .and_then(|probes| probes.document.trigger())
        else {
            ui.weak("No replay trigger is configured.");
            return false;
        };
        let mut clear = false;
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(249, 153, 82),
                format!(
                    "active: p{} {} · {} pre / {} post",
                    trigger.probe().get(),
                    probe_edge_label(trigger.edge()),
                    trigger.pretrigger_samples(),
                    trigger.posttrigger_samples()
                ),
            );
            clear = ui.small_button("clear trigger").clicked();
        });
        clear
    }

    fn add_probe(&mut self, source: WireEndpoint) {
        let mut document = match &self.probes {
            Some(probes) => probes.document.clone(),
            None => match GraphProbeDocument::try_new(
                GraphProbeLimits::interactive(),
                0,
                1,
                None,
                &self.workspace,
                Vec::new(),
            ) {
                Ok(document) => document,
                Err(error) => {
                    self.probe_status =
                        format!("probe creation rejected without mutation: {error}");
                    return;
                }
            },
        };
        let name = probe_name(source);
        let id = match document.add_probe(
            &self.workspace,
            name,
            source,
            GraphProbeCapture::new(
                u32::try_from(MAXIMUM_POINTS_PER_SERIES)
                    .expect("visible trace policy fits canonical u32"),
                1,
            ),
        ) {
            Ok(id) => id,
            Err(error) => {
                self.probe_status = format!("probe creation rejected without mutation: {error}");
                return;
            }
        };
        let encoding = match encode_graph_probes(&document) {
            Ok(encoding) => encoding,
            Err(error) => {
                self.probe_status = format!("probe encoding rejected without mutation: {error}");
                return;
            }
        };
        if let Err(error) = self.commit_probe_package(ProbePackage { document, encoding }) {
            self.probe_status = format!("probe history rejected without mutation: {error}");
            return;
        }
        self.probe_status = format!(
            "created bounded probe {} for #{}.{}; authoring only",
            id.get(),
            source.node.get(),
            source.port.get()
        );
    }

    fn apply_probe_metadata(&mut self, id: GraphProbeId, draft: &ProbeEditDraft) {
        let Some(current) = &self.probes else {
            "probe metadata edit rejected without mutation: no sidecar is attached"
                .clone_into(&mut self.probe_status);
            return;
        };
        let mut document = current.document.clone();
        if let Err(error) = document.replace_probe_metadata(
            &self.workspace,
            id,
            draft.name.clone(),
            GraphProbeCapture::new(draft.maximum_samples, draft.sample_stride),
        ) {
            self.probe_status = format!("probe metadata edit rejected without mutation: {error}");
            return;
        }
        let encoding = match encode_graph_probes(&document) {
            Ok(encoding) => encoding,
            Err(error) => {
                self.probe_status =
                    format!("probe metadata encoding rejected without mutation: {error}");
                return;
            }
        };
        let changed = match self.commit_probe_package(ProbePackage { document, encoding }) {
            Ok(changed) => changed,
            Err(error) => {
                self.probe_status =
                    format!("probe metadata history rejected without mutation: {error}");
                return;
            }
        };
        self.probe_status = format!(
            "{} p{} metadata: {} · ≤{} retained values / stride {}; authoring only",
            if changed { "updated" } else { "retained" },
            id.get(),
            draft.name,
            draft.maximum_samples,
            draft.sample_stride
        );
    }

    fn reset_probe_draft(&mut self, id: GraphProbeId) {
        let Some(probe) = self
            .probes
            .as_ref()
            .and_then(|probes| probes.document.probe(id))
        else {
            self.probe_drafts.remove(&id);
            self.probe_status = format!("probe {id:?} no longer exists; draft removed");
            return;
        };
        self.probe_drafts
            .insert(id, ProbeEditDraft::from_probe(probe));
        self.probe_status = format!(
            "reset p{} metadata fields from the unchanged canonical sidecar",
            id.get()
        );
    }

    fn remove_probe(&mut self, id: GraphProbeId) {
        let Some(current) = &self.probes else {
            "probe removal rejected without mutation: no sidecar is attached"
                .clone_into(&mut self.probe_status);
            return;
        };
        let mut document = current.document.clone();
        if let Err(error) = document.remove_probe(&self.workspace, id) {
            self.probe_status = format!("probe removal rejected without mutation: {error}");
            return;
        }
        let encoding = match encode_graph_probes(&document) {
            Ok(encoding) => encoding,
            Err(error) => {
                self.probe_status = format!("probe encoding rejected without mutation: {error}");
                return;
            }
        };
        let changed = match self.commit_probe_package(ProbePackage { document, encoding }) {
            Ok(changed) => changed,
            Err(error) => {
                self.probe_status = format!("probe history rejected without mutation: {error}");
                return;
            }
        };
        self.probe_status = format!(
            "{} probe {}; identity was not reused and any bound trigger was cleared",
            if changed { "removed" } else { "retained" },
            id.get(),
        );
    }

    fn set_probe_trigger(&mut self, id: GraphProbeId, edge: GraphProbeEdge) {
        let Some(current) = &self.probes else {
            "trigger edit rejected without mutation: no sidecar is attached"
                .clone_into(&mut self.probe_status);
            return;
        };
        let trigger = GraphProbeTrigger::new(
            id,
            edge,
            self.trigger_pre_samples,
            self.trigger_post_samples,
        );
        let mut document = current.document.clone();
        if let Err(error) = document.set_trigger(&self.workspace, trigger) {
            self.probe_status = format!("trigger edit rejected without mutation: {error}");
            return;
        }
        let encoding = match encode_graph_probes(&document) {
            Ok(encoding) => encoding,
            Err(error) => {
                self.probe_status = format!("trigger encoding rejected without mutation: {error}");
                return;
            }
        };
        let changed = match self.commit_probe_package(ProbePackage { document, encoding }) {
            Ok(changed) => changed,
            Err(error) => {
                self.probe_status = format!("trigger history rejected without mutation: {error}");
                return;
            }
        };
        self.probe_status = format!(
            "{} replay-only p{} {} trigger with {} pre / {} post samples",
            if changed { "set" } else { "retained" },
            id.get(),
            probe_edge_label(edge),
            self.trigger_pre_samples,
            self.trigger_post_samples
        );
    }

    fn clear_probe_trigger(&mut self) {
        let Some(current) = &self.probes else {
            "trigger clear rejected without mutation: no sidecar is attached"
                .clone_into(&mut self.probe_status);
            return;
        };
        let mut document = current.document.clone();
        if let Err(error) = document.clear_trigger(&self.workspace) {
            self.probe_status = format!("trigger clear rejected without mutation: {error}");
            return;
        }
        let encoding = match encode_graph_probes(&document) {
            Ok(encoding) => encoding,
            Err(error) => {
                self.probe_status = format!("trigger encoding rejected without mutation: {error}");
                return;
            }
        };
        let changed = match self.commit_probe_package(ProbePackage { document, encoding }) {
            Ok(changed) => changed,
            Err(error) => {
                self.probe_status = format!("trigger history rejected without mutation: {error}");
                return;
            }
        };
        if changed {
            "cleared replay-only trigger; probe bindings are unchanged"
                .clone_into(&mut self.probe_status);
        } else {
            "replay-only trigger was already clear; canonical ALGP is unchanged"
                .clone_into(&mut self.probe_status);
        }
    }

    #[cfg(test)]
    fn trigger_resolution(&self) -> Result<GraphProbeTriggerResolution, String> {
        let probes = self
            .probes
            .as_ref()
            .ok_or_else(|| "no canonical ALGP sidecar is attached".to_owned())?;
        exact_probe_projection(&self.fixture, &self.workspace, &probes.document)
            .map(|projection| projection.trigger_resolution())
    }

    fn trace_projection(&self) -> Result<TraceProjection, String> {
        let probes = self
            .probes
            .as_ref()
            .ok_or_else(|| "no canonical ALGP sidecar is attached".to_owned())?;
        let exact = exact_probe_projection(&self.fixture, &self.workspace, &probes.document)?;
        let traces = projected_trace_series(&exact, &probes.document, &self.workspace)?;
        let source_clocks = traces
            .iter()
            .flat_map(|series| series.points.iter().map(|point| point.clock))
            .collect::<BTreeSet<_>>();
        let time_axis = TraceTimeAxis::try_new(&traces, exact.trigger_root_window())?;
        Ok(TraceProjection {
            exact,
            traces,
            source_clocks,
            time_axis,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "mixed analog/logic/state layout, shared cursor interaction, and legend rendering remain one egui frame operation"
    )]
    fn show_trace(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Exact mixed-signal control trace");
            ui.label(
                "ALGP-projected series/window · certified analog enclosures, exact Boolean lanes, and canonical state events",
            );
        });
        let trace_projection = match self.trace_projection() {
            Ok(projection) => projection,
            Err(error) => {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("exact replay projection unavailable: {error}"),
                );
                return;
            }
        };
        let projection = &trace_projection.exact;
        let trigger_resolution = projection.trigger_resolution();
        match trigger_resolution {
            GraphProbeTriggerResolution::Disabled => {
                ui.weak(
                    "Trigger disabled; each series shows its latest bounded, decimated replay samples.",
                );
            }
            GraphProbeTriggerResolution::Waiting(trigger) => {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!(
                        "p{} {} trigger has no match; latest bounded, decimated replay shown",
                        trigger.probe().get(),
                        probe_edge_label(trigger.edge())
                    ),
                );
            }
            GraphProbeTriggerResolution::Matched(matched) => {
                ui.colored_label(
                    egui::Color32::from_rgb(249, 153, 82),
                    format!(
                        "p{} {} matched tick {} / sequence {} · exact window {}…{} · {} pre / {} post{}{}",
                        matched.trigger().probe().get(),
                        probe_edge_label(matched.trigger().edge()),
                        matched.trigger_tick(),
                        matched.trigger_sequence(),
                        matched.first_tick(),
                        matched.last_tick(),
                        matched.retained_pretrigger_samples(),
                        matched.retained_posttrigger_samples(),
                        if matched.pretrigger_complete() { "" } else { " · truncated pre" },
                        if matched.posttrigger_complete() { "" } else { " · truncated post" }
                    ),
                );
            }
        }
        if let Some((first, trigger, last)) = projection.trigger_root_window() {
            ui.monospace(format!(
                "root clock {} exact ticks {}…{} · trigger {}",
                projection.root_clock().get(),
                first,
                last,
                trigger
            ));
        }
        ui.weak(format!(
            "{} retained samples across {} probes · source clocks {}",
            projection.retained_samples(),
            projection.series().len(),
            graph_clock_set_label(&trace_projection.source_clocks)
        ));
        let trigger_root_tick = projection
            .trigger_root_window()
            .map(|(_, trigger, _)| trigger.clone());
        let traces = trace_projection.traces;
        if traces.is_empty() {
            ui.weak(
                "No attached ALGP probe retains samples in the canonical reference replay. Select a reference node output, add a probe, or revise its capture policy.",
            );
            return;
        }
        let Some(time_axis) = trace_projection.time_axis else {
            ui.colored_label(
                egui::Color32::YELLOW,
                "exact root-time axis unavailable for retained probe samples",
            );
            return;
        };
        self.cursor_root_tick = time_axis.clamp(&self.cursor_root_tick);
        let analog_groups = match analog_trace_groups(&traces) {
            Ok(groups) => groups,
            Err(error) => {
                ui.colored_label(egui::Color32::YELLOW, error);
                return;
            }
        };
        let mut digital = traces
            .iter()
            .filter(|series| series.kind == TraceSeriesKind::Boolean)
            .collect::<Vec<_>>();
        digital.sort_by_key(|series| trace_digital_order(&series.signal));
        let state = match state_trace_lanes(&traces) {
            Ok(state) => state,
            Err(error) => {
                ui.colored_label(egui::Color32::YELLOW, error);
                return;
            }
        };
        let digital_height = display_index(digital.len()) * 36.0;
        let state_height = display_index(state.len()) * STATE_TRACE_LANE_HEIGHT;
        let section_count =
            analog_groups.len() + usize::from(!digital.is_empty()) + usize::from(!state.is_empty());
        let section_gaps = display_index(section_count.saturating_sub(1)) * TRACE_SECTION_GAP;
        let surface_height = 14.0
            + display_index(analog_groups.len()) * ANALOG_TRACE_GROUP_HEIGHT
            + digital_height
            + state_height
            + section_gaps
            + 31.0;
        let width = ui.available_width().max(164.0);
        let (response, painter) = ui.allocate_painter(
            egui::vec2(width, surface_height.max(82.0)),
            egui::Sense::hover(),
        );
        let plot_left = response.rect.left() + 112.0;
        let plot_right = response.rect.right() - 12.0;
        let mut section_top = response.rect.top() + 14.0;
        let mut analog_plots = Vec::with_capacity(analog_groups.len());
        for _ in &analog_groups {
            let rect = egui::Rect::from_min_max(
                egui::pos2(plot_left, section_top),
                egui::pos2(plot_right, section_top + ANALOG_TRACE_GROUP_HEIGHT),
            );
            analog_plots.push(rect);
            section_top = rect.bottom() + TRACE_SECTION_GAP;
        }
        let digital_plot = if digital.is_empty() {
            None
        } else {
            let rect = egui::Rect::from_min_max(
                egui::pos2(plot_left, section_top),
                egui::pos2(plot_right, section_top + digital_height),
            );
            section_top = rect.bottom() + TRACE_SECTION_GAP;
            Some(rect)
        };
        let state_plot = if state.is_empty() {
            None
        } else {
            Some(egui::Rect::from_min_max(
                egui::pos2(plot_left, section_top),
                egui::pos2(plot_right, section_top + state_height),
            ))
        };

        for (index, (group, plot)) in analog_groups.iter().zip(&analog_plots).enumerate() {
            painter.rect_filled(*plot, 3.0, egui::Color32::from_rgb(17, 21, 29));
            let (minimum_value, maximum_value) = trace_value_bounds(&group.series);
            paint_trace_grid(
                &painter,
                *plot,
                &time_axis,
                minimum_value,
                maximum_value,
                digital_plot.is_none() && state_plot.is_none() && index + 1 == analog_groups.len(),
            );
            for series in &group.series {
                paint_analog_trace_series(
                    &painter,
                    *plot,
                    series,
                    &time_axis,
                    minimum_value,
                    maximum_value,
                );
            }
            painter.text(
                egui::pos2(response.rect.left() + 6.0, plot.top()),
                egui::Align2::LEFT_TOP,
                group.label(),
                egui::FontId::proportional(10.0),
                egui::Color32::GRAY,
            );
        }
        if let Some(plot) = digital_plot {
            painter.rect_filled(plot, 3.0, egui::Color32::from_rgb(17, 21, 29));
            paint_trace_time_grid(&painter, plot, &time_axis, state_plot.is_none());
            paint_digital_trace_series(&painter, plot, &digital, &time_axis);
            painter.text(
                egui::pos2(response.rect.left() + 6.0, plot.top()),
                egui::Align2::LEFT_TOP,
                "logic",
                egui::FontId::proportional(10.0),
                egui::Color32::GRAY,
            );
        }
        if let Some(plot) = state_plot {
            painter.rect_filled(plot, 3.0, egui::Color32::from_rgb(17, 21, 29));
            paint_trace_time_grid(&painter, plot, &time_axis, true);
            paint_state_trace_series(&painter, plot, &state, &time_axis);
            painter.text(
                egui::pos2(response.rect.left() + 6.0, plot.top()),
                egui::Align2::LEFT_TOP,
                "state / events",
                egui::FontId::proportional(10.0),
                egui::Color32::GRAY,
            );
        }

        let time_plot = analog_plots
            .first()
            .copied()
            .or(digital_plot)
            .or(state_plot)
            .expect("one trace section exists");
        let last_plot = state_plot
            .or(digital_plot)
            .or_else(|| analog_plots.last().copied())
            .expect("one trace section exists");
        let interaction = time_plot.union(last_plot);
        if let Some(pointer) = response
            .hover_pos()
            .filter(|position| interaction.contains(*position))
            && let Some(tick) = time_axis.nearest_tick(time_plot, pointer.x)
        {
            self.cursor_root_tick = tick;
        }
        if let Some(trigger_x) = trigger_root_tick
            .as_ref()
            .and_then(|tick| time_axis.plot_x(time_plot, tick))
        {
            painter.line_segment(
                [
                    egui::pos2(trigger_x, interaction.top()),
                    egui::pos2(trigger_x, interaction.bottom()),
                ],
                egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(249, 153, 82)),
            );
            painter.text(
                egui::pos2(trigger_x + 4.0, interaction.top() + 4.0),
                egui::Align2::LEFT_TOP,
                "TRIGGER",
                egui::FontId::monospace(9.0),
                egui::Color32::from_rgb(249, 153, 82),
            );
        }
        if let Some(cursor_x) = time_axis.plot_x(time_plot, &self.cursor_root_tick) {
            painter.line_segment(
                [
                    egui::pos2(cursor_x, interaction.top()),
                    egui::pos2(cursor_x, interaction.bottom()),
                ],
                egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(110)),
            );
        }

        painter.text(
            egui::pos2(time_plot.left(), response.rect.bottom() - 8.0),
            egui::Align2::LEFT_BOTTOM,
            format!("root clock {} exact tick", projection.root_clock().get()),
            egui::FontId::proportional(10.0),
            egui::Color32::GRAY,
        );

        let hierarchy_component = self.component.as_ref();
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("root tick {}", self.cursor_root_tick));
            for series in analog_groups
                .iter()
                .flat_map(|group| group.series.iter().copied())
                .chain(digital.iter().copied())
                .chain(state.iter().copied())
            {
                if let Some(point) = trace_point_at_or_before(series, &self.cursor_root_tick) {
                    let value = trace_cursor_label(&series.signal, point);
                    let label = hierarchy_component
                        .and_then(|component| {
                            hierarchy_endpoint_correlation(component, series.signal.source)
                        })
                        .map_or(value.clone(), |origin| format!("{origin} · {value}"));
                    ui.colored_label(trace_signal_color(&series.signal), label);
                }
            }
        });
    }
}

impl TargetResourceProof {
    fn catalog_selection_choices(&self) -> Vec<(String, String)> {
        self.catalog
            .entries()
            .iter()
            .map(|entry| {
                (
                    format!(
                        "{} · {} v{}",
                        graph_resource_label(entry.resource().resource),
                        short_kind(entry.kind().name()),
                        entry.kind().version()
                    ),
                    format!(
                        "{} · {:?}",
                        graph_resource_label(entry.resource().resource),
                        entry.resource().support
                    ),
                )
            })
            .collect()
    }

    fn show_catalog_selector(&mut self, ui: &mut egui::Ui) {
        let choices = self.catalog_selection_choices();
        let selected = choices
            .get(self.catalog_index)
            .map_or("resource unavailable", |choice| choice.0.as_str());
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("tinybee_resource_catalog")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (index, (_, choice)) in choices.iter().enumerate() {
                        ui.selectable_value(&mut self.catalog_index, index, choice);
                    }
                });
            if ui.small_button("reset target draft").clicked() {
                self.reset();
            }
        });
    }

    fn selected_is_present(&self) -> bool {
        self.workspace
            .graph()
            .nodes()
            .iter()
            .flat_map(NodeDefinition::parameters)
            .any(|parameter| {
                graph_value_contains_catalog_resource(
                    &self.catalog,
                    parameter.value().value(),
                    self.catalog_index,
                )
            })
    }

    fn selected_reference_entry(&self) -> Option<usize> {
        let slot = TARGET_RESOURCE_REFERENCE_SLOTS.get(self.reference_slot)?;
        self.workspace
            .graph()
            .node(self.pair_node)
            .and_then(|node| {
                target_resource_handle_at_path(
                    node,
                    self.registry.semantic_registry().context_schema(),
                    slot.path,
                )
            })
            .and_then(|handle| self.catalog.entry_index_for_handle(handle))
    }

    fn show_reference_selector(&mut self, ui: &mut egui::Ui) {
        if self.reference_slot >= TARGET_RESOURCE_REFERENCE_SLOTS.len() {
            self.reference_slot = 0;
        }
        egui::ComboBox::from_id_salt("tinybee_resource_reference_slot")
            .selected_text(format!(
                "composite leaf · {}",
                TARGET_RESOURCE_REFERENCE_SLOTS[self.reference_slot].label
            ))
            .show_ui(ui, |ui| {
                for (index, slot) in TARGET_RESOURCE_REFERENCE_SLOTS.iter().enumerate() {
                    ui.selectable_value(
                        &mut self.reference_slot,
                        index,
                        format!("composite leaf · {}", slot.label),
                    );
                }
            });
        let slot = TARGET_RESOURCE_REFERENCE_SLOTS[self.reference_slot];
        let current_entry = self.selected_reference_entry();
        let current = current_entry
            .and_then(|entry| self.catalog.entries().get(entry))
            .map_or_else(
                || "unresolved catalog identity".to_owned(),
                |entry| graph_resource_label(entry.resource().resource),
            );
        ui.monospace(format!(
            "#{} capability references · {} = {} · {} segment(s)",
            self.pair_node.get(),
            slot.label,
            current,
            slot.path.len()
        ));
        let target_is_current = current_entry == Some(self.catalog_index);
        let target_is_present = self.selected_is_present();
        let can_rebind = current_entry.is_some() && !target_is_current && !target_is_present;
        let mut rebind = false;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Executable paired stable inputs");
            rebind = ui
                .add_enabled(can_rebind, egui::Button::new("rebind composite leaf"))
                .on_disabled_hover_text(if current_entry.is_none() {
                    "the reviewed paired-input field is unavailable"
                } else if target_is_current {
                    "the leaf already carries this exact catalog entry"
                } else {
                    "the other paired-input field already carries this resource identity"
                })
                .clicked();
            ui.monospace(format!("stable value path: resources.{}", slot.label));
        });
        if rebind {
            self.rebind_reference();
        }
    }

    fn show_replay_evidence_files(
        &mut self,
        ui: &mut egui::Ui,
        success_bridge: &mut BoundedFileBridge,
        fault_bridge: &mut BoundedFileBridge,
    ) {
        let actor = &self.deployment.actor_replay;
        let success_name = format!(
            "alumina-{}-success.algrrep",
            digest_prefix(actor.success_evidence.digest().0)
        );
        let success_events = success_bridge.show(
            ui,
            actor.success_evidence.encoded(),
            MAX_GRAPH_DEPLOYMENT_REPLAY_EVIDENCE_BYTES,
            &success_name,
            ALGR_SUCCESS_REPLAY_FILE,
        );
        let fault_name = format!(
            "alumina-{}-fault.algrrep",
            digest_prefix(actor.fault_evidence.digest().0)
        );
        let fault_events = fault_bridge.show(
            ui,
            actor.fault_evidence.encoded(),
            MAX_GRAPH_DEPLOYMENT_REPLAY_EVIDENCE_BYTES,
            &fault_name,
            ALGR_FAULT_REPLAY_FILE,
        );

        let mut file_status = None;
        for event in success_events {
            file_status = Some(target_replay_file_event_status(
                event,
                "success",
                &actor.success_evidence,
                &self.deployment.report,
                self.catalog.target(),
            ));
        }
        for event in fault_events {
            file_status = Some(target_replay_file_event_status(
                event,
                "fault",
                &actor.fault_evidence,
                &self.deployment.report,
                self.catalog.target(),
            ));
        }
        if let Some(status) = file_status {
            self.status = status;
        }
        ui.weak(format!(
            "canonical actor transcripts · success {} bytes · fault {} bytes · imports rerun fresh actors and mutate no draft",
            actor.success_evidence.encoded().len(),
            actor.fault_evidence.encoded().len(),
        ));
    }

    fn rebind_reference(&mut self) {
        let slot = TARGET_RESOURCE_REFERENCE_SLOTS[self.reference_slot];
        let previous = self
            .selected_reference_entry()
            .and_then(|index| self.catalog.entries().get(index))
            .map_or_else(
                || "unresolved resource".to_owned(),
                |entry| graph_resource_label(entry.resource().resource),
            );
        let selected = self.catalog.entries().get(self.catalog_index).map_or_else(
            || "unavailable resource".to_owned(),
            |entry| graph_resource_label(entry.resource().resource),
        );
        let mut candidate = self.workspace.clone();
        match select_graph_capability_node_resource(
            &self.catalog,
            &self.registry,
            &mut candidate,
            self.pair_node,
            TARGET_RESOURCE_PARAMETER,
            slot.path,
            self.catalog_index,
        ) {
            Ok(encoding) => {
                let deployment = match validate_target_resource_workspace(
                    &self.catalog,
                    &self.registry,
                    &candidate,
                ) {
                    Ok(deployment) => deployment,
                    Err(error) => {
                        self.status =
                            format!("paired resource selection rejected without mutation: {error}");
                        return;
                    }
                };
                self.workspace = candidate;
                self.encoding = encoding;
                self.deployment = deployment;
                self.status = format!(
                    "rebound executable pair #{} at resources.{} from {previous} to {selected}; offline ALGR changed and firmware session authority remains closed",
                    self.pair_node.get(),
                    slot.label,
                );
            }
            Err(error) => {
                self.status =
                    format!("paired resource selection rejected without mutation: {error}");
            }
        }
    }

    fn reset(&mut self) {
        match initial_target_resource_workspace(&self.catalog, &self.registry) {
            Ok((workspace, encoding, pair_node, deployment)) => {
                self.workspace = workspace;
                self.encoding = encoding;
                self.deployment = deployment;
                self.pair_node = pair_node;
                self.reference_slot = 0;
                self.catalog_index = 3;
                "reset executable target-I/O draft and offline ALGR; nothing was sent to firmware"
                    .clone_into(&mut self.status);
            }
            Err(error) => {
                self.status = format!("target-I/O reset failed without mutation: {error}");
            }
        }
    }
}

fn target_replay_file_event_status(
    event: BoundedFileEvent,
    kind: &str,
    expected: &CanonicalGraphDeploymentReplayEvidence1,
    report: &GraphDeploymentReport,
    target: GraphDeploymentTarget,
) -> String {
    match event {
        BoundedFileEvent::Import(Ok(bytes)) => {
            match verify_graph_deployment_evidence_bytes(
                &bytes,
                expected.digest(),
                report,
                target,
                board_mks_tinybee::PACKAGE.graph.opcodes,
                board_mks_tinybee::PACKAGE.graph.resources,
                GraphDeploymentReplayLimits::interactive(),
            ) {
                Ok(replay) if replay.evidence() == expected => format!(
                    "imported {} canonical {kind} ALGRREP1 bytes after fresh fixed-memory firmware-actor replay; no draft or session changed",
                    bytes.len()
                ),
                Ok(_) => format!(
                    "{kind} ALGRREP1 import rejected without mutation: fresh replay did not reproduce the current artifact"
                ),
                Err(error) => format!("{kind} ALGRREP1 import rejected without mutation: {error}"),
            }
        }
        BoundedFileEvent::Import(Err(error)) => {
            format!("{kind} ALGRREP1 file read rejected: {error}")
        }
        BoundedFileEvent::Export(Ok(bytes)) => {
            format!("exported {bytes} exact canonical {kind} ALGRREP1 bytes")
        }
        BoundedFileEvent::Export(Err(error)) => {
            format!("{kind} ALGRREP1 export failed: {error}")
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the offline proof keeps its exact target schema, clocks, three reviewed implementations, capability derivation, and initial executable pair visibly co-located"
)]
fn tinybee_resource_proof() -> Result<TargetResourceProof, String> {
    const DEVICE: DeviceId = DeviceId([0x54; 16]);
    const ROOT: GraphClockId = GraphClockId::new(1);
    const INPUT_CLOCK: GraphClockId = GraphClockId::new(2);

    let resource_class =
        ResourceClassId::new(board_mks_tinybee::GRAPH_STABLE_BOOLEAN_INPUT_CLASS.get());
    let schema = GraphSchema::try_new(
        GraphLimits::interactive(),
        Vec::new(),
        vec![
            TypeDefinition::new(TARGET_RESOURCE_BOOL_TYPE, "core.bool", TypeKind::Boolean),
            TypeDefinition::new(
                TARGET_RESOURCE_STREAM_TYPE,
                "tinybee.stream.stable-bool",
                TypeKind::Stream {
                    sample: TARGET_RESOURCE_BOOL_TYPE,
                    clock: INPUT_CLOCK,
                    capacity: 1,
                },
            ),
            TypeDefinition::new(
                TARGET_RESOURCE_HANDLE_TYPE,
                "tinybee.resource.stable-bool",
                TypeKind::ResourceHandle {
                    class: resource_class,
                },
            ),
            TypeDefinition::new(
                TARGET_RESOURCE_PAIR_TYPE,
                "tinybee.resource.stable-bool-pair",
                TypeKind::Record {
                    fields: vec![
                        RecordField::new(
                            TARGET_RESOURCE_FIRST_FIELD,
                            "permit",
                            TARGET_RESOURCE_HANDLE_TYPE,
                        ),
                        RecordField::new(
                            TARGET_RESOURCE_SECOND_FIELD,
                            "interlock",
                            TARGET_RESOURCE_HANDLE_TYPE,
                        ),
                    ],
                },
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let clocks = vec![
        ClockDefinition::new(
            ROOT,
            "tinybee.reference.cpu",
            ClockKind::DeviceCycle {
                device_id: DEVICE,
                ticks_per_second: 240_000_000,
            },
        ),
        ClockDefinition::new(
            INPUT_CLOCK,
            "tinybee.reference.input-1khz",
            ClockKind::Derived {
                source: ROOT,
                numerator: 1,
                denominator: 240_000,
            },
        ),
    ];
    let context = GraphDocument::try_new(0, schema, clocks, Vec::new(), Vec::new())
        .map_err(|error| error.to_string())?;
    let scalar_kind = NodeKind::new(TARGET_RESOURCE_SCALAR_KIND_NAME, 1);
    let pair_kind = NodeKind::new(TARGET_RESOURCE_PAIR_KIND_NAME, 1);
    let sink_kind = NodeKind::new(TARGET_RESOURCE_SINK_KIND_NAME, 1);
    let output = PortDefinition::new(GraphPortId::new(1), "samples", TARGET_RESOURCE_STREAM_TYPE);
    let semantic = GraphNodeRegistry::try_new(
        GraphAnalysisLimits::interactive(),
        &context,
        vec![
            NodeSchema::new(
                scalar_kind.clone(),
                ExecutionDomainSet::REALTIME,
                Vec::new(),
                Vec::new(),
                vec![output.clone()],
                vec![NodeParameterContract::new(
                    TARGET_RESOURCE_PARAMETER,
                    "resource",
                    TARGET_RESOURCE_HANDLE_TYPE,
                )],
                vec![NodeOutputDependency::new(GraphPortId::new(1), Vec::new())],
                Vec::new(),
                None,
            ),
            NodeSchema::new(
                pair_kind.clone(),
                ExecutionDomainSet::REALTIME,
                Vec::new(),
                Vec::new(),
                vec![output.clone()],
                vec![NodeParameterContract::new(
                    TARGET_RESOURCE_PARAMETER,
                    "resources",
                    TARGET_RESOURCE_PAIR_TYPE,
                )],
                vec![NodeOutputDependency::new(GraphPortId::new(1), Vec::new())],
                Vec::new(),
                None,
            ),
            NodeSchema::new(
                sink_kind.clone(),
                ExecutionDomainSet::REALTIME,
                vec![PortDefinition::new(
                    GraphPortId::new(1),
                    "samples",
                    TARGET_RESOURCE_STREAM_TYPE,
                )],
                vec![NodeInputChannelContract::new(
                    GraphPortId::new(1),
                    InputConnectionRequirement::Required,
                    NodeInputChannelKind::StreamQueue {
                        capacity: 1,
                        full_policy: ChannelFullPolicy::Fault,
                    },
                )],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let registry = GraphDeploymentRegistry::try_new(
        semantic,
        vec![
            GraphDeploymentImplementation::new(
                scalar_kind,
                GraphDeploymentNodeKind::StableBooleanInput {
                    output: GraphPortId::new(1),
                    resource_parameter: TARGET_RESOURCE_PARAMETER,
                },
                INPUT_CLOCK,
                100,
            ),
            GraphDeploymentImplementation::new(
                pair_kind,
                GraphDeploymentNodeKind::StableBooleanPairAll {
                    output: GraphPortId::new(1),
                    resource_parameter: TARGET_RESOURCE_PARAMETER,
                    first_field: TARGET_RESOURCE_FIRST_FIELD,
                    second_field: TARGET_RESOURCE_SECOND_FIELD,
                },
                INPUT_CLOCK,
                120,
            ),
            GraphDeploymentImplementation::new(
                sink_kind,
                GraphDeploymentNodeKind::BooleanStreamSink {
                    input: GraphPortId::new(1),
                },
                INPUT_CLOCK,
                40,
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let capability_document = tinybee_capability_document()?;
    let target = GraphDeploymentTarget {
        device_id: DEVICE,
        capability_digest: board_mks_tinybee::PACKAGE.board.capability_digest,
        config_digest: Digest([0x43; 32]),
    };
    let catalog = derive_graph_capability_node_catalog(
        &capability_document,
        target,
        &registry,
        GraphCapabilityCatalogLimits::interactive(),
    )
    .map_err(|error| error.to_string())?;
    if catalog.entries().len() != 4 {
        return Err(format!(
            "TinyBee reference capability produced {} resource nodes instead of four",
            catalog.entries().len()
        ));
    }
    let (workspace, encoding, pair_node, deployment) =
        initial_target_resource_workspace(&catalog, &registry)?;
    Ok(TargetResourceProof {
        catalog,
        registry,
        workspace,
        encoding,
        deployment,
        catalog_index: 3,
        pair_node,
        reference_slot: 0,
        status: "digest-verified offline catalog lowered an executable GPIO22/GPIO32 pair; no firmware session exists"
            .to_owned(),
    })
}

fn tinybee_capability_document() -> Result<Vec<u8>, String> {
    let package = &board_mks_tinybee::PACKAGE;
    let identity = calculate_identity(package).map_err(|error| format!("{error:?}"))?;
    let mut document = vec![
        0_u8;
        usize::try_from(identity.byte_len).map_err(|_| {
            "TinyBee capability length exceeds host usize".to_owned()
        })?
    ];
    let mut offset = 0_u32;
    while offset < identity.byte_len {
        let mut chunk = [0_u8; MAX_CAPABILITY_CHUNK_BYTES];
        let read = read_verified_range(package, offset, &mut chunk)
            .map_err(|error| format!("{error:?}"))?;
        if read.byte_len == 0 {
            return Err("TinyBee capability range encoder made no progress".to_owned());
        }
        let start = usize::try_from(offset)
            .map_err(|_| "TinyBee capability offset exceeds host usize".to_owned())?;
        let count = usize::from(read.byte_len);
        document[start..start + count].copy_from_slice(&chunk[..count]);
        offset = offset
            .checked_add(u32::from(read.byte_len))
            .ok_or_else(|| "TinyBee capability offset overflowed".to_owned())?;
    }
    Ok(document)
}

fn target_catalog_resource_handle(
    catalog: &GraphCapabilityNodeCatalog,
    entry: usize,
) -> Result<ResourceGraphHandle, String> {
    let parameter = catalog
        .entries()
        .get(entry)
        .and_then(|entry| entry.resource_parameter())
        .ok_or_else(|| format!("TinyBee resource catalog entry {entry} is unavailable"))?;
    match parameter.value().value() {
        GraphValue::ResourceHandle(handle) => Ok(*handle),
        _ => Err(format!(
            "TinyBee resource catalog entry {entry} is not a resource handle"
        )),
    }
}

fn target_resource_pair_prototype(
    catalog: &GraphCapabilityNodeCatalog,
    registry: &GraphDeploymentRegistry,
) -> Result<GraphNodePrototype, String> {
    let value = TypedGraphValue::try_new(
        registry.semantic_registry().context_schema(),
        TARGET_RESOURCE_PAIR_TYPE,
        GraphValue::Record(vec![
            RecordValueField {
                field: TARGET_RESOURCE_FIRST_FIELD,
                value: GraphValue::ResourceHandle(target_catalog_resource_handle(catalog, 0)?),
            },
            RecordValueField {
                field: TARGET_RESOURCE_SECOND_FIELD,
                value: GraphValue::ResourceHandle(target_catalog_resource_handle(catalog, 1)?),
            },
        ]),
    )
    .map_err(|error| error.to_string())?;
    Ok(GraphNodePrototype::new(
        NodeKind::new(TARGET_RESOURCE_PAIR_KIND_NAME, 1),
        "Paired stable safety inputs",
        ExecutionDomain::Realtime {
            device_id: catalog.target().device_id,
        },
        Vec::new(),
        vec![PortDefinition::new(
            GraphPortId::new(1),
            "samples",
            TARGET_RESOURCE_STREAM_TYPE,
        )],
        vec![NodeParameter::new(
            TARGET_RESOURCE_PARAMETER,
            "resources",
            value,
        )],
    ))
}

fn target_resource_sink_prototype(catalog: &GraphCapabilityNodeCatalog) -> GraphNodePrototype {
    GraphNodePrototype::new(
        NodeKind::new(TARGET_RESOURCE_SINK_KIND_NAME, 1),
        "Paired-input proof sink",
        ExecutionDomain::Realtime {
            device_id: catalog.target().device_id,
        },
        vec![PortDefinition::new(
            GraphPortId::new(1),
            "samples",
            TARGET_RESOURCE_STREAM_TYPE,
        )],
        Vec::new(),
        Vec::new(),
    )
}

fn initial_target_resource_workspace(
    catalog: &GraphCapabilityNodeCatalog,
    registry: &GraphDeploymentRegistry,
) -> Result<
    (
        GraphWorkspaceDocument,
        CanonicalGraphWorkspaceEncoding,
        GraphNodeId,
        TargetResourceDeployment,
    ),
    String,
> {
    let mut workspace = empty_target_workspace(registry)?;
    let pair_node = workspace
        .create_node(target_resource_pair_prototype(catalog, registry)?, 28, 28)
        .map_err(|error| error.to_string())?;
    let sink_node = workspace
        .create_node(target_resource_sink_prototype(catalog), 328, 28)
        .map_err(|error| error.to_string())?;
    workspace
        .connect(
            WireEndpoint {
                node: pair_node,
                port: GraphPortId::new(1),
            },
            WireEndpoint {
                node: sink_node,
                port: GraphPortId::new(1),
            },
        )
        .map_err(|error| error.to_string())?;
    let deployment = validate_target_resource_workspace(catalog, registry, &workspace)?;
    let encoding = encode_graph_workspace(&workspace).map_err(|error| error.to_string())?;
    Ok((workspace, encoding, pair_node, deployment))
}

fn target_resource_handle_at_path(
    node: &NodeDefinition,
    schema: &GraphSchema,
    path: &[GraphValuePathSegment],
) -> Option<ResourceGraphHandle> {
    let parameter = node
        .parameters()
        .iter()
        .find(|parameter| parameter.id() == TARGET_RESOURCE_PARAMETER)?;
    let (value_type, value) = parameter.value().value_at_path(schema, path).ok()?;
    if !matches!(
        schema.value_type(value_type)?.kind(),
        TypeKind::ResourceHandle { .. }
    ) {
        return None;
    }
    match value {
        GraphValue::ResourceHandle(handle) => Some(*handle),
        _ => None,
    }
}

fn collect_resource_handles(value: &GraphValue, handles: &mut Vec<ResourceGraphHandle>) {
    match value {
        GraphValue::Array(values) => {
            for value in values {
                collect_resource_handles(value, handles);
            }
        }
        GraphValue::Record(fields) => {
            for field in fields {
                collect_resource_handles(&field.value, handles);
            }
        }
        GraphValue::OptionSome(value)
        | GraphValue::ResultOk(value)
        | GraphValue::ResultError(value) => collect_resource_handles(value, handles),
        GraphValue::ResourceHandle(handle) => handles.push(*handle),
        GraphValue::Boolean(_)
        | GraphValue::ExactRational(_)
        | GraphValue::MeasurementInterval { .. }
        | GraphValue::CanonicalI64(_)
        | GraphValue::CanonicalU64(_)
        | GraphValue::Text(_)
        | GraphValue::Bytes(_)
        | GraphValue::OptionNone
        | GraphValue::JobHandle(_) => {}
    }
}

fn graph_value_contains_catalog_resource(
    catalog: &GraphCapabilityNodeCatalog,
    value: &GraphValue,
    entry: usize,
) -> bool {
    let Ok(selected) = target_catalog_resource_handle(catalog, entry) else {
        return false;
    };
    let mut handles = Vec::new();
    collect_resource_handles(value, &mut handles);
    handles.contains(&selected)
}

#[allow(
    clippy::too_many_lines,
    reason = "one fail-closed transaction validates graph shape, catalog provenance, uniqueness, and the independently decoded lowering result"
)]
fn validate_target_resource_workspace(
    catalog: &GraphCapabilityNodeCatalog,
    registry: &GraphDeploymentRegistry,
    workspace: &GraphWorkspaceDocument,
) -> Result<TargetResourceDeployment, String> {
    analyze_graph_draft(workspace.graph(), registry.semantic_registry())
        .map_err(|error| error.to_string())?;
    let pair_nodes = workspace
        .graph()
        .nodes()
        .iter()
        .filter(|node| {
            node.kind().name() == TARGET_RESOURCE_PAIR_KIND_NAME && node.kind().version() == 1
        })
        .collect::<Vec<_>>();
    let sink_nodes = workspace
        .graph()
        .nodes()
        .iter()
        .filter(|node| {
            node.kind().name() == TARGET_RESOURCE_SINK_KIND_NAME && node.kind().version() == 1
        })
        .collect::<Vec<_>>();
    if workspace.graph().nodes().len() != 2 || pair_nodes.len() != 1 || sink_nodes.len() != 1 {
        return Err(format!(
            "target resource draft must contain exactly one paired input and one sink, found {} node(s), {} pair(s), and {} sink(s)",
            workspace.graph().nodes().len(),
            pair_nodes.len(),
            sink_nodes.len(),
        ));
    }
    let [wire] = workspace.graph().wires() else {
        return Err(format!(
            "target resource draft has {} wires instead of one",
            workspace.graph().wires().len()
        ));
    };
    if wire.source()
        != (WireEndpoint {
            node: pair_nodes[0].id(),
            port: GraphPortId::new(1),
        })
        || wire.target()
            != (WireEndpoint {
                node: sink_nodes[0].id(),
                port: GraphPortId::new(1),
            })
    {
        return Err(
            "target resource draft wire does not connect pair output to proof sink".to_owned(),
        );
    }

    let mut selected_resources = Vec::with_capacity(TARGET_RESOURCE_REFERENCE_SLOTS.len());
    for slot in TARGET_RESOURCE_REFERENCE_SLOTS {
        let handle = target_resource_handle_at_path(
            pair_nodes[0],
            registry.semantic_registry().context_schema(),
            slot.path,
        )
        .ok_or_else(|| {
            format!(
                "target resource pair has no typed resources.{} field",
                slot.label
            )
        })?;
        let resource = catalog
            .entry_index_for_handle(handle)
            .and_then(|index| catalog.entries().get(index))
            .map(|entry| entry.resource().resource)
            .ok_or_else(|| {
                format!(
                    "target resource resources.{} carries a raw or foreign identity",
                    slot.label
                )
            })?;
        selected_resources.push(resource);
    }

    let mut retained = Vec::new();
    for node in workspace.graph().nodes() {
        for parameter in node.parameters() {
            let mut handles = Vec::new();
            collect_resource_handles(parameter.value().value(), &mut handles);
            for handle in handles {
                if catalog.entry_index_for_handle(handle).is_none() {
                    return Err(format!(
                        "target resource node #{} parameter {} carries a raw or foreign identity",
                        node.id().get(),
                        parameter.id()
                    ));
                }
                if retained.contains(&handle) {
                    return Err(format!(
                        "target resource node #{} parameter {} duplicates a physical identity",
                        node.id().get(),
                        parameter.id()
                    ));
                }
                retained.push(handle);
            }
        }
    }

    let [first, second] = selected_resources.as_slice() else {
        return Err(
            "target resource pair did not retain exactly two selected resources".to_owned(),
        );
    };
    lower_target_resource_workspace(catalog, registry, workspace, (*first, *second))
}

fn lower_target_resource_workspace(
    catalog: &GraphCapabilityNodeCatalog,
    registry: &GraphDeploymentRegistry,
    workspace: &GraphWorkspaceDocument,
    expected_pair: (ResourceId, ResourceId),
) -> Result<TargetResourceDeployment, String> {
    let capability_document = tinybee_capability_document()?;
    let limits =
        GraphDeploymentLimits::from_capability_document(&capability_document, 1_000_000, 100)
            .map_err(|error| error.to_string())?;
    if limits.capability_identity() != catalog.capability_identity() {
        return Err(
            "target lowering limits do not retain the catalog capability identity".to_owned(),
        );
    }
    let report = lower_graph_deployment(workspace.graph(), registry, catalog.target(), &limits)
        .map_err(|error| error.to_string())?;
    let package = report.package();
    let lowered_nodes = package.nodes().collect::<Vec<_>>();
    if lowered_nodes.len() != 2
        || lowered_nodes[0].opcode != GraphIrOpcode::StableBooleanPairAll
        || lowered_nodes[1].opcode != GraphIrOpcode::BooleanStreamSink
    {
        return Err(
            "target resource draft did not lower to the exact pair-then-sink opcode sequence"
                .to_owned(),
        );
    }
    let lowered_pair = decode_graph_resource_pair_parameter(lowered_nodes[0].parameter)
        .map_err(|error| format!("lowered target resource pair is not canonical: {error:?}"))?;
    if lowered_pair != expected_pair {
        return Err(format!(
            "lowered target resource pair {lowered_pair:?} does not preserve stable field order {expected_pair:?}"
        ));
    }
    let realtime = package.header().realtime_schedule;
    let actor_replay = replay_target_resource_deployment(
        &report,
        catalog.target(),
        lowered_pair,
        realtime.period_cycles,
    )?;
    Ok(TargetResourceDeployment {
        package_digest: package.digest(),
        implementation_digest: report.implementation_digest(),
        package_bytes: package.bytes().len(),
        first: lowered_pair.0,
        second: lowered_pair.1,
        realtime_period_cycles: realtime.period_cycles,
        realtime_wcet_cycles: realtime.total_wcet_cycles,
        actor_replay,
        report,
    })
}

fn replay_target_resource_deployment(
    report: &GraphDeploymentReport,
    target: GraphDeploymentTarget,
    pair: (ResourceId, ResourceId),
    period_cycles: u64,
) -> Result<TargetResourceActorReplay, String> {
    if period_cycles == 0 {
        return Err("target resource package has a zero Realtime period".to_owned());
    }
    let start = DeviceCycle(period_cycles);
    let (success_evidence, truth_table, ordered_reads) =
        replay_target_resource_success(report, target, pair, period_cycles, start)?;
    let (fault_evidence, fault_reads, terminal_fault) =
        replay_target_resource_fault(report, target, pair, start)?;
    Ok(TargetResourceActorReplay {
        success_evidence,
        fault_evidence,
        truth_table,
        ordered_reads,
        fault_reads,
        terminal_fault,
    })
}

fn replay_target_resource_success(
    report: &GraphDeploymentReport,
    target: GraphDeploymentTarget,
    pair: (ResourceId, ResourceId),
    period_cycles: u64,
    start: DeviceCycle,
) -> Result<
    (
        CanonicalGraphDeploymentReplayEvidence1,
        [bool; 4],
        [ResourceId; 2],
    ),
    String,
> {
    let truth_inputs = [(false, false), (false, true), (true, false), (true, true)]
        .into_iter()
        .enumerate()
        .map(|(index, (first, second))| {
            let release = u64::try_from(index + 1)
                .map_err(|_| "target resource replay release index overflowed".to_owned())?;
            let cycle = period_cycles
                .checked_mul(release)
                .map(DeviceCycle)
                .ok_or_else(|| "target resource replay cycle overflowed".to_owned())?;
            Ok(GraphDeploymentReplayInput::new(
                cycle,
                true,
                vec![
                    GraphDeploymentResourceSample::new(pair.1, Some(second)),
                    GraphDeploymentResourceSample::new(pair.0, Some(first)),
                ],
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let success = replay_realtime_graph_deployment(
        report,
        target,
        board_mks_tinybee::PACKAGE.graph.opcodes,
        board_mks_tinybee::PACKAGE.graph.resources,
        1,
        1,
        start,
        &truth_inputs,
        GraphDeploymentReplayLimits::interactive(),
    )
    .map_err(|error| format!("target resource firmware-actor success replay failed: {error}"))?;
    if !success.complete()
        || success.requested_releases() != truth_inputs.len()
        || success.releases().len() != truth_inputs.len()
    {
        return Err(
            "target resource firmware actors did not complete the four-case replay".to_owned(),
        );
    }

    let expected_reads = [pair.0, pair.1];
    let mut truth_table = [false; 4];
    for (index, release) in success.releases().iter().enumerate() {
        if release.reads() != expected_reads {
            return Err(format!(
                "target resource firmware actor release {index} read {:?} instead of {:?}",
                release.reads(),
                expected_reads,
            ));
        }
        truth_table[index] = match release.outcome() {
            GraphDeploymentReplayReleaseOutcome::Completed(release) => {
                release.last_sink_value.ok_or_else(|| {
                    format!("target resource firmware actor release {index} produced no sink value")
                })?
            }
            GraphDeploymentReplayReleaseOutcome::Faulted(error) => {
                return Err(format!(
                    "target resource firmware actor release {index} faulted unexpectedly: {error:?}"
                ));
            }
        };
    }
    if truth_table != [false, false, false, true] {
        return Err(format!(
            "target resource firmware actor produced an invalid conjunction table {truth_table:?}"
        ));
    }
    Ok((success.evidence().clone(), truth_table, expected_reads))
}

fn replay_target_resource_fault(
    report: &GraphDeploymentReport,
    target: GraphDeploymentTarget,
    pair: (ResourceId, ResourceId),
    start: DeviceCycle,
) -> Result<
    (
        CanonicalGraphDeploymentReplayEvidence1,
        [ResourceId; 2],
        GraphExecutionFault,
    ),
    String,
> {
    let expected_reads = [pair.0, pair.1];
    let fault_input = [GraphDeploymentReplayInput::new(
        start,
        true,
        vec![
            GraphDeploymentResourceSample::new(pair.1, Some(true)),
            GraphDeploymentResourceSample::new(pair.0, None),
        ],
    )];
    let fault = replay_realtime_graph_deployment(
        report,
        target,
        board_mks_tinybee::PACKAGE.graph.opcodes,
        board_mks_tinybee::PACKAGE.graph.resources,
        2,
        1,
        start,
        &fault_input,
        GraphDeploymentReplayLimits::interactive(),
    )
    .map_err(|error| format!("target resource firmware-actor fault replay failed: {error}"))?;
    let [fault_release] = fault.releases() else {
        return Err(format!(
            "target resource firmware-actor fault replay attempted {} release(s) instead of one",
            fault.releases().len()
        ));
    };
    let fault_reads: [ResourceId; 2] = fault_release.reads().try_into().map_err(|_| {
        format!(
            "target resource firmware-actor fault replay retained {} read(s) instead of two",
            fault_release.reads().len()
        )
    })?;
    if fault.complete() || fault.requested_releases() != 1 || fault_reads != expected_reads {
        return Err(
            "target resource firmware actors did not preserve the ordered fault replay".to_owned(),
        );
    }
    let terminal_fault = match fault_release.outcome() {
        GraphDeploymentReplayReleaseOutcome::Faulted(error)
            if error.observation.fault == GraphExecutionFault::ResourceUnavailable =>
        {
            error.observation.fault
        }
        GraphDeploymentReplayReleaseOutcome::Faulted(error) => {
            return Err(format!(
                "target resource firmware actor produced the wrong terminal fault: {error:?}"
            ));
        }
        GraphDeploymentReplayReleaseOutcome::Completed(_) => {
            return Err(
                "target resource firmware actor completed the unavailable-input release".to_owned(),
            );
        }
    };
    Ok((fault.evidence().clone(), fault_reads, terminal_fault))
}

fn tinybee_board_explorer() -> Result<BoardExplorerPanel, String> {
    let package = alumina_sim::capability::package();
    let expected_identity = calculate_identity(&package).map_err(|error| format!("{error:?}"))?;
    let mut document = vec![
        0_u8;
        usize::try_from(expected_identity.byte_len).map_err(|_| {
            "simulator capability length exceeds host usize".to_owned()
        })?
    ];
    let mut offset = 0_u32;
    while offset < expected_identity.byte_len {
        let mut chunk = [0_u8; MAX_CAPABILITY_CHUNK_BYTES];
        let read = read_verified_range(&package, offset, &mut chunk)
            .map_err(|error| format!("{error:?}"))?;
        if read.byte_len == 0 {
            return Err("simulator capability range encoder made no progress".to_owned());
        }
        let start = usize::try_from(offset)
            .map_err(|_| "simulator capability offset exceeds host usize".to_owned())?;
        let count = usize::from(read.byte_len);
        document[start..start + count].copy_from_slice(&chunk[..count]);
        offset = offset
            .checked_add(u32::from(read.byte_len))
            .ok_or_else(|| "simulator capability offset overflowed".to_owned())?;
    }
    let snapshot = build_board_explorer_snapshot(
        &document,
        expected_identity,
        BoardCapabilityLimits::interactive(),
    )
    .map_err(|error| error.to_string())?;
    let fixture = tinybee_diagnostic_fixture().map_err(|error| format!("{error:?}"))?;
    let diagnostics = build_diagnostic_explorer_snapshot(
        &snapshot,
        fixture.overview_bytes(),
        fixture.digital_capture_bytes(),
        DiagnosticLimits::interactive(),
    )
    .map_err(|error| error.to_string())?;
    Ok(BoardExplorerPanel {
        snapshot,
        diagnostics,
        filter: BoardResourceFilter::All,
        search: String::new(),
        selected: Some(ResourceId::Gpio(33)),
        diagnostic_cursor_offset: 500,
    })
}

fn empty_target_workspace(
    registry: &GraphDeploymentRegistry,
) -> Result<GraphWorkspaceDocument, String> {
    let context = GraphDocument::try_new(
        0,
        registry.semantic_registry().context_schema().clone(),
        registry.semantic_registry().context_clocks().to_vec(),
        Vec::new(),
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    GraphWorkspaceDocument::try_new(
        GraphWorkspaceLimits::interactive(),
        0,
        1,
        1,
        context,
        Vec::new(),
    )
    .map_err(|error| error.to_string())
}

fn control_palette(
    fixture: &RepresentativeExactControlGraph,
) -> Result<Vec<NodePaletteEntry>, String> {
    let registry = fixture.registry();
    let semantic = registry.semantic_registry();
    let mut palette = Vec::with_capacity(semantic.schemas().len());
    for schema in semantic.schemas() {
        if registry.implementation(schema.kind()).is_none() {
            return Err(format!(
                "audited palette kind {} v{} has no fixed simulation implementation",
                schema.kind().name(),
                schema.kind().version()
            ));
        }
        let exemplar = fixture
            .document()
            .nodes()
            .iter()
            .find(|node| node.kind() == schema.kind())
            .ok_or_else(|| {
                format!(
                    "audited palette kind {} v{} has no reviewed default instance",
                    schema.kind().name(),
                    schema.kind().version()
                )
            })?;
        if exemplar.inputs() != schema.inputs()
            || exemplar.outputs() != schema.outputs()
            || !schema.allowed_domains().contains(exemplar.domain())
            || !parameters_match_schema(exemplar.parameters(), schema.parameters())
        {
            return Err(format!(
                "audited palette kind {} v{} disagrees with its reviewed default instance",
                schema.kind().name(),
                schema.kind().version()
            ));
        }
        let short = short_kind(schema.kind().name());
        palette.push(NodePaletteEntry {
            display_name: format!("{short} v{}", schema.kind().version()),
            prototype: GraphNodePrototype::new(
                schema.kind().clone(),
                format!("New {short}"),
                exemplar.domain(),
                schema.inputs().to_vec(),
                schema.outputs().to_vec(),
                exemplar.parameters().to_vec(),
            ),
        });
    }
    if palette.is_empty() {
        return Err("audited node palette is empty".to_owned());
    }
    Ok(palette)
}

fn parameters_match_schema(
    parameters: &[NodeParameter],
    contracts: &[alumina_interface_core::graph::NodeParameterContract],
) -> bool {
    parameters.len() == contracts.len()
        && parameters
            .iter()
            .zip(contracts)
            .all(|(parameter, contract)| {
                parameter.id() == contract.id()
                    && parameter.name() == contract.name()
                    && parameter.value().value_type() == contract.value_type()
            })
}

fn new_node_position(workspace: &GraphWorkspaceDocument) -> Result<(i32, i32), String> {
    let Some(maximum_x) = workspace
        .placements()
        .iter()
        .map(|placement| placement.x())
        .max()
    else {
        return Ok((NEW_NODE_ORIGIN, NEW_NODE_ORIGIN));
    };
    let x = maximum_x
        .checked_add(NEW_NODE_X_GAP)
        .ok_or_else(|| "node palette position overflowed the canvas lattice".to_owned())?;
    if x.unsigned_abs() > workspace.limits().maximum_coordinate_magnitude {
        return Err("node palette position exceeds the canonical canvas policy".to_owned());
    }
    let y = workspace
        .placements()
        .iter()
        .map(|placement| placement.y())
        .min()
        .unwrap_or(NEW_NODE_ORIGIN);
    Ok((x, y))
}

fn parameter_edit_text(document: &GraphDocument, value: &TypedGraphValue) -> Option<String> {
    format_graph_literal_text(
        document.schema(),
        value,
        GraphLiteralTextLimits::new(parameter_text_limit(document, value.value_type())),
    )
    .ok()
}

fn parameter_text_limit(document: &GraphDocument, value_type: GraphTypeId) -> usize {
    let rational_digits = document.schema().limits().maximum_rational_digits;
    let interactive = GraphLiteralTextLimits::interactive().maximum_bytes();
    document
        .schema()
        .value_type(value_type)
        .map_or(1, |definition| match definition.kind() {
            TypeKind::Boolean => 5,
            TypeKind::ExactRational { .. } => rational_digits.saturating_mul(2).saturating_add(4),
            TypeKind::MeasurementInterval { .. } => {
                rational_digits.saturating_mul(4).saturating_add(10)
            }
            TypeKind::CanonicalI64 { .. } | TypeKind::CanonicalU64 { .. } => 20,
            TypeKind::Text { maximum_bytes } => (*maximum_bytes as usize)
                .saturating_mul(6)
                .saturating_add(2),
            TypeKind::Bytes { maximum_bytes } => (*maximum_bytes as usize)
                .saturating_mul(2)
                .saturating_add(6),
            TypeKind::Array { .. }
            | TypeKind::Record { .. }
            | TypeKind::Option { .. }
            | TypeKind::Result { .. } => interactive,
            TypeKind::Event { .. }
            | TypeKind::Stream { .. }
            | TypeKind::ResourceHandle { .. }
            | TypeKind::JobHandle => 1,
        })
        .min(interactive)
}

fn parse_parameter_text(
    document: &GraphDocument,
    value_type: GraphTypeId,
    source: &str,
) -> Result<TypedGraphValue, String> {
    let maximum = parameter_text_limit(document, value_type);
    parse_graph_literal_text(
        document.schema(),
        value_type,
        source,
        GraphLiteralTextLimits::new(maximum),
    )
    .map_err(|error| error.to_string())
}

#[cfg(any(target_arch = "wasm32", test))]
fn encode_persisted_authoring_session(
    session: &CanonicalGraphAuthoringSessionEncoding,
) -> Result<String, String> {
    let bytes = session.bytes();
    if bytes.len() > MAX_GRAPH_AUTHORING_SESSION_BYTES {
        return Err(format!(
            "canonical ALGS has {} bytes; browser persistence admits at most {}",
            bytes.len(),
            MAX_GRAPH_AUTHORING_SESSION_BYTES
        ));
    }
    let encoded_bytes = bytes
        .len()
        .checked_mul(2)
        .and_then(|length| length.checked_add(PERSISTED_AUTHORING_SESSION_PREFIX.len()))
        .ok_or_else(|| "persisted ALGS text length overflowed".to_owned())?;
    let mut result = String::with_capacity(encoded_bytes);
    result.push_str(PERSISTED_AUTHORING_SESSION_PREFIX);
    append_lower_hex(&mut result, bytes);
    Ok(result)
}

#[cfg(any(target_arch = "wasm32", test))]
fn append_lower_hex(result: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

fn decode_persisted_authoring_session(value: &str) -> Result<Vec<u8>, String> {
    let encoded = value
        .strip_prefix(PERSISTED_AUTHORING_SESSION_PREFIX)
        .ok_or_else(|| "persisted ALGS prefix/version is unsupported".to_owned())?;
    decode_persisted_hex(encoded, MAX_GRAPH_AUTHORING_SESSION_BYTES, "ALGS")
}

fn decode_persisted_hex(
    encoded: &str,
    maximum_bytes: usize,
    artifact: &str,
) -> Result<Vec<u8>, String> {
    if !encoded.len().is_multiple_of(2) {
        return Err(format!("persisted {artifact} hex length is odd"));
    }
    let byte_length = encoded.len() / 2;
    if byte_length > maximum_bytes {
        return Err(format!(
            "persisted {artifact} exceeds the {maximum_bytes}-byte admission limit"
        ));
    }
    let mut bytes = Vec::with_capacity(byte_length);
    for pair in encoded.as_bytes().chunks_exact(2) {
        let high = canonical_hex_nibble(pair[0])
            .ok_or_else(|| format!("persisted {artifact} is not canonical lowercase hex"))?;
        let low = canonical_hex_nibble(pair[1])
            .ok_or_else(|| format!("persisted {artifact} is not canonical lowercase hex"))?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

const fn canonical_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn initial_workspace(
    fixture: &RepresentativeExactControlGraph,
) -> Result<
    (
        GraphWorkspaceDocument,
        CanonicalGraphWorkspaceEncoding,
        GraphPresentation,
    ),
    String,
> {
    let automatic = graph_presentation(fixture.document(), fixture.registry(), None)?;
    let placements = fixture
        .document()
        .nodes()
        .iter()
        .map(|node| {
            let layout = automatic
                .nodes
                .get(&node.id())
                .ok_or_else(|| format!("automatic layout omitted node {}", node.id().get()))?;
            Ok(GraphNodePlacement::new(
                node.id(),
                canonical_initial_coordinate(layout.rect.left())?,
                canonical_initial_coordinate(layout.rect.top())?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let next_node_id = fixture
        .document()
        .nodes()
        .iter()
        .map(|node| u64::from(node.id().get()))
        .max()
        .unwrap_or(0)
        + 1;
    let next_wire_id = fixture
        .document()
        .wires()
        .iter()
        .map(|wire| u64::from(wire.id().get()))
        .max()
        .unwrap_or(0)
        + 1;
    let workspace = GraphWorkspaceDocument::try_new(
        GraphWorkspaceLimits::interactive(),
        1,
        next_node_id,
        next_wire_id,
        fixture.document().clone(),
        placements,
    )
    .map_err(|error| error.to_string())?;
    let encoding = encode_graph_workspace(&workspace).map_err(|error| error.to_string())?;
    let presentation = graph_presentation(
        workspace.graph(),
        fixture.registry(),
        Some(workspace.placements()),
    )?;
    Ok((workspace, encoding, presentation))
}

fn representative_probes(workspace: &GraphWorkspaceDocument) -> Result<ProbePackage, String> {
    let probes = SIGNALS
        .into_iter()
        .enumerate()
        .map(|(index, signal)| {
            GraphProbeDefinition::new(
                GraphProbeId::new(
                    u32::try_from(index + 1).expect("reference probe count fits canonical u32"),
                ),
                probe_name(signal.endpoint()),
                signal.endpoint(),
                GraphProbeCapture::new(
                    u32::try_from(MAXIMUM_POINTS_PER_SERIES)
                        .expect("visible trace policy fits canonical u32"),
                    1,
                ),
            )
        })
        .collect();
    let trigger_probe = SIGNALS
        .iter()
        .position(|signal| *signal == RepresentativeControlSignal::MeasurementWithinRange)
        .and_then(|index| u32::try_from(index + 1).ok())
        .map(GraphProbeId::new)
        .ok_or_else(|| "reference interlock trigger probe is unavailable".to_owned())?;
    let document = GraphProbeDocument::try_new(
        GraphProbeLimits::interactive(),
        1,
        u64::try_from(SIGNALS.len() + 1).expect("reference probe count fits canonical u64"),
        Some(GraphProbeTrigger::new(
            trigger_probe,
            GraphProbeEdge::Falling,
            2,
            2,
        )),
        workspace,
        probes,
    )
    .map_err(|error| error.to_string())?;
    let encoding = encode_graph_probes(&document).map_err(|error| error.to_string())?;
    Ok(ProbePackage { document, encoding })
}

fn empty_probes(workspace: &GraphWorkspaceDocument) -> Result<ProbePackage, String> {
    let document = GraphProbeDocument::try_new(
        GraphProbeLimits::interactive(),
        0,
        1,
        None,
        workspace,
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    let encoding = encode_graph_probes(&document).map_err(|error| error.to_string())?;
    Ok(ProbePackage { document, encoding })
}

fn probe_name(source: WireEndpoint) -> String {
    SIGNALS
        .into_iter()
        .find(|signal| signal.endpoint() == source)
        .map_or_else(
            || format!("node-{}-port-{}", source.node.get(), source.port.get()),
            |signal| match signal {
                RepresentativeControlSignal::Error => "error".to_owned(),
                RepresentativeControlSignal::IntegralPrior => "integral-prior".to_owned(),
                RepresentativeControlSignal::ClampedController => "controller-clamped".to_owned(),
                RepresentativeControlSignal::ExternalPermit => "external-permit".to_owned(),
                RepresentativeControlSignal::MeasurementWithinRange => {
                    "measurement-in-range".to_owned()
                }
                RepresentativeControlSignal::CombinedPermit => "combined-permit".to_owned(),
                RepresentativeControlSignal::PermittedOutput => "output-permitted".to_owned(),
            },
        )
}

fn representative_component(
    workspace: &GraphWorkspaceDocument,
    fixture: &RepresentativeExactControlGraph,
) -> Result<ComponentPackage, String> {
    let (document, encoding) = representative_component_document(workspace)?;
    let hierarchy = representative_hierarchy(&document, encoding.digest(), fixture)?;
    Ok(ComponentPackage {
        document,
        encoding,
        hierarchy,
    })
}

fn representative_component_document(
    workspace: &GraphWorkspaceDocument,
) -> Result<(GraphComponentDocument, CanonicalGraphComponentEncoding), String> {
    let outputs = SIGNALS
        .iter()
        .copied()
        .enumerate()
        .map(|(index, signal)| {
            let id = u32::try_from(index + 1)
                .map_err(|_| "representative output identity overflowed".to_owned())?;
            let name = match signal {
                RepresentativeControlSignal::Error => "error",
                RepresentativeControlSignal::IntegralPrior => "integral_prior",
                RepresentativeControlSignal::ClampedController => "clamped_controller",
                RepresentativeControlSignal::ExternalPermit => "external_permit",
                RepresentativeControlSignal::MeasurementWithinRange => "measurement_within_range",
                RepresentativeControlSignal::CombinedPermit => "combined_permit",
                RepresentativeControlSignal::PermittedOutput => "permitted_output",
            };
            Ok(GraphComponentOutput::new(
                GraphComponentOutputId::new(id),
                name,
                signal.endpoint(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let parameter_specs = [
        (1, "proportional_gain", 8, 1, 20, 20),
        (2, "integral_gain", 11, 1, 20, 84),
        (3, "derivative_gain", 14, 1, 20, 148),
        (4, "output_minimum", 17, 1, 240, 20),
        (5, "output_maximum", 17, 2, 240, 84),
        (6, "safe_output", 18, 1, 240, 148),
        (11, "interlock_minimum", 20, 1, 240, 212),
        (12, "interlock_maximum", 20, 2, 240, 276),
    ];
    let mut panel_items = parameter_specs
        .into_iter()
        .map(|(id, name, node, parameter, x, y)| {
            GraphFrontPanelItem::new(
                GraphFrontPanelItemId::new(id),
                name,
                GraphFrontPanelBinding::ParameterControl {
                    node: GraphNodeId::new(node),
                    parameter,
                },
                GraphFrontPanelRect::new(x, y, 200, 54),
            )
        })
        .collect::<Vec<_>>();
    let output_specs = [
        (7, "error_indicator", 1, 20),
        (8, "integral_prior_indicator", 2, 84),
        (9, "clamped_controller_indicator", 3, 148),
        (10, "permitted_output_indicator", 4, 212),
        (13, "measurement_within_range_indicator", 5, 340),
        (14, "combined_permit_indicator", 6, 404),
        (15, "external_permit_indicator", 7, 276),
    ];
    panel_items.extend(output_specs.into_iter().map(|(id, name, output, y)| {
        GraphFrontPanelItem::new(
            GraphFrontPanelItemId::new(id),
            name,
            GraphFrontPanelBinding::OutputIndicator(GraphComponentOutputId::new(output)),
            GraphFrontPanelRect::new(460, y, 240, 54),
        )
    }));
    let document = GraphComponentDocument::try_new(
        GraphComponentLimits::interactive(),
        workspace.revision(),
        1,
        "control.reference_pid",
        1,
        8,
        16,
        workspace.clone(),
        Vec::new(),
        outputs,
        panel_items,
    )
    .map_err(|error| error.to_string())?;
    let encoding = encode_graph_component(&document).map_err(|error| error.to_string())?;
    Ok((document, encoding))
}

fn empty_library_component(
    name: &str,
    authority: &GraphWorkspaceDocument,
) -> Result<GraphComponentDocument, String> {
    let graph = GraphDocument::try_new(
        1,
        authority.graph().schema().clone(),
        authority.graph().clocks().to_vec(),
        Vec::new(),
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    let workspace = GraphWorkspaceDocument::try_new(authority.limits(), 1, 1, 1, graph, Vec::new())
        .map_err(|error| error.to_string())?;
    GraphComponentDocument::try_new(
        GraphComponentLimits::interactive(),
        1,
        1,
        name,
        1,
        1,
        1,
        workspace,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .map_err(|error| error.to_string())
}

fn representative_wrapper_component(
    component: &GraphComponentDocument,
) -> Result<(GraphComponentDocument, Digest), String> {
    let nested_id = GraphNodeId::new(1);
    let nested_prototype = graph_component_instance_prototype(component, "Reference PID leaf")
        .map_err(|error| error.to_string())?;
    let nested_instance = NodeDefinition::new(
        nested_id,
        nested_prototype.kind().clone(),
        "Reference PID leaf",
        nested_prototype.domain(),
        nested_prototype.inputs().to_vec(),
        nested_prototype.outputs().to_vec(),
        nested_prototype.parameters().to_vec(),
    );
    let wrapper_graph = GraphDocument::try_new(
        1,
        component.workspace().graph().schema().clone(),
        component.workspace().graph().clocks().to_vec(),
        vec![nested_instance],
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    let wrapper_workspace = GraphWorkspaceDocument::try_new(
        GraphWorkspaceLimits::interactive(),
        1,
        2,
        1,
        wrapper_graph,
        vec![GraphNodePlacement::new(nested_id, 28, 28)],
    )
    .map_err(|error| error.to_string())?;
    let wrapper_inputs = component
        .inputs()
        .iter()
        .map(|input| {
            let port =
                graph_component_instance_input_port(component, input.id()).ok_or_else(|| {
                    format!("component input {} has no instance port", input.id().get())
                })?;
            Ok(GraphComponentInput::new(
                input.id(),
                input.name(),
                WireEndpoint {
                    node: nested_id,
                    port,
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let wrapper_outputs = component
        .outputs()
        .iter()
        .map(|output| {
            let port =
                graph_component_instance_output_port(component, output.id()).ok_or_else(|| {
                    format!(
                        "component output {} has no instance port",
                        output.id().get()
                    )
                })?;
            Ok(GraphComponentOutput::new(
                output.id(),
                output.name(),
                WireEndpoint {
                    node: nested_id,
                    port,
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let wrapper = GraphComponentDocument::try_new(
        GraphComponentLimits::interactive(),
        component.revision(),
        1,
        "control.reference_pid_wrapper",
        component.next_input_id(),
        component.next_output_id(),
        1,
        wrapper_workspace,
        wrapper_inputs,
        wrapper_outputs,
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    let wrapper_digest = encode_graph_component(&wrapper)
        .map_err(|error| error.to_string())?
        .digest();
    Ok((wrapper, wrapper_digest))
}

fn representative_hierarchy(
    component: &GraphComponentDocument,
    component_digest: Digest,
    fixture: &RepresentativeExactControlGraph,
) -> Result<HierarchyPackage, String> {
    let nested_id = GraphNodeId::new(1);
    let (wrapper, wrapper_digest) = representative_wrapper_component(component)?;
    let prototype = graph_component_instance_prototype(&wrapper, "Reference PID wrapper")
        .map_err(|error| error.to_string())?;
    let instance_id = GraphNodeId::new(1);
    let instance = NodeDefinition::new(
        instance_id,
        prototype.kind().clone(),
        "Reference PID wrapper",
        prototype.domain(),
        prototype.inputs().to_vec(),
        prototype.outputs().to_vec(),
        prototype.parameters().to_vec(),
    );
    let root_graph = GraphDocument::try_new(
        1,
        component.workspace().graph().schema().clone(),
        component.workspace().graph().clocks().to_vec(),
        vec![instance],
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    let root = GraphWorkspaceDocument::try_new(
        GraphWorkspaceLimits::interactive(),
        1,
        2,
        1,
        root_graph,
        vec![GraphNodePlacement::new(instance_id, 28, 28)],
    )
    .map_err(|error| error.to_string())?;
    let document = GraphHierarchyDocument::try_new(
        GraphHierarchyLimits::interactive(),
        component.revision(),
        root,
        vec![component.clone(), wrapper],
        vec![
            GraphComponentInstance::root(instance_id, wrapper_digest),
            GraphComponentInstance::nested(wrapper_digest, nested_id, component_digest),
        ],
    )
    .map_err(|error| error.to_string())?;
    hierarchy_package(document, fixture)
}

fn hierarchy_package(
    document: GraphHierarchyDocument,
    fixture: &RepresentativeExactControlGraph,
) -> Result<HierarchyPackage, String> {
    analyze_component_library_drafts(&document, fixture)?;
    let encoding = encode_graph_hierarchy(&document).map_err(|error| error.to_string())?;
    let flattening = flatten_graph_hierarchy(&document).map_err(|error| error.to_string())?;
    let source_map = encode_graph_hierarchy_source_map(
        &flattening,
        GraphHierarchySourceMapLimits::interactive(),
    )
    .map_err(|error| error.to_string())?;
    analyze_graph_draft(
        flattening.workspace().graph(),
        fixture.registry().semantic_registry(),
    )
    .map_err(|error| format!("flattened component draft semantics rejected: {error}"))?;
    Ok(HierarchyPackage {
        document,
        encoding,
        flattening,
        source_map,
    })
}

fn analyze_component_library_drafts(
    document: &GraphHierarchyDocument,
    fixture: &RepresentativeExactControlGraph,
) -> Result<(), String> {
    for dependency in document.dependencies() {
        let mut workspace = dependency.document().workspace().clone();
        let placeholders = workspace
            .graph()
            .nodes()
            .iter()
            .filter(|node| {
                node.kind().name() == GRAPH_COMPONENT_INSTANCE_KIND
                    && node.kind().version() == GRAPH_COMPONENT_INSTANCE_VERSION
            })
            .map(NodeDefinition::id)
            .collect::<Vec<_>>();
        for placeholder in placeholders {
            workspace
                .delete_node(placeholder)
                .map_err(|error| error.to_string())?;
        }
        analyze_graph_draft(workspace.graph(), fixture.registry().semantic_registry()).map_err(
            |error| {
                format!(
                    "component library ALGC {}… draft semantics rejected: {error}",
                    digest_prefix(dependency.digest().0)
                )
            },
        )?;
    }
    Ok(())
}

fn structural_workspace_presentation(
    workspace: &GraphWorkspaceDocument,
) -> Result<GraphPresentation, String> {
    let document = workspace.graph();
    if document.nodes().is_empty() {
        return Ok(GraphPresentation {
            nodes: BTreeMap::new(),
            wires: BTreeMap::new(),
            size: egui::vec2(EMPTY_CANVAS_WIDTH, EMPTY_CANVAS_HEIGHT),
        });
    }
    if document.nodes().len() > MAXIMUM_VISIBLE_NODES {
        return Err("hierarchy root exceeds the visible-node limit".to_owned());
    }
    if document.wires().len() > MAXIMUM_VISIBLE_WIRES {
        return Err("hierarchy root exceeds the visible-wire limit".to_owned());
    }
    if workspace.placements().len() != document.nodes().len() {
        return Err("hierarchy root canvas does not cover every node".to_owned());
    }
    let minimum_x = workspace
        .placements()
        .iter()
        .map(|placement| display_coordinate(placement.x()))
        .fold(f32::INFINITY, f32::min);
    let minimum_y = workspace
        .placements()
        .iter()
        .map(|placement| display_coordinate(placement.y()))
        .fold(f32::INFINITY, f32::min);
    let offset = egui::vec2(
        (CANVAS_MARGIN - minimum_x).max(0.0),
        (CANVAS_MARGIN - minimum_y).max(0.0),
    );
    let mut nodes = BTreeMap::new();
    let mut maximum_bottom = 0.0_f32;
    let mut maximum_right = 0.0_f32;
    for placement in workspace.placements() {
        let node = document
            .node(placement.node())
            .ok_or_else(|| format!("hierarchy root node {} is missing", placement.node().get()))?;
        let rect = egui::Rect::from_min_size(
            egui::pos2(
                display_coordinate(placement.x()) + offset.x,
                display_coordinate(placement.y()) + offset.y,
            ),
            egui::vec2(NODE_WIDTH, node_height(node)),
        );
        maximum_bottom = maximum_bottom.max(rect.bottom());
        maximum_right = maximum_right.max(rect.right());
        nodes.insert(placement.node(), NodePresentation { rect, rank: 0 });
    }
    let wires = document
        .wires()
        .iter()
        .map(|wire| {
            (
                wire.id(),
                WirePresentation {
                    feedback_lane: None,
                },
            )
        })
        .collect();
    Ok(GraphPresentation {
        nodes,
        wires,
        size: egui::vec2(
            maximum_right + CANVAS_MARGIN,
            maximum_bottom + CANVAS_MARGIN + 36.0,
        ),
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "layout admission, state-edge classification, ranking, and bounded placement stay together for auditability"
)]
fn graph_presentation(
    document: &GraphDocument,
    registry: &GraphSimulationRegistry,
    placements: Option<&[GraphNodePlacement]>,
) -> Result<GraphPresentation, String> {
    if document.nodes().is_empty() {
        if placements.is_some_and(|placements| !placements.is_empty()) {
            return Err("empty exact control graph retained canvas placements".to_owned());
        }
        return Ok(GraphPresentation {
            nodes: BTreeMap::new(),
            wires: BTreeMap::new(),
            size: egui::vec2(EMPTY_CANVAS_WIDTH, EMPTY_CANVAS_HEIGHT),
        });
    }
    if document.nodes().len() > MAXIMUM_VISIBLE_NODES {
        return Err("exact control graph exceeds the visible-node limit".to_owned());
    }
    if document.wires().len() > MAXIMUM_VISIBLE_WIRES {
        return Err("exact control graph exceeds the visible-wire limit".to_owned());
    }

    let mut instantaneous_edges = BTreeSet::new();
    let mut feedback = BTreeSet::new();
    for wire in document.wires() {
        let target = document
            .node(wire.target().node)
            .ok_or_else(|| format!("wire {} has no target node", wire.id().get()))?;
        let schema = registry
            .semantic_registry()
            .schema(target.kind())
            .ok_or_else(|| format!("node {} has no audited schema", target.id().get()))?;
        let is_state_capture = schema
            .state()
            .is_some_and(|state| state.next_input() == wire.target().port);
        let has_current_tick_effect = schema.outputs().is_empty()
            || schema
                .output_dependencies()
                .iter()
                .any(|dependency| dependency.inputs().contains(&wire.target().port));
        if is_state_capture || !has_current_tick_effect {
            feedback.insert(wire.id());
        } else {
            instantaneous_edges.insert((wire.source().node, wire.target().node));
        }
    }

    let mut indegree: BTreeMap<GraphNodeId, usize> =
        document.nodes().iter().map(|node| (node.id(), 0)).collect();
    let mut successors: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
    for (source, target) in &instantaneous_edges {
        let Some(value) = indegree.get_mut(target) else {
            return Err(format!("layout target node {} is missing", target.get()));
        };
        *value = value
            .checked_add(1)
            .ok_or_else(|| "graph layout indegree overflow".to_owned())?;
        successors.entry(*source).or_default().push(*target);
    }

    let mut ranks: BTreeMap<GraphNodeId, usize> =
        document.nodes().iter().map(|node| (node.id(), 0)).collect();
    let mut ready: BTreeSet<GraphNodeId> = indegree
        .iter()
        .filter_map(|(node, degree)| (*degree == 0).then_some(*node))
        .collect();
    let mut visited = 0_usize;
    while let Some(node) = ready.pop_first() {
        visited += 1;
        let source_rank = *ranks
            .get(&node)
            .ok_or_else(|| "graph layout rank is missing".to_owned())?;
        for target in successors.get(&node).into_iter().flatten() {
            let candidate = source_rank
                .checked_add(1)
                .ok_or_else(|| "graph layout rank overflow".to_owned())?;
            let target_rank = ranks
                .get_mut(target)
                .ok_or_else(|| "graph layout target rank is missing".to_owned())?;
            *target_rank = (*target_rank).max(candidate);
            let degree = indegree
                .get_mut(target)
                .ok_or_else(|| "graph layout target indegree is missing".to_owned())?;
            *degree = degree
                .checked_sub(1)
                .ok_or_else(|| "graph layout indegree underflow".to_owned())?;
            if *degree == 0 {
                ready.insert(*target);
            }
        }
    }
    if visited != document.nodes().len() {
        return Err("instantaneous graph layout contains a cycle".to_owned());
    }

    let mut columns: BTreeMap<usize, Vec<GraphNodeId>> = BTreeMap::new();
    for node in document.nodes() {
        columns
            .entry(ranks[&node.id()])
            .or_default()
            .push(node.id());
    }
    let mut nodes = BTreeMap::new();
    let mut maximum_bottom = 0.0_f32;
    let mut maximum_right = 0.0_f32;
    if let Some(placements) = placements {
        if placements.len() != document.nodes().len() {
            return Err("saved canvas does not cover every graph node".to_owned());
        }
        let minimum_x = placements
            .iter()
            .map(|placement| display_coordinate(placement.x()))
            .fold(f32::INFINITY, f32::min);
        let minimum_y = placements
            .iter()
            .map(|placement| display_coordinate(placement.y()))
            .fold(f32::INFINITY, f32::min);
        let offset = egui::vec2(
            (CANVAS_MARGIN - minimum_x).max(0.0),
            (CANVAS_MARGIN - minimum_y).max(0.0),
        );
        for placement in placements {
            let node = document
                .node(placement.node())
                .ok_or_else(|| format!("layout node {} is missing", placement.node().get()))?;
            let height = node_height(node);
            let rect = egui::Rect::from_min_size(
                egui::pos2(
                    display_coordinate(placement.x()) + offset.x,
                    display_coordinate(placement.y()) + offset.y,
                ),
                egui::vec2(NODE_WIDTH, height),
            );
            maximum_bottom = maximum_bottom.max(rect.bottom());
            maximum_right = maximum_right.max(rect.right());
            nodes.insert(
                placement.node(),
                NodePresentation {
                    rect,
                    rank: ranks[&placement.node()],
                },
            );
        }
    } else {
        for (rank, column) in &columns {
            let x = CANVAS_MARGIN + display_index(*rank) * (NODE_WIDTH + COLUMN_GAP);
            let mut y = CANVAS_MARGIN;
            for id in column {
                let node = document
                    .node(*id)
                    .ok_or_else(|| format!("layout node {} is missing", id.get()))?;
                let height = node_height(node);
                let rect =
                    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(NODE_WIDTH, height));
                maximum_bottom = maximum_bottom.max(rect.bottom());
                maximum_right = maximum_right.max(rect.right());
                nodes.insert(*id, NodePresentation { rect, rank: *rank });
                y += height + NODE_GAP;
            }
        }
    }
    let feedback_height = 36.0 + display_index(feedback.len()) * 13.0;
    let size = egui::vec2(
        maximum_right + CANVAS_MARGIN,
        maximum_bottom + CANVAS_MARGIN + feedback_height,
    );
    let wires = document
        .wires()
        .iter()
        .map(|wire| {
            let feedback_lane = feedback.iter().position(|id| *id == wire.id());
            (wire.id(), WirePresentation { feedback_lane })
        })
        .collect();
    Ok(GraphPresentation { nodes, wires, size })
}

fn exact_probe_projection(
    fixture: &RepresentativeExactControlGraph,
    workspace: &GraphWorkspaceDocument,
    probes: &GraphProbeDocument,
) -> Result<GraphProbeProjection, String> {
    project_graph_probe_replay(
        probes,
        workspace,
        fixture.simulation(),
        fixture.registry(),
        GraphProbeProjectionLimits::interactive(),
    )
    .map_err(|error| error.to_string())
}

fn trace_series(
    fixture: &RepresentativeExactControlGraph,
    workspace: &GraphWorkspaceDocument,
    probes: &GraphProbeDocument,
) -> Result<Vec<TraceSeries>, String> {
    let mut complete = probes.clone();
    complete
        .clear_trigger(workspace)
        .map_err(|error| error.to_string())?;
    let projection = exact_probe_projection(fixture, workspace, &complete)?;
    let result = projected_trace_series(&projection, &complete, workspace)?;
    for signal in SIGNALS {
        if !result
            .iter()
            .any(|series| series.signal.representative == Some(signal))
        {
            return Err(format!("{} trace is empty", signal.label()));
        }
    }
    Ok(result)
}

fn projected_trace_series(
    projection: &GraphProbeProjection,
    probes: &GraphProbeDocument,
    workspace: &GraphWorkspaceDocument,
) -> Result<Vec<TraceSeries>, String> {
    let mut result = Vec::with_capacity(projection.series().len());
    let mut state_identity_bytes = 0_usize;
    for projected in projection.series() {
        let Some(first) = projected.samples().first() else {
            continue;
        };
        let definition = probes.probe(projected.probe()).ok_or_else(|| {
            format!(
                "projected probe p{} has no canonical definition",
                projected.probe().get()
            )
        })?;
        if definition.source() != projected.source() {
            return Err(format!(
                "projected probe p{} changed its exact output endpoint",
                projected.probe().get()
            ));
        }
        let signal = trace_signal(definition, workspace, first.entry().value().value_type())?;
        let mut points = Vec::with_capacity(projected.samples().len());
        let mut kind = None;
        for sample in projected.samples() {
            if points.len() >= MAXIMUM_POINTS_PER_SERIES {
                return Err(format!("{} trace exceeds display policy", signal.label()));
            }
            let (point_kind, point) = trace_point(
                &signal,
                workspace.graph().schema(),
                sample.entry(),
                sample.root_tick(),
            )?;
            if kind.is_some_and(|retained| retained != point_kind) {
                return Err(format!("{} trace changes value kind", signal.label()));
            }
            retain_state_identity_bytes(&mut state_identity_bytes, &point.value)?;
            kind = Some(point_kind);
            points.push(point);
        }
        if let Some(kind) = kind {
            result.push(TraceSeries {
                signal,
                kind,
                points,
            });
        }
    }
    Ok(result)
}

fn trace_signal(
    definition: &GraphProbeDefinition,
    workspace: &GraphWorkspaceDocument,
    sample_type: GraphTypeId,
) -> Result<TraceSignal, String> {
    let schema = workspace.graph().schema();
    let type_definition = schema.value_type(sample_type).ok_or_else(|| {
        format!(
            "probe p{} sample type t{} is unavailable",
            definition.id().get(),
            sample_type.get()
        )
    })?;
    let unit_symbol = match type_definition.kind() {
        TypeKind::ExactRational { unit }
        | TypeKind::MeasurementInterval { unit }
        | TypeKind::CanonicalI64 { unit, .. }
        | TypeKind::CanonicalU64 { unit, .. } => Some(
            schema
                .unit(*unit)
                .ok_or_else(|| {
                    format!(
                        "probe p{} physical scalar unit is unavailable",
                        definition.id().get()
                    )
                })?
                .symbol()
                .to_owned(),
        ),
        _ => None,
    };
    Ok(TraceSignal {
        probe: definition.id(),
        name: definition.name().to_owned(),
        source: definition.source(),
        representative: SIGNALS
            .iter()
            .copied()
            .find(|signal| signal.endpoint() == definition.source()),
        sample_type,
        sample_type_name: type_definition.name().to_owned(),
        unit_symbol,
    })
}

fn trace_point(
    signal: &TraceSignal,
    schema: &GraphSchema,
    entry: &GraphTraceEntry,
    root_tick: &Rational,
) -> Result<(TraceSeriesKind, TracePoint), String> {
    if entry.endpoint() != signal.source || entry.value().value_type() != signal.sample_type {
        return Err(format!(
            "{} trace changed its exact endpoint or sample type",
            signal.label()
        ));
    }
    let (kind, value) = trace_sample_value(signal, schema, entry.value())?;
    Ok((
        kind,
        TracePoint {
            clock: entry.clock(),
            tick: entry.clock_tick(),
            sequence: entry.sequence(),
            root_tick: root_tick.clone(),
            value,
        },
    ))
}

fn trace_sample_value(
    signal: &TraceSignal,
    schema: &GraphSchema,
    typed_value: &TypedGraphValue,
) -> Result<(TraceSeriesKind, TracePointValue), String> {
    if typed_value.value_type() != signal.sample_type {
        return Err(format!(
            "{} trace changed its exact sample type from t{} to t{}",
            signal.label(),
            signal.sample_type.get(),
            typed_value.value_type().get()
        ));
    }
    let sample_kind = schema
        .value_type(signal.sample_type)
        .ok_or_else(|| {
            format!(
                "{} trace sample type t{} is unavailable",
                signal.label(),
                signal.sample_type.get()
            )
        })?
        .kind();
    let value = typed_value.value();
    match (sample_kind, value) {
        (TypeKind::Boolean, GraphValue::Boolean(value)) => {
            Ok((TraceSeriesKind::Boolean, TracePointValue::Boolean(*value)))
        }
        (TypeKind::ExactRational { .. }, GraphValue::ExactRational(value)) => {
            analog_trace_point_value(signal, value.to_string(), value, value)
        }
        (
            TypeKind::MeasurementInterval { .. },
            GraphValue::MeasurementInterval { lower, upper },
        ) => analog_trace_point_value(signal, format!("{lower}..{upper}"), lower, upper),
        (TypeKind::CanonicalI64 { quantum, .. }, GraphValue::CanonicalI64(count)) => {
            let exact = Rational::from(*count) * quantum;
            analog_trace_point_value(
                signal,
                format!("{exact} [{count} lattice counts]"),
                &exact,
                &exact,
            )
        }
        (TypeKind::CanonicalU64 { quantum, .. }, GraphValue::CanonicalU64(count)) => {
            let exact = Rational::from(*count) * quantum;
            analog_trace_point_value(
                signal,
                format!("{exact} [{count} lattice counts]"),
                &exact,
                &exact,
            )
        }
        (TypeKind::Text { .. }, GraphValue::Text(_))
        | (TypeKind::Bytes { .. }, GraphValue::Bytes(_))
        | (TypeKind::Array { .. }, GraphValue::Array(_))
        | (TypeKind::Record { .. }, GraphValue::Record(_))
        | (TypeKind::Option { .. }, GraphValue::OptionNone | GraphValue::OptionSome(_))
        | (TypeKind::Result { .. }, GraphValue::ResultOk(_) | GraphValue::ResultError(_))
        | (TypeKind::ResourceHandle { .. }, GraphValue::ResourceHandle(_))
        | (TypeKind::JobHandle, GraphValue::JobHandle(_)) => {
            state_trace_point_value(signal, schema, typed_value)
        }
        _ => Err(format!(
            "{} trace type {} [t{}] does not admit a {:?} value",
            signal.label(),
            signal.sample_type_name,
            signal.sample_type.get(),
            value.kind()
        )),
    }
}

fn state_trace_point_value(
    signal: &TraceSignal,
    schema: &GraphSchema,
    value: &TypedGraphValue,
) -> Result<(TraceSeriesKind, TracePointValue), String> {
    let encoding = encode_typed_graph_value(schema, value).map_err(|error| {
        format!(
            "{} state trace failed canonical typed-value encoding: {error}",
            signal.label()
        )
    })?;
    Ok((
        TraceSeriesKind::State,
        TracePointValue::State {
            summary: state_value_summary(value.value()),
            encoding,
        },
    ))
}

fn retain_state_identity_bytes(total: &mut usize, value: &TracePointValue) -> Result<(), String> {
    let TracePointValue::State { encoding, .. } = value else {
        return Ok(());
    };
    *total = total
        .checked_add(encoding.bytes().len())
        .ok_or_else(|| "state trace identity byte count overflowed".to_owned())?;
    if *total > MAXIMUM_STATE_IDENTITY_BYTES {
        return Err(format!(
            "state traces exceed the bounded {MAXIMUM_STATE_IDENTITY_BYTES}-byte canonical identity display policy"
        ));
    }
    Ok(())
}

fn state_value_summary(value: &GraphValue) -> String {
    match value {
        GraphValue::Text(value) => {
            let character_count = value.chars().count();
            let mut preview = String::new();
            for character in value.chars().take(MAXIMUM_STATE_TEXT_PREVIEW_CHARS) {
                preview.extend(character.escape_default());
            }
            if character_count > MAXIMUM_STATE_TEXT_PREVIEW_CHARS {
                preview.push('…');
            }
            format!("text \"{preview}\" [{} UTF-8 bytes]", value.len())
        }
        GraphValue::Bytes(value) => {
            format!("bytes {} [{} bytes]", state_byte_prefix(value), value.len())
        }
        GraphValue::Array(values) => format!("array [{} items]", values.len()),
        GraphValue::Record(fields) => format!("record [{} fields]", fields.len()),
        GraphValue::OptionNone => "none".to_owned(),
        GraphValue::OptionSome(value) => format!("some({})", state_value_shape(value)),
        GraphValue::ResultOk(value) => format!("ok({})", state_value_shape(value)),
        GraphValue::ResultError(value) => format!("error({})", state_value_shape(value)),
        GraphValue::ResourceHandle(handle) => format!(
            "resource device {}… · board {}… · class {} · selector {}",
            digest_prefix16(handle.device_id.0),
            digest_prefix(handle.board_package_digest.0),
            handle.class.get(),
            handle.resource_selector
        ),
        GraphValue::JobHandle(handle) => format!(
            "job device {}… · global {}… · partition {}…",
            digest_prefix16(handle.device_id.0),
            digest_prefix(handle.global_job_digest.0),
            digest_prefix(handle.partition_digest.0)
        ),
        GraphValue::Boolean(_)
        | GraphValue::ExactRational(_)
        | GraphValue::MeasurementInterval { .. }
        | GraphValue::CanonicalI64(_)
        | GraphValue::CanonicalU64(_) => state_value_shape(value).to_owned(),
    }
}

fn state_value_shape(value: &GraphValue) -> &'static str {
    match value {
        GraphValue::Boolean(_) => "Boolean",
        GraphValue::ExactRational(_) => "exact rational",
        GraphValue::MeasurementInterval { .. } => "measurement interval",
        GraphValue::CanonicalI64(_) => "signed lattice count",
        GraphValue::CanonicalU64(_) => "unsigned lattice count",
        GraphValue::Text(_) => "text",
        GraphValue::Bytes(_) => "bytes",
        GraphValue::Array(_) => "array",
        GraphValue::Record(_) => "record",
        GraphValue::OptionNone | GraphValue::OptionSome(_) => "option",
        GraphValue::ResultOk(_) | GraphValue::ResultError(_) => "result",
        GraphValue::ResourceHandle(_) => "resource handle",
        GraphValue::JobHandle(_) => "job handle",
    }
}

fn state_byte_prefix(value: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut result = String::with_capacity(17);
    for byte in value.iter().take(8) {
        write!(&mut result, "{byte:02x}").expect("writing to String is infallible");
    }
    if value.len() > 8 {
        result.push('…');
    }
    result
}

fn analog_trace_point_value(
    signal: &TraceSignal,
    exact: String,
    lower: &Rational,
    upper: &Rational,
) -> Result<(TraceSeriesKind, TracePointValue), String> {
    if lower > upper {
        return Err(format!(
            "{} trace has a reversed exact analog interval",
            signal.label()
        ));
    }
    let lower_enclosure = lower
        .to_f64_enclosure()
        .filter(|bounds| bounds.iter().all(|bound| bound.is_finite()))
        .ok_or_else(|| format!("{} has no finite lower display enclosure", signal.label()))?;
    let upper_enclosure = upper
        .to_f64_enclosure()
        .filter(|bounds| bounds.iter().all(|bound| bound.is_finite()))
        .ok_or_else(|| format!("{} has no finite upper display enclosure", signal.label()))?;
    let enclosure = [lower_enclosure[0], upper_enclosure[1]];
    if enclosure[0] > enclosure[1] {
        return Err(format!(
            "{} trace has an inverted display enclosure",
            signal.label()
        ));
    }
    Ok((
        TraceSeriesKind::Analog,
        TracePointValue::Analog { exact, enclosure },
    ))
}

fn node_height(node: &NodeDefinition) -> f32 {
    let port_rows = node.inputs().len().max(node.outputs().len()).max(1);
    NODE_HEADER_HEIGHT + display_index(port_rows) * PORT_ROW_HEIGHT + 30.0
}

fn port_anchor(
    document: &GraphDocument,
    presentation: &GraphPresentation,
    endpoint: WireEndpoint,
    output: bool,
) -> Option<egui::Pos2> {
    let node = document.node(endpoint.node)?;
    let layout = presentation.nodes.get(&endpoint.node)?;
    let ports = if output {
        node.outputs()
    } else {
        node.inputs()
    };
    let index = ports.iter().position(|port| port.id() == endpoint.port)?;
    Some(port_anchor_for_rect(layout.rect, index, output))
}

fn port_anchor_for_rect(rect: egui::Rect, index: usize, output: bool) -> egui::Pos2 {
    let x = if output { rect.right() } else { rect.left() };
    egui::pos2(
        x,
        rect.top() + NODE_HEADER_HEIGHT + PORT_ROW_HEIGHT * (display_index(index) + 0.5),
    )
}

fn wire_color(document: &GraphDocument, source: WireEndpoint) -> egui::Color32 {
    let boolean = document
        .node(source.node)
        .and_then(|node| node.outputs().iter().find(|port| port.id() == source.port))
        .and_then(|port| document.schema().value_type(port.value_type()))
        .and_then(|value_type| match value_type.kind() {
            TypeKind::Stream { sample, .. } => document.schema().value_type(*sample),
            _ => None,
        })
        .is_some_and(|sample| matches!(sample.kind(), TypeKind::Boolean));
    if boolean {
        egui::Color32::from_rgb(241, 178, 84)
    } else {
        egui::Color32::from_rgb(96, 169, 232)
    }
}

fn paint_hierarchy_root_wire(
    painter: &egui::Painter,
    origin: egui::Vec2,
    document: &GraphDocument,
    presentation: &GraphPresentation,
    focus: HierarchyRootCanvasFocus,
    wire: &WireDefinition,
) {
    let source = wire.source();
    let target = wire.target();
    let (Some(source_anchor), Some(target_anchor)) = (
        port_anchor(document, presentation, source, true),
        port_anchor(document, presentation, target, false),
    ) else {
        return;
    };
    let source_anchor = source_anchor + origin;
    let target_anchor = target_anchor + origin;
    let selected = focus.selects_wire(wire);
    let color = if selected {
        egui::Color32::WHITE
    } else {
        wire_color(document, source)
    };
    let stroke = egui::Stroke::new(if selected { 2.4_f32 } else { 1.5_f32 }, color);
    let middle_x = (source_anchor.x + target_anchor.x) * 0.5;
    painter.add(egui::Shape::line(
        vec![
            source_anchor,
            egui::pos2(middle_x, source_anchor.y),
            egui::pos2(middle_x, target_anchor.y),
            target_anchor,
        ],
        stroke,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![
            target_anchor,
            target_anchor + egui::vec2(-7.0, -4.0),
            target_anchor + egui::vec2(-7.0, 4.0),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

fn paint_hierarchy_root_node(
    painter: &egui::Painter,
    rect: egui::Rect,
    node: &NodeDefinition,
    selected: bool,
    bound_instance: bool,
    placement: Option<GraphNodePlacement>,
) {
    let fill = if selected {
        egui::Color32::from_rgb(53, 76, 108)
    } else if bound_instance {
        egui::Color32::from_rgb(44, 50, 73)
    } else {
        egui::Color32::from_rgb(34, 43, 56)
    };
    let border = if selected {
        egui::Color32::from_rgb(126, 195, 255)
    } else if bound_instance {
        egui::Color32::from_rgb(184, 153, 244)
    } else {
        egui::Color32::from_rgb(89, 111, 139)
    };
    painter.rect_filled(rect, 6.0, fill);
    painter.rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, border),
    );
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.top() + NODE_HEADER_HEIGHT),
            egui::pos2(rect.right(), rect.top() + NODE_HEADER_HEIGHT),
        ],
        egui::Stroke::new(1.0_f32, border.gamma_multiply(0.65)),
    );
    painter.text(
        rect.left_top() + egui::vec2(10.0, 9.0),
        egui::Align2::LEFT_TOP,
        node.label(),
        egui::FontId::proportional(13.0),
        egui::Color32::WHITE,
    );
    painter.text(
        rect.left_top() + egui::vec2(10.0, 28.0),
        egui::Align2::LEFT_TOP,
        if bound_instance {
            "root ALGC instance"
        } else {
            "root structural node"
        },
        egui::FontId::monospace(10.5),
        egui::Color32::GRAY,
    );
    for (index, port) in node.inputs().iter().enumerate() {
        let anchor = port_anchor_for_rect(rect, index, false);
        painter.circle_filled(anchor, 4.0, egui::Color32::from_rgb(136, 171, 211));
        painter.text(
            anchor + egui::vec2(9.0, 0.0),
            egui::Align2::LEFT_CENTER,
            port.name(),
            egui::FontId::proportional(11.0),
            egui::Color32::LIGHT_GRAY,
        );
    }
    for (index, port) in node.outputs().iter().enumerate() {
        let anchor = port_anchor_for_rect(rect, index, true);
        painter.circle_filled(anchor, 4.0, egui::Color32::from_rgb(122, 211, 185));
        painter.text(
            anchor + egui::vec2(-9.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            port.name(),
            egui::FontId::proportional(11.0),
            egui::Color32::LIGHT_GRAY,
        );
    }
    let footer = placement.map_or_else(
        || "placement unavailable".to_owned(),
        |placement| format!("exact ({}, {})", placement.x(), placement.y()),
    );
    painter.text(
        rect.left_bottom() + egui::vec2(10.0, -8.0),
        egui::Align2::LEFT_BOTTOM,
        footer,
        egui::FontId::monospace(9.5),
        egui::Color32::from_rgb(170, 180, 194),
    );
}

fn typed_value_text(
    document: &GraphDocument,
    value: &alumina_interface_core::graph::TypedGraphValue,
) -> String {
    match value.value() {
        GraphValue::ExactRational(exact) => {
            let symbol = document
                .schema()
                .value_type(value.value_type())
                .and_then(|value_type| match value_type.kind() {
                    TypeKind::ExactRational { unit } => document.schema().unit(*unit),
                    _ => None,
                })
                .map_or("", alumina_interface_core::graph::UnitDefinition::symbol);
            format!("{exact} {symbol}")
        }
        GraphValue::Boolean(value) => value.to_string(),
        other => format!("{other:?}"),
    }
}

fn port_description(
    document: &GraphDocument,
    direction: &str,
    port: &alumina_interface_core::graph::PortDefinition,
) -> String {
    let type_name = document.schema().value_type(port.value_type()).map_or(
        "unknown",
        alumina_interface_core::graph::TypeDefinition::name,
    );
    format!(
        "{direction}.{}: {type_name} [t{}]",
        port.name(),
        port.value_type().get()
    )
}

fn paint_grid(painter: &egui::Painter, rect: egui::Rect) {
    let stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(13));
    let spacing = 24.0;
    let mut x = rect.left();
    while x <= rect.right() {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            stroke,
        );
        x += spacing;
    }
    let mut y = rect.top();
    while y <= rect.bottom() {
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            stroke,
        );
        y += spacing;
    }
}

fn component_input_endpoint_choices(
    component: &GraphComponentDocument,
    editing: Option<GraphComponentInputId>,
) -> Vec<(WireEndpoint, String)> {
    let connected = component
        .workspace()
        .graph()
        .wires()
        .iter()
        .map(|wire| wire.target())
        .collect::<BTreeSet<_>>();
    let exposed = component
        .inputs()
        .iter()
        .filter(|input| Some(input.id()) != editing)
        .map(GraphComponentInput::target)
        .collect::<BTreeSet<_>>();
    component
        .workspace()
        .graph()
        .nodes()
        .iter()
        .flat_map(|node| {
            node.inputs().iter().filter_map(|port| {
                let endpoint = WireEndpoint {
                    node: node.id(),
                    port: port.id(),
                };
                (!connected.contains(&endpoint) && !exposed.contains(&endpoint)).then(|| {
                    (
                        endpoint,
                        component_endpoint_label(component, endpoint, false),
                    )
                })
            })
        })
        .collect()
}

fn component_output_endpoint_choices(
    component: &GraphComponentDocument,
    editing: Option<GraphComponentOutputId>,
) -> Vec<(WireEndpoint, String)> {
    let exposed = component
        .outputs()
        .iter()
        .filter(|output| Some(output.id()) != editing)
        .map(GraphComponentOutput::source)
        .collect::<BTreeSet<_>>();
    component
        .workspace()
        .graph()
        .nodes()
        .iter()
        .flat_map(|node| {
            node.outputs().iter().filter_map(|port| {
                let endpoint = WireEndpoint {
                    node: node.id(),
                    port: port.id(),
                };
                (!exposed.contains(&endpoint)).then(|| {
                    (
                        endpoint,
                        component_endpoint_label(component, endpoint, true),
                    )
                })
            })
        })
        .collect()
}

fn component_endpoint_label(
    component: &GraphComponentDocument,
    endpoint: WireEndpoint,
    output: bool,
) -> String {
    let Some(node) = component.workspace().graph().node(endpoint.node) else {
        return format!("missing #{}.{}", endpoint.node.get(), endpoint.port.get());
    };
    let port = if output {
        node.outputs()
    } else {
        node.inputs()
    }
    .iter()
    .find(|port| port.id() == endpoint.port);
    let Some(port) = port else {
        return format!("missing #{}.{}", endpoint.node.get(), endpoint.port.get());
    };
    let value_type = component
        .workspace()
        .graph()
        .schema()
        .value_type(port.value_type())
        .map_or("unknown", TypeDefinition::name);
    format!(
        "#{}.{} · {} / {} · {} [t{}]",
        endpoint.node.get(),
        endpoint.port.get(),
        node.label(),
        port.name(),
        value_type,
        port.value_type().get(),
    )
}

fn endpoint_choice_selected_label(
    selected: Option<WireEndpoint>,
    choices: &[(WireEndpoint, String)],
    unavailable: &str,
) -> String {
    selected
        .and_then(|selected| {
            choices
                .iter()
                .find(|(endpoint, _)| *endpoint == selected)
                .map(|(_, label)| label.clone())
        })
        .unwrap_or_else(|| unavailable.to_owned())
}

fn component_connector_label(
    component: &GraphComponentDocument,
    connector: ComponentConnectorSelection,
) -> String {
    match connector {
        ComponentConnectorSelection::Input(id) => component.input(id).map_or_else(
            || format!("missing input #{}", id.get()),
            |input| {
                let port = graph_component_instance_input_port(component, id)
                    .map_or_else(|| "?".to_owned(), |port| port.get().to_string());
                format!(
                    "input #{} · instance p{} · {} → #{}.{}",
                    id.get(),
                    port,
                    input.name(),
                    input.target().node.get(),
                    input.target().port.get(),
                )
            },
        ),
        ComponentConnectorSelection::Output(id) => component.output(id).map_or_else(
            || format!("missing output #{}", id.get()),
            |output| {
                let port = graph_component_instance_output_port(component, id)
                    .map_or_else(|| "?".to_owned(), |port| port.get().to_string());
                format!(
                    "output #{} · instance p{} · {} ← #{}.{}",
                    id.get(),
                    port,
                    output.name(),
                    output.source().node.get(),
                    output.source().port.get(),
                )
            },
        ),
    }
}

fn panel_item_label(name: &str) -> String {
    name.replace('_', " ")
}

fn panel_binding_label(
    component: &GraphComponentDocument,
    binding: GraphFrontPanelBinding,
) -> String {
    match binding {
        GraphFrontPanelBinding::InputControl(input) => component.input(input).map_or_else(
            || format!("missing input {}", input.get()),
            |input| format!("input {} · {}", input.id().get(), input.name()),
        ),
        GraphFrontPanelBinding::ParameterControl { node, parameter } => component
            .workspace()
            .graph()
            .node(node)
            .and_then(|definition| {
                definition
                    .parameters()
                    .iter()
                    .find(|candidate| candidate.id() == parameter)
                    .map(|parameter_definition| {
                        format!(
                            "parameter #{}.{} · {}.{}",
                            node.get(),
                            parameter,
                            definition.label(),
                            parameter_definition.name()
                        )
                    })
            })
            .unwrap_or_else(|| format!("missing parameter #{}.{}", node.get(), parameter)),
        GraphFrontPanelBinding::OutputIndicator(output) => component.output(output).map_or_else(
            || format!("missing output {}", output.get()),
            |output| format!("output {} · {}", output.id().get(), output.name()),
        ),
    }
}

fn panel_binding_choices(
    component: &GraphComponentDocument,
) -> Vec<(GraphFrontPanelBinding, String)> {
    let mut choices = Vec::new();
    choices.extend(component.inputs().iter().map(|input| {
        let binding = GraphFrontPanelBinding::InputControl(input.id());
        (binding, panel_binding_label(component, binding))
    }));
    for node in component.workspace().graph().nodes() {
        choices.extend(node.parameters().iter().map(|parameter| {
            let binding = GraphFrontPanelBinding::ParameterControl {
                node: node.id(),
                parameter: parameter.id(),
            };
            (binding, panel_binding_label(component, binding))
        }));
    }
    choices.extend(component.outputs().iter().map(|output| {
        let binding = GraphFrontPanelBinding::OutputIndicator(output.id());
        (binding, panel_binding_label(component, binding))
    }));
    choices
}

fn next_panel_item_rect(component: &GraphComponentDocument) -> Result<GraphFrontPanelRect, String> {
    const X: i32 = 20;
    const GAP: u32 = 20;
    const WIDTH: u32 = 220;
    const HEIGHT: u32 = 54;
    let bottom = component
        .panel_items()
        .iter()
        .map(|item| {
            u32::try_from(item.rect().y())
                .ok()
                .and_then(|y| y.checked_add(item.rect().height()))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "validated panel rectangle could not be projected".to_owned())?
        .into_iter()
        .max()
        .unwrap_or(0);
    let y = bottom
        .checked_add(GAP)
        .ok_or_else(|| "front-panel placement overflowed".to_owned())?;
    let right = u32::try_from(X)
        .ok()
        .and_then(|x| x.checked_add(WIDTH))
        .ok_or_else(|| "front-panel placement overflowed".to_owned())?;
    let lower = y
        .checked_add(HEIGHT)
        .ok_or_else(|| "front-panel placement overflowed".to_owned())?;
    if right > component.limits().maximum_panel_coordinate
        || lower > component.limits().maximum_panel_coordinate
    {
        return Err("front-panel has no bounded space for another default item".to_owned());
    }
    Ok(GraphFrontPanelRect::new(
        X,
        i32::try_from(y).map_err(|_| "front-panel y coordinate exceeds i32".to_owned())?,
        WIDTH,
        HEIGHT,
    ))
}

fn paint_analog_trace_series(
    painter: &egui::Painter,
    rect: egui::Rect,
    series: &TraceSeries,
    time_axis: &TraceTimeAxis,
    minimum_value: f64,
    maximum_value: f64,
) {
    debug_assert_eq!(series.kind, TraceSeriesKind::Analog);
    let color = trace_signal_color(&series.signal);
    let mut line: Vec<egui::Pos2> = Vec::new();
    for point in &series.points {
        let TracePointValue::Analog { enclosure, .. } = &point.value else {
            continue;
        };
        let Some(x) = time_axis.plot_x(rect, &point.root_tick) else {
            continue;
        };
        let lower = plot_y(rect, enclosure[0], minimum_value, maximum_value);
        let upper = plot_y(rect, enclosure[1], minimum_value, maximum_value);
        let middle = (lower + upper) * 0.5;
        if let Some(previous) = line.last().copied() {
            line.push(egui::pos2(x, previous.y));
        }
        line.push(egui::pos2(x, middle));
        painter.line_segment(
            [egui::pos2(x, lower), egui::pos2(x, upper)],
            egui::Stroke::new(3.0_f32, color.gamma_multiply(0.55)),
        );
        painter.circle_filled(egui::pos2(x, middle), 2.8, color);
    }
    painter.add(egui::Shape::line(line, egui::Stroke::new(1.8_f32, color)));
}

fn paint_digital_trace_series(
    painter: &egui::Painter,
    rect: egui::Rect,
    series: &[&TraceSeries],
    time_axis: &TraceTimeAxis,
) {
    let lane_height = rect.height() / display_index(series.len()).max(1.0);
    let separator = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(35));
    for (index, series) in series.iter().copied().enumerate() {
        debug_assert_eq!(series.kind, TraceSeriesKind::Boolean);
        let lane_top = rect.top() + display_index(index) * lane_height;
        let lane_bottom = lane_top + lane_height;
        let high = lane_top + 8.0;
        let low = lane_bottom - 8.0;
        if index > 0 {
            painter.line_segment(
                [
                    egui::pos2(rect.left(), lane_top),
                    egui::pos2(rect.right(), lane_top),
                ],
                separator,
            );
        }
        let color = trace_signal_color(&series.signal);
        painter.text(
            egui::pos2(rect.left() - 6.0, (lane_top + lane_bottom) * 0.5),
            egui::Align2::RIGHT_CENTER,
            series.signal.label(),
            egui::FontId::monospace(9.0),
            color,
        );
        let mut line: Vec<egui::Pos2> = Vec::new();
        for point in &series.points {
            let TracePointValue::Boolean(value) = &point.value else {
                continue;
            };
            let Some(x) = time_axis.plot_x(rect, &point.root_tick) else {
                continue;
            };
            let position = egui::pos2(x, if *value { high } else { low });
            if let Some(previous) = line.last().copied() {
                line.push(egui::pos2(position.x, previous.y));
            }
            line.push(position);
            painter.circle_filled(position, 2.5, color);
        }
        if let Some(last) = line.last().copied().filter(|point| point.x < rect.right()) {
            line.push(egui::pos2(rect.right(), last.y));
        }
        painter.add(egui::Shape::line(line, egui::Stroke::new(1.8_f32, color)));
    }
}

fn paint_state_trace_series(
    painter: &egui::Painter,
    rect: egui::Rect,
    series: &[&TraceSeries],
    time_axis: &TraceTimeAxis,
) {
    let lane_height = rect.height() / display_index(series.len()).max(1.0);
    let separator = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(35));
    for (lane, series) in series.iter().copied().enumerate() {
        debug_assert_eq!(series.kind, TraceSeriesKind::State);
        let lane_top = rect.top() + display_index(lane) * lane_height;
        let lane_bottom = lane_top + lane_height;
        let center = (lane_top + lane_bottom) * 0.5;
        if lane > 0 {
            painter.line_segment(
                [
                    egui::pos2(rect.left(), lane_top),
                    egui::pos2(rect.right(), lane_top),
                ],
                separator,
            );
        }
        let color = trace_signal_color(&series.signal);
        painter.text(
            egui::pos2(rect.left() - 6.0, center),
            egui::Align2::RIGHT_CENTER,
            state_signal_label(&series.signal),
            egui::FontId::monospace(9.0),
            color,
        );
        painter.line_segment(
            [
                egui::pos2(rect.left(), center),
                egui::pos2(rect.right(), center),
            ],
            egui::Stroke::new(1.0_f32, color.gamma_multiply(0.28)),
        );
        let mut index = 0_usize;
        while let Some(point) = series.points.get(index) {
            let TracePointValue::State { .. } = &point.value else {
                index += 1;
                continue;
            };
            let run_end = state_same_time_run_end(series, index);
            let Some(x) = time_axis.plot_x(rect, &point.root_tick) else {
                index = run_end;
                continue;
            };
            let position = egui::pos2(x, center);
            if (index..run_end).any(|candidate| state_point_changed(series, candidate)) {
                painter.line_segment(
                    [
                        egui::pos2(x, lane_top + 5.0),
                        egui::pos2(x, lane_bottom - 5.0),
                    ],
                    egui::Stroke::new(2.0_f32, color),
                );
                painter.circle_filled(position, 3.2, color);
            } else {
                painter.circle_stroke(position, 2.6, egui::Stroke::new(1.2_f32, color));
            }
            let multiplicity = run_end - index;
            if multiplicity > 1 {
                painter.text(
                    egui::pos2(x + 4.0, lane_top + 3.0),
                    egui::Align2::LEFT_TOP,
                    format!("×{multiplicity}"),
                    egui::FontId::monospace(8.0),
                    color,
                );
            }
            index = run_end;
        }
    }
}

fn state_same_time_run_end(series: &TraceSeries, start: usize) -> usize {
    let Some(first) = series.points.get(start) else {
        return start;
    };
    series.points[start + 1..]
        .iter()
        .position(|point| point.root_tick != first.root_tick)
        .map_or(series.points.len(), |offset| start + 1 + offset)
}

fn state_point_changed(series: &TraceSeries, index: usize) -> bool {
    let Some(TracePoint {
        value: TracePointValue::State { encoding, .. },
        ..
    }) = series.points.get(index)
    else {
        return false;
    };
    let Some(TracePoint {
        value: TracePointValue::State {
            encoding: previous, ..
        },
        ..
    }) = index
        .checked_sub(1)
        .and_then(|prior| series.points.get(prior))
    else {
        return true;
    };
    encoding.bytes() != previous.bytes()
}

fn trace_cursor_label(signal: &TraceSignal, point: &TracePoint) -> String {
    match &point.value {
        TracePointValue::Analog { exact, .. } => format!(
            "{} = {exact}{} @ c{}:t{}:s{}",
            signal.label(),
            signal
                .unit_symbol
                .as_deref()
                .map_or_else(String::new, |unit| format!(" {unit}")),
            point.clock.get(),
            point.tick,
            point.sequence
        ),
        TracePointValue::Boolean(value) => format!(
            "{} = {value} @ c{}:t{}:s{}",
            signal.label(),
            point.clock.get(),
            point.tick,
            point.sequence
        ),
        TracePointValue::State { summary, encoding } => format!(
            "{} · {} [t{}] = {} @ c{}:t{}:s{}",
            signal.label(),
            signal.sample_type_name,
            signal.sample_type.get(),
            state_identity_label(summary, encoding),
            point.clock.get(),
            point.tick,
            point.sequence
        ),
    }
}

fn state_signal_label(signal: &TraceSignal) -> String {
    format!(
        "{}\n{} [t{}]",
        signal.label(),
        signal.sample_type_name,
        signal.sample_type.get()
    )
}

fn state_identity_label(summary: &str, encoding: &CanonicalTypedGraphValueEncoding) -> String {
    format!(
        "{summary} · sha256 {} · {} canonical bytes",
        digest_hex(encoding.digest()),
        encoding.bytes().len()
    )
}

fn trace_point_at_or_before<'a>(
    series: &'a TraceSeries,
    root_tick: &Rational,
) -> Option<&'a TracePoint> {
    series
        .points
        .iter()
        .rev()
        .find(|point| &point.root_tick <= root_tick)
}

fn graph_clock_set_label(clocks: &BTreeSet<GraphClockId>) -> String {
    if clocks.is_empty() {
        return "none".to_owned();
    }
    clocks
        .iter()
        .map(|clock| clock.get().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

const fn digital_signal_order(signal: RepresentativeControlSignal) -> u8 {
    match signal {
        RepresentativeControlSignal::ExternalPermit => 0,
        RepresentativeControlSignal::MeasurementWithinRange => 1,
        RepresentativeControlSignal::CombinedPermit => 2,
        RepresentativeControlSignal::Error
        | RepresentativeControlSignal::IntegralPrior
        | RepresentativeControlSignal::ClampedController
        | RepresentativeControlSignal::PermittedOutput => u8::MAX,
    }
}

fn trace_digital_order(signal: &TraceSignal) -> (u8, u32) {
    (
        signal.representative.map_or(u8::MAX, digital_signal_order),
        signal.probe.get(),
    )
}

const fn probe_edge_label(edge: GraphProbeEdge) -> &'static str {
    match edge {
        GraphProbeEdge::Rising => "rising-edge",
        GraphProbeEdge::Falling => "falling-edge",
        GraphProbeEdge::Either => "either-edge",
    }
}

fn paint_trace_grid(
    painter: &egui::Painter,
    rect: egui::Rect,
    time_axis: &TraceTimeAxis,
    minimum_value: f64,
    maximum_value: f64,
    label_ticks: bool,
) {
    paint_trace_time_grid(painter, rect, time_axis, label_ticks);
    let grid = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(28));
    for index in 0..=4 {
        let fraction = f64::from(index) / 4.0;
        let value = minimum_value + (maximum_value - minimum_value) * fraction;
        let y = plot_y(rect, value, minimum_value, maximum_value);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            grid,
        );
        painter.text(
            egui::pos2(rect.left() - 5.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{value:.1}"),
            egui::FontId::monospace(9.0),
            egui::Color32::GRAY,
        );
    }
}

fn paint_trace_time_grid(
    painter: &egui::Painter,
    rect: egui::Rect,
    time_axis: &TraceTimeAxis,
    label_ticks: bool,
) {
    let grid = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(28));
    for tick in time_axis.grid_ticks() {
        let Some(x) = time_axis.plot_x(rect, &tick) else {
            continue;
        };
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            grid,
        );
        if label_ticks {
            painter.text(
                egui::pos2(x, rect.bottom() + 4.0),
                egui::Align2::CENTER_TOP,
                tick.to_string(),
                egui::FontId::monospace(9.5),
                egui::Color32::GRAY,
            );
        }
    }
}

fn analog_trace_groups(traces: &[TraceSeries]) -> Result<Vec<AnalogTraceGroup<'_>>, String> {
    let mut by_type = BTreeMap::<GraphTypeId, Vec<&TraceSeries>>::new();
    for series in traces
        .iter()
        .filter(|series| series.kind == TraceSeriesKind::Analog)
    {
        by_type
            .entry(series.signal.sample_type)
            .or_default()
            .push(series);
    }
    if by_type.len() > MAXIMUM_ANALOG_TRACE_GROUPS {
        return Err(format!(
            "{} physical scalar value types exceed the bounded {}-pane trace display policy",
            by_type.len(),
            MAXIMUM_ANALOG_TRACE_GROUPS
        ));
    }
    by_type
        .into_iter()
        .map(|(sample_type, series)| {
            let first = series
                .first()
                .copied()
                .ok_or_else(|| "analog trace group is unexpectedly empty".to_owned())?;
            let sample_type_name = first.signal.sample_type_name.as_str();
            let unit_symbol = first.signal.unit_symbol.as_deref().ok_or_else(|| {
                format!(
                    "physical scalar trace type t{} has no canonical unit",
                    sample_type.get()
                )
            })?;
            if series.iter().any(|candidate| {
                candidate.signal.sample_type != sample_type
                    || candidate.signal.sample_type_name != sample_type_name
                    || candidate.signal.unit_symbol.as_deref() != Some(unit_symbol)
            }) {
                return Err(format!(
                    "physical scalar trace type t{} has inconsistent canonical metadata",
                    sample_type.get()
                ));
            }
            Ok(AnalogTraceGroup {
                sample_type,
                sample_type_name,
                unit_symbol,
                series,
            })
        })
        .collect()
}

fn state_trace_lanes(traces: &[TraceSeries]) -> Result<Vec<&TraceSeries>, String> {
    let mut state = traces
        .iter()
        .filter(|series| series.kind == TraceSeriesKind::State)
        .collect::<Vec<_>>();
    if state.len() > MAXIMUM_STATE_TRACE_LANES {
        return Err(format!(
            "{} state/event series exceed the bounded {}-lane trace display policy",
            state.len(),
            MAXIMUM_STATE_TRACE_LANES
        ));
    }
    state.sort_by_key(|series| series.signal.probe);
    Ok(state)
}

fn trace_value_bounds(series: &[&TraceSeries]) -> (f64, f64) {
    let mut minimum = 0.0_f64;
    let mut maximum = 0.0_f64;
    for point in series.iter().flat_map(|series| &series.points) {
        let TracePointValue::Analog { enclosure, .. } = &point.value else {
            continue;
        };
        minimum = minimum.min(enclosure[0]);
        maximum = maximum.max(enclosure[1]);
    }
    let span = (maximum - minimum).max(1.0);
    (minimum - span * 0.1, maximum + span * 0.1)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "workspace coordinates are bounded to one million and exactly representable in display f32"
)]
fn display_coordinate(value: i32) -> f32 {
    value as f32
}

#[allow(
    clippy::cast_precision_loss,
    reason = "front-panel coordinates are bounded to one million and exactly representable in display f32"
)]
fn display_panel_coordinate(value: u32) -> f32 {
    value as f32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "automatic layout is finite, integral, bounded presentation metadata"
)]
fn canonical_initial_coordinate(value: f32) -> Result<i32, String> {
    let widened = f64::from(value);
    if !value.is_finite()
        || value.fract() != 0.0
        || widened < f64::from(i32::MIN)
        || widened > f64::from(i32::MAX)
    {
        return Err("automatic canvas coordinate is not a canonical i32".to_owned());
    }
    Ok(widened as i32)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a finite pointer delta is rounded once into presentation-only integer canvas metadata"
)]
fn quantized_canvas_coordinate(origin: i32, delta: f32) -> Result<i32, String> {
    let projected = f64::from(origin) + f64::from(delta);
    if !projected.is_finite() {
        return Err("pointer delta is not finite".to_owned());
    }
    let rounded = projected.round();
    if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        return Err("pointer delta exceeds the canvas integer lattice".to_owned());
    }
    Ok(rounded as i32)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "bounded indices are projected only into non-authoritative egui coordinates"
)]
fn display_index(value: usize) -> f32 {
    value as f32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "certified finite enclosure coordinates are intentionally projected to egui f32"
)]
fn plot_y(rect: egui::Rect, value: f64, minimum: f64, maximum: f64) -> f32 {
    let fraction = ((value - minimum) / (maximum - minimum)).clamp(0.0, 1.0) as f32;
    rect.bottom() - rect.height() * fraction
}

fn trace_signal_color(signal: &TraceSignal) -> egui::Color32 {
    signal.representative.map_or_else(
        || match signal.probe.get() % 6 {
            0 => DIAGNOSTIC_CHANNEL_COLORS[0],
            1 => DIAGNOSTIC_CHANNEL_COLORS[1],
            2 => DIAGNOSTIC_CHANNEL_COLORS[2],
            3 => DIAGNOSTIC_CHANNEL_COLORS[3],
            4 => DIAGNOSTIC_CHANNEL_COLORS[4],
            _ => DIAGNOSTIC_CHANNEL_COLORS[5],
        },
        representative_signal_color,
    )
}

const fn representative_signal_color(signal: RepresentativeControlSignal) -> egui::Color32 {
    match signal {
        RepresentativeControlSignal::Error => egui::Color32::from_rgb(247, 196, 86),
        RepresentativeControlSignal::IntegralPrior => egui::Color32::from_rgb(91, 205, 224),
        RepresentativeControlSignal::ClampedController => egui::Color32::from_rgb(102, 221, 142),
        RepresentativeControlSignal::ExternalPermit => egui::Color32::from_rgb(255, 207, 92),
        RepresentativeControlSignal::MeasurementWithinRange => {
            egui::Color32::from_rgb(249, 153, 82)
        }
        RepresentativeControlSignal::CombinedPermit => egui::Color32::from_rgb(181, 143, 255),
        RepresentativeControlSignal::PermittedOutput => egui::Color32::from_rgb(245, 121, 169),
    }
}

fn short_kind(kind: &str) -> &str {
    kind.strip_prefix("control.").unwrap_or(kind)
}

fn show_node_identity_editors(
    ui: &mut egui::Ui,
    node: &NodeDefinition,
    domain_choices: &[ExecutionDomain],
    maximum_label_bytes: usize,
    label_text: &mut String,
) -> (Option<String>, Option<ExecutionDomain>) {
    let mut label_request = None;
    ui.horizontal_wrapped(|ui| {
        ui.monospace("label:");
        let response = ui.add(
            egui::TextEdit::singleline(label_text)
                .desired_width(260.0)
                .char_limit(maximum_label_bytes),
        );
        let apply = ui.small_button("apply label").clicked()
            || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if ui.small_button("reset label").clicked() {
            node.label().clone_into(label_text);
        }
        ui.weak(format!(
            "canonical metadata · {} / {maximum_label_bytes} UTF-8 bytes · never behavior identity",
            label_text.len()
        ));
        if apply {
            label_request = Some(label_text.clone());
        }
    });

    let mut selected_domain = node.domain();
    ui.horizontal_wrapped(|ui| {
        ui.monospace("execution placement:");
        egui::ComboBox::from_id_salt(("node_execution_domain", node.id().get()))
            .selected_text(domain_choice_label(selected_domain))
            .show_ui(ui, |ui| {
                for choice in domain_choices {
                    ui.selectable_value(
                        &mut selected_domain,
                        *choice,
                        domain_choice_label(*choice),
                    );
                }
            });
        ui.weak(format!(
            "{} audited concrete choice(s) · device identities come only from graph clocks/placements",
            domain_choices.len()
        ));
    });
    let domain_request = (selected_domain != node.domain()).then_some(selected_domain);
    (label_request, domain_request)
}

fn show_node_parameter_editors(
    ui: &mut egui::Ui,
    document: &GraphDocument,
    node: &NodeDefinition,
    drafts: &mut BTreeMap<(GraphNodeId, u32), String>,
) -> Option<(u32, String)> {
    if !node.parameters().is_empty() {
        ui.weak(
            "Exact literal text: quoted strings · hex\"bytes\" · [arrays] · {field:value} · some/none · ok/error",
        );
    }
    let mut request = None;
    for parameter in node.parameters() {
        let Some(initial) = parameter_edit_text(document, parameter.value()) else {
            ui.monospace(format!(
                "{} = {} · read-only literal shape",
                parameter.name(),
                typed_value_text(document, parameter.value())
            ));
            continue;
        };
        let maximum = parameter_text_limit(document, parameter.value().value_type());
        let draft = drafts.entry((node.id(), parameter.id())).or_insert(initial);
        ui.horizontal_wrapped(|ui| {
            ui.monospace(format!("{}:", parameter.name()));
            let response = ui.add(
                egui::TextEdit::singleline(draft)
                    .desired_width(180.0)
                    .char_limit(maximum),
            );
            let apply = ui.small_button("apply exact").clicked()
                || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            ui.weak(typed_value_text(document, parameter.value()));
            if apply {
                request = Some((parameter.id(), draft.clone()));
            }
        });
    }
    request
}

fn domain_choice_label(domain: ExecutionDomain) -> String {
    match domain {
        ExecutionDomain::HostExact => "HostExact".to_owned(),
        ExecutionDomain::Service { device_id } => {
            format!("Service · device {}…", digest_prefix16(device_id.0))
        }
        ExecutionDomain::Realtime { device_id } => {
            format!("Realtime · device {}…", digest_prefix16(device_id.0))
        }
    }
}

fn audited_domain_choices(
    document: &GraphDocument,
    allowed: ExecutionDomainSet,
) -> Vec<ExecutionDomain> {
    let mut devices = Vec::new();
    for clock in document.clocks() {
        if let ClockKind::DeviceCycle { device_id, .. } = clock.kind()
            && !devices.contains(&device_id)
        {
            devices.push(device_id);
        }
    }
    for node in document.nodes() {
        let device_id = match node.domain() {
            ExecutionDomain::HostExact => None,
            ExecutionDomain::Service { device_id } | ExecutionDomain::Realtime { device_id } => {
                Some(device_id)
            }
        };
        if let Some(device_id) = device_id
            && !devices.contains(&device_id)
        {
            devices.push(device_id);
        }
    }
    devices.sort_unstable_by_key(|device| device.0);

    let mut choices = Vec::new();
    if allowed.allows_host_exact() {
        choices.push(ExecutionDomain::HostExact);
    }
    if allowed.allows_service() {
        choices.extend(
            devices
                .iter()
                .copied()
                .map(|device_id| ExecutionDomain::Service { device_id }),
        );
    }
    if allowed.allows_realtime() {
        choices.extend(
            devices
                .iter()
                .copied()
                .map(|device_id| ExecutionDomain::Realtime { device_id }),
        );
    }
    choices
}

fn digest_prefix(digest: [u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut result = String::with_capacity(16);
    for byte in &digest[..8] {
        write!(&mut result, "{byte:02x}").expect("writing to String is infallible");
    }
    result
}

fn digest_hex(digest: Digest) -> String {
    use std::fmt::Write as _;

    let mut result = String::with_capacity(64);
    for byte in digest.0 {
        write!(&mut result, "{byte:02x}").expect("writing to String is infallible");
    }
    result
}

fn digest_prefix16(identity: [u8; 16]) -> String {
    use std::fmt::Write as _;

    let mut result = String::with_capacity(16);
    for byte in &identity[..8] {
        write!(&mut result, "{byte:02x}").expect("writing to String is infallible");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use alumina_interface_core::graph::{
        BaseDimensions, GraphComponentInputId, GraphLimits, GraphPortId, JobGraphHandle, NodeKind,
        RecordField, RecordFieldId, RecordValueField, ResourceGraphHandle, TypeDefinition,
        UnitDefinition, UnitId, replay_graph_component, replay_graph_hierarchy,
        replay_graph_probes, replay_graph_workspace,
    };

    struct AlgsHierarchyOffsets {
        selected_component: std::ops::Range<usize>,
        hierarchy: std::ops::Range<usize>,
        source_map: std::ops::Range<usize>,
    }

    fn persisted_algs(bytes: &[u8]) -> String {
        let mut persisted = String::with_capacity(
            PERSISTED_AUTHORING_SESSION_PREFIX.len() + bytes.len().saturating_mul(2),
        );
        persisted.push_str(PERSISTED_AUTHORING_SESSION_PREFIX);
        append_lower_hex(&mut persisted, bytes);
        persisted
    }

    fn replace_exact_embedded_bytes(session: &mut [u8], current: &[u8], replacement: &[u8]) {
        assert_eq!(current.len(), replacement.len());
        let positions = session
            .windows(current.len())
            .enumerate()
            .filter_map(|(index, candidate)| (candidate == current).then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(positions.len(), 1);
        session[positions[0]..positions[0] + current.len()].copy_from_slice(replacement);
    }

    fn algs_hierarchy_offsets(bytes: &[u8]) -> AlgsHierarchyOffsets {
        const FIXED_HEADER: usize = 4 + 2 + 2 + 6 * 8;

        fn take_length(bytes: &[u8], cursor: &mut usize) -> usize {
            let end = *cursor + 4;
            let length =
                usize::try_from(u32::from_le_bytes(bytes[*cursor..end].try_into().unwrap()))
                    .unwrap();
            *cursor = end;
            length
        }

        let mut cursor = FIXED_HEADER;
        for _ in 0..3 {
            let length = take_length(bytes, &mut cursor);
            cursor += length;
        }
        assert_eq!(bytes[cursor], 1);
        cursor += 1;
        let selected_component = cursor..cursor + 32;
        cursor = selected_component.end;
        let hierarchy_length = take_length(bytes, &mut cursor);
        let hierarchy = cursor..cursor + hierarchy_length;
        cursor = hierarchy.end;
        let source_map_length = take_length(bytes, &mut cursor);
        let source_map = cursor..cursor + source_map_length;
        cursor = source_map.end;
        assert_eq!(cursor, bytes.len());
        AlgsHierarchyOffsets {
            selected_component,
            hierarchy,
            source_map,
        }
    }

    fn assert_exact_component_package_equal(
        actual: &ComponentPackage,
        expected: &ComponentPackage,
    ) {
        assert_eq!(actual.document, expected.document);
        assert_eq!(actual.encoding, expected.encoding);
        assert_eq!(actual.hierarchy.document, expected.hierarchy.document);
        assert_eq!(actual.hierarchy.encoding, expected.hierarchy.encoding);
        assert_eq!(actual.hierarchy.flattening, expected.hierarchy.flattening);
        assert_eq!(actual.hierarchy.source_map, expected.hierarchy.source_map);
    }

    fn root_input_component(
        base: &GraphComponentDocument,
    ) -> (GraphComponentDocument, CanonicalGraphComponentEncoding) {
        let mut workspace = base.workspace().clone();
        workspace.disconnect(GraphWireId::new(6)).unwrap();
        let document = GraphComponentDocument::try_new(
            base.limits(),
            base.revision(),
            base.component_version(),
            "control.root_wired_pid",
            2,
            base.next_output_id(),
            base.next_panel_item_id(),
            workspace,
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "error_samples",
                WireEndpoint {
                    node: GraphNodeId::new(8),
                    port: GraphPortId::new(1),
                },
            )],
            base.outputs().to_vec(),
            base.panel_items().to_vec(),
        )
        .unwrap();
        let encoding = encode_graph_component(&document).unwrap();
        (document, encoding)
    }

    #[test]
    fn cached_job_proof_starts_from_complete_reconciled_participant_set() {
        let workspace = ExactControlWorkspace::try_new().unwrap();
        let proof = &workspace.cached_jobs;
        assert_eq!(proof.catalog.entries().len(), 2);
        assert_eq!(proof.workspace.graph().nodes().len(), 1);
        assert_eq!(proof.selected_node, Some(GraphNodeId::new(1)));
        assert_eq!(proof.selected_slot, 0);
        assert!(proof.status.contains("cache reconciliation"));
        validate_cached_job_workspace(&proof.catalog, &proof.registry, &proof.workspace).unwrap();
        for slot in CACHED_JOB_REFERENCE_SLOTS {
            assert_eq!(
                cached_job_handle_at_path(
                    proof.workspace.graph().nodes().first().unwrap(),
                    proof.registry.context_schema(),
                    slot.path,
                ),
                Some(proof.catalog.entries()[0].handle())
            );
        }
        let replay = replay_graph_workspace(
            proof.encoding.bytes(),
            proof.workspace.limits(),
            proof.workspace.graph().schema().limits(),
        )
        .unwrap();
        assert_eq!(replay.document(), &proof.workspace);
        assert_eq!(replay.encoding(), &proof.encoding);
    }

    #[test]
    fn cached_job_ui_rebind_add_and_unified_session_history_stay_catalog_bound() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let node = workspace.cached_jobs.selected_node.unwrap();
        let placement = workspace.cached_jobs.workspace.placement(node);
        let initial = workspace.cached_jobs.encoding.clone();
        let initial_session = workspace.authoring_session_encoding().unwrap();

        workspace.cached_jobs.selected_entry = 1;
        workspace.apply_cached_job_action(CachedJobGraphAction::Rebind);
        assert_ne!(workspace.cached_jobs.encoding, initial);
        assert_eq!(workspace.cached_jobs.workspace.placement(node), placement);
        assert_eq!(workspace.history.undo_len(), 1);
        assert_eq!(workspace.history.redo_len(), 0);
        assert_eq!(
            cached_job_handle_at_path(
                workspace.cached_jobs.workspace.graph().node(node).unwrap(),
                workspace.cached_jobs.registry.context_schema(),
                CACHED_JOB_REFERENCE_SLOTS[0].path,
            ),
            Some(workspace.cached_jobs.catalog.entries()[1].handle())
        );
        for selected_slot in 1..CACHED_JOB_REFERENCE_SLOTS.len() {
            workspace.cached_jobs.selected_slot = selected_slot;
            workspace.apply_cached_job_action(CachedJobGraphAction::Rebind);
        }
        assert_eq!(workspace.history.undo_len(), 3);
        for slot in CACHED_JOB_REFERENCE_SLOTS {
            assert_eq!(
                cached_job_handle_at_path(
                    workspace.cached_jobs.workspace.graph().node(node).unwrap(),
                    workspace.cached_jobs.registry.context_schema(),
                    slot.path,
                ),
                Some(workspace.cached_jobs.catalog.entries()[1].handle())
            );
        }

        workspace.apply_cached_job_action(CachedJobGraphAction::Add);
        assert_eq!(workspace.cached_jobs.workspace.graph().nodes().len(), 2);
        assert_eq!(workspace.history.undo_len(), 4);
        let edited = workspace.cached_jobs.encoding.clone();
        let edited_session = workspace.authoring_session_encoding().unwrap();
        let replay = replay_graph_workspace(
            edited.bytes(),
            workspace.cached_jobs.workspace.limits(),
            workspace.cached_jobs.workspace.graph().schema().limits(),
        )
        .unwrap();
        validate_cached_job_workspace(
            &workspace.cached_jobs.catalog,
            &workspace.cached_jobs.registry,
            replay.document(),
        )
        .unwrap();
        assert_eq!(replay.encoding(), &edited);

        workspace.navigate_history(false);
        assert_eq!(workspace.cached_jobs.workspace.graph().nodes().len(), 1);
        assert_eq!(workspace.history.redo_len(), 1);
        workspace.navigate_history(true);
        assert_eq!(workspace.cached_jobs.encoding, edited);
        assert_eq!(workspace.cached_jobs.workspace.graph().nodes().len(), 2);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            edited_session
        );

        for _ in 0..4 {
            workspace.navigate_history(false);
        }
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            initial_session
        );

        let proof = &workspace.cached_jobs;
        let mut raw = proof.workspace.clone();
        let raw_node = raw.graph().nodes()[0].id();
        let raw_handle = JobGraphHandle {
            global_job_digest: Digest([0x9a; 32]),
            ..proof.catalog.entries()[0].handle()
        };
        let parameter = raw.graph().node(raw_node).unwrap().parameters()[0]
            .value()
            .replacing_value_at_path(
                proof.registry.context_schema(),
                CACHED_JOB_FALLBACK_PATH.as_slice(),
                GraphValue::JobHandle(raw_handle),
            )
            .unwrap();
        raw.set_parameter(raw_node, CACHED_JOB_PARAMETER, parameter)
            .unwrap();
        assert!(
            validate_cached_job_workspace(&proof.catalog, &proof.registry, &raw)
                .unwrap_err()
                .contains("raw, stale, or foreign")
        );
    }

    #[test]
    fn representative_layout_is_bounded_acyclic_and_keeps_feedback_visible() {
        let workspace = ExactControlWorkspace::try_new().unwrap();
        assert_eq!(workspace.presentation.nodes.len(), 21);
        assert_eq!(workspace.presentation.wires.len(), 25);
        assert_eq!(
            workspace
                .presentation
                .wires
                .values()
                .filter(|wire| wire.feedback_lane.is_some())
                .count(),
            2
        );
        let nodes: Vec<_> = workspace.presentation.nodes.values().collect();
        for (index, left) in nodes.iter().enumerate() {
            for right in &nodes[index + 1..] {
                assert!(!left.rect.intersects(right.rect));
            }
        }
        assert!(nodes.iter().any(|node| node.rank == 0));
        let replay = replay_graph_workspace(
            workspace.workspace_encoding.bytes(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(workspace.workspace_encoding.bytes().len(), 3_755);
        assert_eq!(
            workspace.workspace_encoding.digest().0,
            [
                0xbf, 0x51, 0x35, 0xc3, 0x9b, 0x67, 0xc4, 0x6a, 0x3a, 0x59, 0x08, 0xd4, 0xd0, 0xd8,
                0xa1, 0xd1, 0x3d, 0x06, 0x5b, 0x59, 0x23, 0x18, 0x90, 0xe8, 0xfb, 0xdd, 0xa8, 0x18,
                0xf0, 0x64, 0xae, 0x16,
            ]
        );
        assert_eq!(replay.document(), &workspace.workspace);
        assert_eq!(replay.encoding(), &workspace.workspace_encoding);
        assert!(workspace.reference_trace_is_current());
        let probes = workspace.probes.as_ref().unwrap();
        assert_eq!(probes.document.probes().len(), 7);
        assert_eq!(probes.encoding.bytes().len(), 407);
        assert_eq!(
            probes.encoding.digest().0,
            [
                0x50, 0x95, 0x5d, 0xa7, 0xb4, 0x46, 0x4a, 0x02, 0xf6, 0xe3, 0xea, 0xce, 0x1d, 0x51,
                0xa3, 0xe9, 0xd0, 0x8f, 0x56, 0xcd, 0xfa, 0x9c, 0x66, 0x12, 0x59, 0xc3, 0x31, 0x95,
                0xb1, 0x5e, 0xf2, 0x23,
            ]
        );
        let probe_replay = replay_graph_probes(
            probes.encoding.bytes(),
            &workspace.workspace,
            GraphProbeLimits::interactive(),
        )
        .unwrap();
        assert_eq!(probe_replay.document(), &probes.document);
        assert_eq!(probe_replay.encoding(), &probes.encoding);
        let GraphProbeTriggerResolution::Matched(trigger) = workspace.trigger_resolution().unwrap()
        else {
            panic!("canonical interlock trigger did not match");
        };
        assert_eq!(trigger.trigger().probe(), GraphProbeId::new(5));
        assert_eq!(trigger.trigger().edge(), GraphProbeEdge::Falling);
        assert_eq!(trigger.trigger_tick(), 3);
        assert_eq!((trigger.first_tick(), trigger.last_tick()), (1, 5));
        assert_eq!(workspace.cursor_root_tick, Rational::from(30));
        assert_eq!(workspace.palette.len(), 13);
        assert!(workspace.palette.iter().all(|entry| {
            workspace
                .fixture
                .registry()
                .semantic_registry()
                .schema(entry.prototype.kind())
                .is_some()
        }));
    }

    #[test]
    fn representative_trace_retains_four_analog_and_three_causal_boolean_series() {
        let workspace = ExactControlWorkspace::try_new().unwrap();
        assert_eq!(workspace.traces.len(), SIGNALS.len());
        assert!(
            workspace
                .traces
                .iter()
                .all(|series| series.points.len() == 6)
        );
        assert_eq!(
            workspace
                .traces
                .iter()
                .filter(|series| series.kind == TraceSeriesKind::Analog)
                .count(),
            4
        );
        assert_eq!(
            workspace
                .traces
                .iter()
                .filter(|series| series.kind == TraceSeriesKind::Boolean)
                .count(),
            3
        );
        let mut digital_order = workspace
            .traces
            .iter()
            .filter(|series| series.kind == TraceSeriesKind::Boolean)
            .map(|series| series.signal.representative.unwrap())
            .collect::<Vec<_>>();
        digital_order.sort_by_key(|signal| digital_signal_order(*signal));
        assert_eq!(
            digital_order,
            [
                RepresentativeControlSignal::ExternalPermit,
                RepresentativeControlSignal::MeasurementWithinRange,
                RepresentativeControlSignal::CombinedPermit,
            ]
        );
        let external = workspace
            .traces
            .iter()
            .find(|series| {
                series.signal.representative == Some(RepresentativeControlSignal::ExternalPermit)
            })
            .unwrap();
        assert_eq!(
            external
                .points
                .iter()
                .map(|point| match &point.value {
                    TracePointValue::Boolean(value) => *value,
                    TracePointValue::Analog { .. } | TracePointValue::State { .. } => {
                        panic!("external permit trace changed value kind")
                    }
                })
                .collect::<Vec<_>>(),
            [true, true, true, true, false, false]
        );
        let range = workspace
            .traces
            .iter()
            .find(|series| {
                series.signal.representative
                    == Some(RepresentativeControlSignal::MeasurementWithinRange)
            })
            .unwrap();
        assert_eq!(
            range
                .points
                .iter()
                .map(|point| match &point.value {
                    TracePointValue::Boolean(value) => *value,
                    TracePointValue::Analog { .. } | TracePointValue::State { .. } => {
                        panic!("range interlock trace changed value kind")
                    }
                })
                .collect::<Vec<_>>(),
            [true, true, true, false, false, false]
        );
    }

    #[test]
    fn tinybee_catalog_builds_one_executable_pair_and_exact_offline_package() {
        let mut proof = tinybee_resource_proof().unwrap();
        assert_eq!(proof.catalog.advertised_resource_count(), 4);
        assert_eq!(proof.catalog.entries().len(), 4);
        assert_eq!(proof.workspace.graph().nodes().len(), 2);
        assert_eq!(proof.workspace.graph().wires().len(), 1);
        assert_eq!(proof.pair_node, GraphNodeId::new(1));
        for (slot, entry) in [0, 1].into_iter().enumerate() {
            proof.reference_slot = slot;
            assert_eq!(proof.selected_reference_entry(), Some(entry));
        }
        assert_eq!(proof.catalog_index, 3);
        assert!(!proof.selected_is_present());
        assert_eq!(proof.deployment.package_bytes, 4_096);
        assert_eq!(proof.deployment.first, ResourceId::Gpio(22));
        assert_eq!(proof.deployment.second, ResourceId::Gpio(32));
        assert_eq!(proof.deployment.realtime_period_cycles, 240_000);
        assert_eq!(proof.deployment.realtime_wcet_cycles, 160);
        assert_eq!(
            proof.deployment.actor_replay.truth_table,
            [false, false, false, true]
        );
        assert_eq!(
            proof.deployment.actor_replay.ordered_reads,
            [ResourceId::Gpio(22), ResourceId::Gpio(32)]
        );
        assert_eq!(
            proof.deployment.actor_replay.fault_reads,
            [ResourceId::Gpio(22), ResourceId::Gpio(32)]
        );
        assert_eq!(
            proof.deployment.actor_replay.terminal_fault,
            GraphExecutionFault::ResourceUnavailable
        );
        assert_ne!(
            proof.deployment.actor_replay.success_evidence.digest(),
            proof.deployment.actor_replay.fault_evidence.digest()
        );
        assert_eq!(
            digest_hex(proof.deployment.actor_replay.success_evidence.digest()),
            "a26b41965997461d109d2aeec4b562eb0d2c7c3dfe61870139c2bd2293f12147"
        );
        assert_eq!(
            digest_hex(proof.deployment.actor_replay.fault_evidence.digest()),
            "5e122937631516da3b57fd9f3e1f1d39a473ada6bf7dbad9c04b5121a4b89f6c"
        );
        assert_eq!(
            proof
                .workspace
                .graph()
                .node(proof.pair_node)
                .unwrap()
                .domain(),
            ExecutionDomain::Realtime {
                device_id: DeviceId([0x54; 16]),
            }
        );
        let initial_encoding = proof.encoding.clone();
        let initial_deployment = proof.deployment.clone();
        proof.reset();
        assert_eq!(proof.workspace.graph().nodes().len(), 2);
        assert_eq!(proof.workspace.graph().wires().len(), 1);
        assert_eq!(proof.pair_node, GraphNodeId::new(1));
        assert_eq!(proof.workspace.next_node_id(), 3);
        assert_eq!(proof.workspace.next_wire_id(), 2);
        assert_eq!(proof.catalog_index, 3);
        assert_eq!(proof.reference_slot, 0);
        assert_eq!(proof.encoding, initial_encoding);
        assert_eq!(proof.deployment, initial_deployment);
    }

    #[test]
    fn tinybee_replay_files_require_the_current_artifact_and_never_mutate_the_draft() {
        let proof = tinybee_resource_proof().unwrap();
        let retained = proof.deployment.clone();
        let success = &proof.deployment.actor_replay.success_evidence;
        let fault = &proof.deployment.actor_replay.fault_evidence;

        assert_eq!(success.encoded().len(), 528);
        assert_eq!(fault.encoded().len(), 278);
        assert!(
            target_replay_file_event_status(
                BoundedFileEvent::Import(Ok(success.encoded().to_vec())),
                "success",
                success,
                &proof.deployment.report,
                proof.catalog.target(),
            )
            .contains("after fresh fixed-memory firmware-actor replay")
        );
        assert!(
            target_replay_file_event_status(
                BoundedFileEvent::Import(Ok(fault.encoded().to_vec())),
                "fault",
                fault,
                &proof.deployment.report,
                proof.catalog.target(),
            )
            .contains("after fresh fixed-memory firmware-actor replay")
        );

        let mut tampered = success.encoded().to_vec();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(
            target_replay_file_event_status(
                BoundedFileEvent::Import(Ok(tampered)),
                "success",
                success,
                &proof.deployment.report,
                proof.catalog.target(),
            )
            .contains("digest does not match its bytes")
        );
        assert_eq!(proof.deployment, retained);
    }

    #[test]
    fn tinybee_executable_pair_selector_preserves_sibling_and_relowers_atomically() {
        let mut proof = tinybee_resource_proof().unwrap();
        let node = GraphNodeId::new(1);
        proof.reference_slot = 1;
        assert_eq!(proof.selected_reference_entry(), Some(1));
        proof.catalog_index = 3;
        let before_encoding = proof.encoding.clone();
        let before_deployment = proof.deployment.clone();
        let before_node_cursor = proof.workspace.next_node_id();
        let before_wire_cursor = proof.workspace.next_wire_id();
        let before_placement = proof.workspace.placement(node);

        assert!(!proof.selected_is_present());
        proof.rebind_reference();
        assert!(
            proof
                .status
                .contains("resources.interlock from GPIO 32 to GPIO 35")
        );
        assert_ne!(proof.encoding, before_encoding);
        assert_ne!(
            proof.deployment.package_digest,
            before_deployment.package_digest
        );
        assert_ne!(
            proof.deployment.implementation_digest,
            before_deployment.implementation_digest
        );
        assert_ne!(
            proof.deployment.actor_replay.success_evidence.digest(),
            before_deployment.actor_replay.success_evidence.digest()
        );
        assert_ne!(
            proof.deployment.actor_replay.fault_evidence.digest(),
            before_deployment.actor_replay.fault_evidence.digest()
        );
        assert_eq!(proof.deployment.first, ResourceId::Gpio(22));
        assert_eq!(proof.deployment.second, ResourceId::Gpio(35));
        assert_eq!(
            proof.deployment.actor_replay.truth_table,
            [false, false, false, true]
        );
        assert_eq!(
            proof.deployment.actor_replay.ordered_reads,
            [ResourceId::Gpio(22), ResourceId::Gpio(35)]
        );
        assert_eq!(
            proof.deployment.actor_replay.fault_reads,
            [ResourceId::Gpio(22), ResourceId::Gpio(35)]
        );
        assert_eq!(
            proof.deployment.actor_replay.terminal_fault,
            GraphExecutionFault::ResourceUnavailable
        );
        assert_eq!(
            digest_hex(proof.deployment.actor_replay.success_evidence.digest()),
            "cf2215451f222f8b402b85285ae889ffd67c06e7de8eb3d9c03c8d075dc2a8df"
        );
        assert_eq!(
            digest_hex(proof.deployment.actor_replay.fault_evidence.digest()),
            "e99110e8980e319389b8fe7731a6087375a465e4151289c37edaec0ed133b176"
        );
        assert_eq!(proof.workspace.next_node_id(), before_node_cursor);
        assert_eq!(proof.workspace.next_wire_id(), before_wire_cursor);
        assert_eq!(proof.workspace.placement(node), before_placement);
        assert!(proof.selected_is_present());
        assert_eq!(proof.selected_reference_entry(), Some(3));
        for (slot, entry) in [(0, 0), (1, 3)] {
            proof.reference_slot = slot;
            assert_eq!(proof.selected_reference_entry(), Some(entry));
        }

        let retained_encoding = proof.encoding.clone();
        let retained_deployment = proof.deployment.clone();
        proof.catalog_index = 3;
        proof.reference_slot = 0;
        proof.rebind_reference();
        assert_eq!(proof.encoding, retained_encoding);
        assert_eq!(proof.deployment, retained_deployment);
        assert!(proof.status.contains("already carried"));
        proof.reference_slot = 0;
        assert_eq!(
            proof.selected_reference_entry(),
            Some(0),
            "duplicate rejection changed the selected composite sibling"
        );
    }

    #[test]
    fn tinybee_simulator_explorer_keeps_all_three_diagnostic_authorities_distinct() {
        let panel = tinybee_board_explorer().unwrap();
        assert_eq!(
            panel.snapshot.identity(),
            calculate_identity(&alumina_sim::capability::package()).unwrap()
        );
        assert_eq!(panel.snapshot.visuals().len(), 1);
        assert_eq!(panel.snapshot.visuals()[0].id(), "simulated-topology");
        assert_eq!(panel.snapshot.visuals()[0].hotspots().len(), 4);
        assert_eq!(panel.selected, Some(ResourceId::Gpio(33)));
        let resources = panel.snapshot.resources();
        let matching_resources = |filter| {
            resources
                .iter()
                .filter(|resource| resource_matches_filter(resource, filter))
                .count()
        };
        assert_eq!(panel.snapshot.diagnostic_overview().resource_count, 4);
        assert_eq!(panel.snapshot.digital_capture().resource_count, 4);
        assert_eq!(
            matching_resources(BoardResourceFilter::DiagnosticObservable),
            4
        );
        assert_eq!(
            matching_resources(BoardResourceFilter::DiagnosticClosed),
            resources.len() - 4
        );
        assert_eq!(
            matching_resources(BoardResourceFilter::DigitallyCapturable),
            4
        );
        assert_eq!(
            matching_resources(BoardResourceFilter::CaptureClosed),
            resources.len() - 4
        );
        assert_eq!(matching_resources(BoardResourceFilter::GraphReadable), 4);
        assert_eq!(
            matching_resources(BoardResourceFilter::GraphClosed),
            resources.len() - 4
        );
        let x_step = resources
            .iter()
            .find(|resource| resource_matches_search(resource, "axis.x.step"))
            .unwrap();
        assert!(x_step.descriptor().hazardous_output);
        assert!(!x_step.is_diagnostic_observable());
        assert!(!x_step.is_digitally_capturable());
        assert!(!x_step.is_graph_addressable());
        assert_eq!(display_bytes(8 * 1_024 * 1_024), "8 MiB");
        assert_eq!(
            panel.diagnostics.context().capability,
            panel.snapshot.identity()
        );
        assert_eq!(panel.diagnostics.overview_bytes(), 320);
        assert_eq!(panel.diagnostics.capture_bytes(), 512);
        assert_eq!(panel.diagnostics.overview_samples().len(), 4);
        assert_eq!(panel.diagnostics.capture_channels().len(), 4);
        assert_eq!(panel.diagnostics.transitions().len(), 14);
        assert_eq!(
            panel
                .diagnostics
                .overview_sample(ResourceId::Gpio(33))
                .unwrap()
                .value,
            ResourceValue::Boolean(false)
        );
        assert_eq!(
            panel
                .diagnostics
                .digital_level_at(ResourceId::Gpio(33), panel.diagnostic_cursor_offset),
            Some(DigitalLevel::High)
        );
        assert!(
            panel
                .diagnostics
                .capture_quality_flags()
                .contains(CaptureQualityFlags::CLOCK_UNQUALIFIED)
        );
    }

    #[test]
    fn probe_ui_mutations_retain_canonical_sidecar_and_never_touch_graph() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let graph = workspace.workspace.clone();
        assert_eq!(
            workspace.probes.as_ref().unwrap().document.trigger(),
            Some(GraphProbeTrigger::new(
                GraphProbeId::new(5),
                GraphProbeEdge::Falling,
                2,
                2,
            ))
        );
        workspace.trigger_pre_samples = 1;
        workspace.trigger_post_samples = 1;
        workspace.set_probe_trigger(GraphProbeId::new(7), GraphProbeEdge::Falling);
        let GraphProbeTriggerResolution::Matched(trigger) = workspace.trigger_resolution().unwrap()
        else {
            panic!("external-permit falling edge did not match");
        };
        assert_eq!(trigger.trigger_tick(), 4);
        assert_eq!((trigger.first_tick(), trigger.last_tick()), (3, 5));
        assert_eq!(workspace.cursor_root_tick, Rational::from(40));
        assert_eq!(workspace.workspace, graph);

        let canonical = workspace.probes.as_ref().unwrap().encoding.clone();
        workspace.trigger_pre_samples = 4_096;
        workspace.trigger_post_samples = 0;
        workspace.set_probe_trigger(GraphProbeId::new(7), GraphProbeEdge::Either);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, canonical);
        assert!(workspace.probe_status.contains("rejected without mutation"));
        workspace.clear_probe_trigger();
        assert_eq!(workspace.probes.as_ref().unwrap().document.trigger(), None);
        assert_eq!(workspace.workspace, graph);

        let source = WireEndpoint {
            node: GraphNodeId::new(16),
            port: GraphPortId::new(3),
        };
        workspace.add_probe(source);
        assert_eq!(workspace.workspace, graph);
        let package = workspace.probes.as_ref().unwrap();
        assert!(package.document.observes(source));
        let id = package
            .document
            .probes()
            .iter()
            .find(|probe| probe.source() == source)
            .unwrap()
            .id();
        workspace.remove_probe(id);
        assert_eq!(workspace.workspace, graph);
        assert!(!workspace.probes.as_ref().unwrap().document.observes(source));
    }

    fn assert_hierarchy_source_map(initial: &ComponentPackage) {
        assert_eq!(initial.hierarchy.flattening.node_provenance().len(), 21);
        assert_eq!(initial.hierarchy.flattening.wire_provenance().len(), 25);
        assert_eq!(
            hierarchy_node_correlation(initial, GraphNodeId::new(8)).as_deref(),
            Some("[1/1]:n8 → flat n10")
        );
        assert_eq!(
            hierarchy_endpoint_correlation(
                initial,
                WireEndpoint {
                    node: GraphNodeId::new(8),
                    port: GraphPortId::new(1),
                },
            )
            .as_deref(),
            Some("[1/1]:n8.p1 → flat n10.p1")
        );
        assert_eq!(initial.hierarchy.source_map.bytes().len(), 2_550);
        assert_eq!(
            initial.hierarchy.source_map.digest().0,
            [
                0xdb, 0xfa, 0xf6, 0x92, 0x55, 0xa1, 0xa4, 0x15, 0x93, 0x29, 0x76, 0x15, 0x23, 0xfb,
                0xe1, 0x20, 0xd8, 0xb9, 0x56, 0x54, 0x8a, 0xdb, 0x4b, 0xbf, 0x19, 0x58, 0xe7, 0x3a,
                0xb0, 0x14, 0xbb, 0x77,
            ]
        );
        let replay = replay_graph_hierarchy_source_map(
            initial.hierarchy.source_map.bytes(),
            &initial.hierarchy.document,
            GraphHierarchySourceMapLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.encoding(), &initial.hierarchy.source_map);
        assert_eq!(replay.flattening(), &initial.hierarchy.flattening);
    }

    fn assert_recursive_hierarchy_package(initial: &ComponentPackage) {
        assert_eq!(initial.hierarchy.document.dependencies().len(), 2);
        assert_eq!(initial.hierarchy.document.instances().len(), 2);
        assert_eq!(initial.hierarchy.document.flattened_instance_count(), 2);
        assert_eq!(initial.hierarchy.flattening.instances().len(), 2);
        assert_eq!(
            initial.hierarchy.flattening.instances()[0].source_path(),
            [GraphNodeId::new(1)]
        );
        assert!(
            initial.hierarchy.flattening.instances()[0]
                .nodes()
                .is_empty()
        );
        assert_eq!(
            initial.hierarchy.flattening.instances()[1].source_path(),
            [GraphNodeId::new(1), GraphNodeId::new(1)]
        );
        assert_eq!(
            initial.hierarchy.flattening.instances()[1].nodes().len(),
            21
        );
        assert_eq!(initial.hierarchy.encoding.bytes().len(), 7_124);
        assert_eq!(
            initial.hierarchy.encoding.digest().0,
            [
                0xf9, 0x75, 0x10, 0x73, 0x01, 0x58, 0x28, 0xf2, 0x01, 0x54, 0xa5, 0x53, 0x6d, 0x8b,
                0x21, 0x7a, 0x7d, 0x38, 0x43, 0xe2, 0xd6, 0x3d, 0x3b, 0x56, 0x89, 0xfc, 0xfb, 0x8c,
                0x23, 0x79, 0x80, 0x6a,
            ]
        );
        assert_eq!(initial.hierarchy.flattening.encoding().bytes().len(), 3_755);
        assert_eq!(
            initial.hierarchy.flattening.encoding().digest().0,
            [
                0x68, 0x04, 0xb9, 0x64, 0x53, 0x5d, 0x08, 0xb9, 0xce, 0xea, 0xd3, 0xd4, 0x38, 0x91,
                0xc3, 0xae, 0x4c, 0x5a, 0xa5, 0xce, 0x38, 0xb0, 0x15, 0xc3, 0x4b, 0x4b, 0x38, 0x5b,
                0xfa, 0x32, 0x57, 0xd4,
            ]
        );
        assert_eq!(initial.hierarchy.document.flattened_node_count(), 21);
        assert_eq!(initial.hierarchy.document.flattened_wire_count(), 25);
        assert_eq!(
            initial
                .hierarchy
                .flattening
                .workspace()
                .graph()
                .nodes()
                .len(),
            21
        );
        assert_eq!(
            initial
                .hierarchy
                .flattening
                .workspace()
                .graph()
                .wires()
                .len(),
            25
        );
        let hierarchy_replay = replay_graph_hierarchy(
            initial.hierarchy.encoding.bytes(),
            GraphHierarchyLimits::interactive(),
            GraphComponentLimits::interactive(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(hierarchy_replay.document(), &initial.hierarchy.document);
        assert_eq!(hierarchy_replay.encoding(), &initial.hierarchy.encoding);
        assert_hierarchy_source_map(initial);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one exact node/wire source-navigation lifecycle proves occurrence resolution, transient selection, rejection retention, history isolation, and persistence isolation"
    )]
    fn flattened_source_browser_opens_exact_nodes_and_wires_without_authoring_mutation() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.mark_persisted();
        workspace.reconcile_hierarchy_source_browser();
        let retained_session = workspace.authoring_session_encoding().unwrap();
        let retained_component = workspace.component.as_ref().unwrap().clone();
        let retained_history = workspace.history.clone();
        let authoritative = retained_component.encoding.digest();
        let wrapper = retained_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .digest();
        let node_mapping = retained_component.hierarchy.flattening.node_provenance()[7].clone();
        let node_origin = node_mapping.origin().clone();
        let GraphHierarchyNodeOrigin::Component {
            source_path,
            component,
            node,
        } = &node_origin
        else {
            panic!("reference final node did not retain a component origin");
        };
        let expected_node = *node;
        assert_eq!(source_path, &[GraphNodeId::new(1), GraphNodeId::new(1)]);
        assert_eq!(*component, authoritative);

        workspace.selected_hierarchy_component = Some(wrapper);
        workspace.reconcile_hierarchy_selection(None);
        workspace.selected_node = None;
        let node_selection = HierarchySourceSelection::Node {
            flattened: node_mapping.flattened_node(),
            origin: node_origin,
        };
        workspace.open_hierarchy_source(node_selection.clone());

        assert_eq!(workspace.selected_hierarchy_component, Some(authoritative));
        assert_eq!(
            workspace.selected_hierarchy_instance,
            Some(GraphNodeId::new(1))
        );
        assert_eq!(workspace.selected_node, Some(expected_node));
        assert_eq!(
            workspace.hierarchy_source_browser.last_opened,
            Some(node_selection)
        );
        assert!(workspace.hierarchy_source_browser.scroll_pending);
        assert!(
            workspace
                .hierarchy_source_browser
                .status
                .contains("exact occurrence [1/1]")
        );
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &retained_component,
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());

        let wire_mapping = retained_component.hierarchy.flattening.wire_provenance()[0].clone();
        let wire_origin = wire_mapping.origin().clone();
        let GraphHierarchyWireOrigin::Component {
            source_path,
            component,
            wire,
        } = &wire_origin
        else {
            panic!("reference final wire did not retain a component origin");
        };
        assert_eq!(source_path, &[GraphNodeId::new(1), GraphNodeId::new(1)]);
        assert_eq!(*component, authoritative);
        let expected_target = retained_component
            .hierarchy
            .document
            .dependency(*component)
            .unwrap()
            .document()
            .workspace()
            .graph()
            .wires()
            .iter()
            .find(|candidate| candidate.id() == *wire)
            .unwrap()
            .target()
            .node;
        let wire_selection = HierarchySourceSelection::Wire {
            flattened: wire_mapping.flattened_wire(),
            origin: wire_origin,
        };
        workspace.open_hierarchy_source(wire_selection.clone());

        assert_eq!(workspace.selected_node, Some(expected_target));
        assert_eq!(
            workspace.hierarchy_source_browser.last_opened,
            Some(wire_selection.clone())
        );
        assert!(workspace.hierarchy_source_browser.status.contains("wire"));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &retained_component,
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());

        workspace.open_hierarchy_source(HierarchySourceSelection::Node {
            flattened: node_mapping.flattened_node(),
            origin: GraphHierarchyNodeOrigin::Root(GraphNodeId::new(99)),
        });
        assert_eq!(
            workspace.hierarchy_source_browser.last_opened,
            Some(wire_selection)
        );
        assert!(
            workspace
                .hierarchy_source_browser
                .status
                .contains("rejected without authoring mutation")
        );
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &retained_component,
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
    }

    #[test]
    fn flattened_source_browser_opens_private_definition_and_clears_stale_origin() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let authoritative = initial_component.encoding.digest();
        let wrapper = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .digest();
        workspace.selected_hierarchy_component = Some(wrapper);
        workspace.reconcile_component_definition_editor();
        workspace.component_definition.palette_index = 0;
        workspace.add_component_definition_node(wrapper);
        let private_scope = workspace.component_definition.scope.unwrap();
        let private_node = GraphNodeId::new(2);
        let mapping = workspace
            .component
            .as_ref()
            .unwrap()
            .hierarchy
            .flattening
            .node_provenance()
            .iter()
            .find(|mapping| {
                matches!(
                    mapping.origin(),
                    GraphHierarchyNodeOrigin::Component {
                        source_path,
                        component,
                        node,
                    } if source_path == &[GraphNodeId::new(1)]
                        && *component == private_scope
                        && *node == private_node
                )
            })
            .unwrap()
            .clone();
        let selection = HierarchySourceSelection::Node {
            flattened: mapping.flattened_node(),
            origin: mapping.origin().clone(),
        };
        workspace.mark_persisted();
        let retained_session = workspace.authoring_session_encoding().unwrap();
        let retained_component = workspace.component.as_ref().unwrap().clone();
        let retained_history = workspace.history.clone();

        workspace.selected_hierarchy_component = Some(authoritative);
        workspace.reconcile_hierarchy_selection(None);
        workspace.reconcile_component_definition_editor();
        workspace.component_definition.selected_node = None;
        workspace.open_hierarchy_source(selection.clone());

        assert_eq!(workspace.selected_hierarchy_component, Some(private_scope));
        assert_eq!(
            workspace.selected_hierarchy_instance,
            Some(GraphNodeId::new(1))
        );
        assert_eq!(workspace.component_definition.scope, Some(private_scope));
        assert_eq!(
            workspace.component_definition.selected_node,
            Some(private_node)
        );
        assert_eq!(
            workspace.hierarchy_source_browser.last_opened,
            Some(selection)
        );
        assert!(workspace.hierarchy_source_browser.scroll_pending);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &retained_component,
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());

        workspace.commit_component_definition_node_label(
            private_scope,
            private_node,
            "source_navigation_remap",
        );
        assert_eq!(workspace.hierarchy_source_browser.last_opened, None);
        assert!(!workspace.hierarchy_source_browser.scroll_pending);
        assert!(
            workspace
                .hierarchy_source_browser
                .status
                .contains("absent from the current exact hierarchy")
        );
    }

    #[test]
    fn canonical_component_panel_tracks_exact_edits_and_detaches_transactionally() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial = workspace.component.as_ref().unwrap();
        assert_eq!(initial.encoding.bytes().len(), 4_815);
        assert_eq!(
            initial.encoding.digest().0,
            [
                0x10, 0xe6, 0x49, 0x8e, 0xc3, 0x6a, 0xfc, 0x37, 0x7f, 0x13, 0x8c, 0xac, 0xb5, 0xc6,
                0xaf, 0xe2, 0x09, 0x1c, 0x40, 0x74, 0x9e, 0xa3, 0xc9, 0xe9, 0xd4, 0xbb, 0xa8, 0x92,
                0x5a, 0x4f, 0x02, 0x28,
            ]
        );
        assert!(initial.document.inputs().is_empty());
        assert_eq!(initial.document.outputs().len(), 7);
        assert_eq!(initial.document.panel_items().len(), 15);
        assert_eq!(
            initial.document.workspace_digest(),
            workspace.workspace_encoding.digest()
        );
        let replay = replay_graph_component(
            initial.encoding.bytes(),
            GraphComponentLimits::interactive(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.document(), &initial.document);
        assert_eq!(replay.encoding(), &initial.encoding);
        assert_recursive_hierarchy_package(initial);

        let initial_digest = initial.encoding.digest();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "201");
        let edited = workspace.component.as_ref().unwrap();
        assert_ne!(edited.encoding.digest(), initial_digest);
        assert_eq!(
            edited.document.workspace_digest(),
            workspace.workspace_encoding.digest()
        );

        workspace.delete_selected_node(GraphNodeId::new(18));
        assert!(workspace.component.is_none());
        assert!(workspace.component_status.contains("detached"));
        workspace.navigate_history(false);
        assert!(workspace.component.is_some());
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .document
                .workspace_digest(),
            workspace.workspace_encoding.digest()
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one panel lifecycle proves monotonic identity, recursive component replacement, complete-session isolation/history, workspace-edit retention, and persistence"
    )]
    fn front_panel_authoring_is_exact_historical_monotonic_and_persistent() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().encoding.clone();
        let initial_cached_job = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let initial_flattened = initial_component.hierarchy.flattening.encoding().clone();
        let removed_item = GraphFrontPanelItemId::new(15);
        let binding = GraphFrontPanelBinding::OutputIndicator(GraphComponentOutputId::new(7));

        workspace.apply_panel_action(PanelUiAction::Remove(removed_item));
        let removed_session = workspace.authoring_session_encoding().unwrap();
        let removed = workspace.component.as_ref().unwrap();
        assert_ne!(removed_session, initial_session);
        assert_eq!(removed.document.panel_items().len(), 14);
        assert_eq!(removed.document.next_panel_item_id(), 16);
        assert_eq!(removed.document.workspace(), &initial_workspace);
        assert_eq!(
            removed.hierarchy.document.root(),
            initial_component.hierarchy.document.root()
        );
        assert_eq!(removed.hierarchy.flattening.encoding(), &initial_flattened);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_job);

        workspace.apply_panel_action(PanelUiAction::Add {
            name: "external_permit_live".to_owned(),
            binding,
            rect: GraphFrontPanelRect::new(20, 480, 220, 54),
        });
        let added_session = workspace.authoring_session_encoding().unwrap();
        let added_item = GraphFrontPanelItemId::new(16);
        let added = workspace.component.as_ref().unwrap();
        assert_ne!(added_session, removed_session);
        assert_eq!(added.document.next_panel_item_id(), 17);
        assert_eq!(workspace.selected_panel_item, Some(added_item));
        assert_eq!(
            added.document.panel_item(added_item).unwrap().binding(),
            binding
        );
        assert_eq!(added.hierarchy.flattening.encoding(), &initial_flattened);

        let edited_draft = PanelItemDraft {
            name: "external_permit_scope".to_owned(),
            binding,
            x: 57,
            y: 511,
            width: 260,
            height: 64,
        };
        workspace.apply_panel_action(PanelUiAction::Update {
            item: added_item,
            draft: edited_draft.clone(),
        });
        let edited_session = workspace.authoring_session_encoding().unwrap();
        let edited = workspace.component.as_ref().unwrap();
        let item = edited.document.panel_item(added_item).unwrap();
        assert_eq!(item.name(), "external_permit_scope");
        assert_eq!(item.rect(), edited_draft.rect());
        assert_eq!(edited.document.next_panel_item_id(), 17);
        assert_eq!(edited.hierarchy.flattening.encoding(), &initial_flattened);

        let history_lengths = (workspace.history.undo_len(), workspace.history.redo_len());
        workspace.apply_panel_action(PanelUiAction::Update {
            item: added_item,
            draft: edited_draft.clone(),
        });
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            edited_session
        );
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            history_lengths
        );

        let retained_status = workspace.component_status.clone();
        let invalid = PanelItemDraft {
            binding: GraphFrontPanelBinding::OutputIndicator(GraphComponentOutputId::new(1)),
            ..edited_draft.clone()
        };
        workspace.apply_panel_action(PanelUiAction::Update {
            item: added_item,
            draft: invalid,
        });
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            edited_session
        );
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            history_lengths
        );
        assert_ne!(workspace.component_status, retained_status);
        assert!(
            workspace
                .component_status
                .contains("rejected without mutation")
        );

        workspace.apply_panel_action(PanelUiAction::Remove(added_item));
        let removed_again_session = workspace.authoring_session_encoding().unwrap();
        workspace.apply_panel_action(PanelUiAction::Add {
            name: "external_permit_final".to_owned(),
            binding,
            rect: GraphFrontPanelRect::new(64, 520, 260, 64),
        });
        let final_item = GraphFrontPanelItemId::new(17);
        let panel_session = workspace.authoring_session_encoding().unwrap();
        let panel_component = workspace.component.as_ref().unwrap();
        assert_eq!(panel_component.document.next_panel_item_id(), 18);
        assert!(panel_component.document.panel_item(added_item).is_none());
        assert_eq!(workspace.selected_panel_item, Some(final_item));

        workspace.navigate_history(false);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            removed_again_session
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            panel_session
        );
        assert_eq!(
            workspace.selected_panel_item,
            Some(GraphFrontPanelItemId::new(1))
        );

        let mut moved_workspace = workspace.workspace.clone();
        let placement = moved_workspace.placement(GraphNodeId::new(1)).unwrap();
        moved_workspace
            .move_node(
                placement.node(),
                placement.x().checked_add(1).unwrap(),
                placement.y(),
            )
            .unwrap();
        assert!(workspace.commit_candidate(
            moved_workspace,
            "moved control node while retaining authored front panel"
        ));
        let moved_session = workspace.authoring_session_encoding().unwrap();
        let moved_component = workspace.component.as_ref().unwrap();
        let retained_item = moved_component.document.panel_item(final_item).unwrap();
        assert_eq!(retained_item.name(), "external_permit_final");
        assert_eq!(
            retained_item.rect(),
            GraphFrontPanelRect::new(64, 520, 260, 64)
        );
        assert_eq!(moved_component.document.next_panel_item_id(), 18);

        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            moved_session
        );
        assert_eq!(
            restored
                .component
                .as_ref()
                .unwrap()
                .document
                .panel_item(final_item)
                .unwrap()
                .name(),
            "external_permit_final"
        );
        assert_eq!(
            (restored.history.undo_len(), restored.history.redo_len()),
            (0, 0)
        );
        assert!(!restored.persistence_pending());
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one connector lifecycle proves both directions, recursive stable-ID remapping, exact no-ops/rejections, history, and persistence"
    )]
    fn component_connector_authoring_is_recursive_monotonic_and_persistent() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().encoding.clone();
        let initial_cached_job = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let initial_root = initial_component.hierarchy.document.root().clone();
        let initial_wrapper = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .digest();
        let first_source =
            component_output_endpoint_choices(&initial_component.document, None)[0].0;

        workspace.apply_component_connector_action(ComponentConnectorUiAction::AddOutput {
            name: "setpoint_diagnostic".to_owned(),
            source: first_source,
        });
        let added_session = workspace.authoring_session_encoding().unwrap();
        let added_output = GraphComponentOutputId::new(8);
        let added = workspace.component.as_ref().unwrap();
        assert_ne!(added_session, initial_session);
        assert_eq!(added.document.next_output_id(), 9);
        assert_eq!(
            added.document.output(added_output).unwrap().source(),
            first_source
        );
        assert_eq!(
            workspace.selected_component_connector,
            Some(ComponentConnectorSelection::Output(added_output))
        );
        assert_eq!(added.hierarchy.document.root(), &initial_root);
        assert!(
            added
                .hierarchy
                .document
                .dependency(initial_component.encoding.digest())
                .is_none()
        );
        assert!(
            added
                .hierarchy
                .document
                .dependency(initial_wrapper)
                .is_none()
        );
        let refreshed_wrapper = added
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .digest();
        assert!(added.hierarchy.document.instances().iter().any(|instance| {
            *instance
                == GraphComponentInstance::nested(
                    refreshed_wrapper,
                    GraphNodeId::new(1),
                    added.encoding.digest(),
                )
        }));
        assert!(added.hierarchy.document.instances().iter().any(|instance| {
            *instance == GraphComponentInstance::root(GraphNodeId::new(1), refreshed_wrapper)
        }));
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_job);

        let second_source = component_output_endpoint_choices(&added.document, Some(added_output))
            .into_iter()
            .map(|(endpoint, _)| endpoint)
            .find(|endpoint| *endpoint != first_source)
            .unwrap();
        let updated_draft = ComponentConnectorDraft {
            name: "measurement_diagnostic".to_owned(),
            endpoint: second_source,
        };
        workspace.apply_component_connector_action(ComponentConnectorUiAction::UpdateOutput {
            output: added_output,
            draft: updated_draft.clone(),
        });
        let updated_session = workspace.authoring_session_encoding().unwrap();
        let updated = workspace.component.as_ref().unwrap();
        let output = updated.document.output(added_output).unwrap();
        assert_eq!(output.name(), "measurement_diagnostic");
        assert_eq!(output.source(), second_source);
        assert_eq!(updated.document.next_output_id(), 9);

        let history_lengths = (workspace.history.undo_len(), workspace.history.redo_len());
        workspace.apply_component_connector_action(ComponentConnectorUiAction::UpdateOutput {
            output: added_output,
            draft: updated_draft,
        });
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            updated_session
        );
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            history_lengths
        );

        let retained_history = workspace.history.clone();
        workspace.apply_component_connector_action(ComponentConnectorUiAction::RemoveOutput(
            GraphComponentOutputId::new(1),
        ));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            updated_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(
            workspace
                .component_status
                .contains("rejected without mutation")
        );

        workspace.apply_component_connector_action(ComponentConnectorUiAction::RemoveOutput(
            added_output,
        ));
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .document
                .next_output_id(),
            9
        );
        workspace.apply_component_connector_action(ComponentConnectorUiAction::AddOutput {
            name: "diagnostic_final".to_owned(),
            source: first_source,
        });
        let final_output = GraphComponentOutputId::new(9);
        assert!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .document
                .output(final_output)
                .is_some()
        );

        let mut disconnected = workspace.workspace.clone();
        let wire = disconnected.graph().wires()[0];
        let input_target = wire.target();
        disconnected.disconnect(wire.id()).unwrap();
        assert!(workspace.commit_candidate(
            disconnected,
            "disconnected one exact input for public connector authoring"
        ));
        assert!(
            component_input_endpoint_choices(
                &workspace.component.as_ref().unwrap().document,
                None,
            )
            .iter()
            .any(|(endpoint, _)| *endpoint == input_target)
        );
        workspace.apply_component_connector_action(ComponentConnectorUiAction::AddInput {
            name: "setpoint_command".to_owned(),
            target: input_target,
        });
        let first_input = GraphComponentInputId::new(1);
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .document
                .next_input_id(),
            2
        );
        workspace.apply_component_connector_action(ComponentConnectorUiAction::UpdateInput {
            input: first_input,
            draft: ComponentConnectorDraft {
                name: "setpoint_exact".to_owned(),
                endpoint: input_target,
            },
        });
        workspace
            .apply_component_connector_action(ComponentConnectorUiAction::RemoveInput(first_input));
        workspace.apply_component_connector_action(ComponentConnectorUiAction::AddInput {
            name: "setpoint_final".to_owned(),
            target: input_target,
        });
        let final_input = GraphComponentInputId::new(2);
        let final_session = workspace.authoring_session_encoding().unwrap();
        let final_component = workspace.component.as_ref().unwrap();
        assert_eq!(final_component.document.next_input_id(), 3);
        assert_eq!(
            final_component.document.input(final_input).unwrap().name(),
            "setpoint_final"
        );
        assert_eq!(final_component.document.next_output_id(), 10);
        assert!(final_component.document.output(final_output).is_some());

        workspace.navigate_history(false);
        assert_ne!(
            workspace.authoring_session_encoding().unwrap(),
            final_session
        );
        assert!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .document
                .input(final_input)
                .is_none()
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            final_session
        );

        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            final_session
        );
        let restored_component = &restored.component.as_ref().unwrap().document;
        assert_eq!(restored_component.next_input_id(), 3);
        assert_eq!(restored_component.next_output_id(), 10);
        assert_eq!(
            restored_component.input(final_input).unwrap().target(),
            input_target
        );
        assert_eq!(
            restored_component.output(final_output).unwrap().source(),
            first_source
        );
        assert!(!restored.persistence_pending());
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one selected-library lifecycle proves recursive root refresh, control-authority isolation, stable connector identity, exact history, and persistence together"
    )]
    fn selected_library_connector_authoring_preserves_control_authority() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().encoding.clone();
        let initial_cached_job = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let authoritative = initial_component.encoding.digest();
        let initial_root = initial_component.hierarchy.document.root().clone();
        let initial_root_instance = workspace.selected_hierarchy_instance;
        let initial_wrapper = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap();
        let initial_wrapper_digest = initial_wrapper.digest();
        let retained_source = initial_wrapper
            .document()
            .output(GraphComponentOutputId::new(7))
            .unwrap()
            .source();

        workspace.selected_hierarchy_component = Some(initial_wrapper_digest);
        workspace.reconcile_component_connector_selection(None);
        assert_eq!(
            workspace.component_connector_scope,
            Some(initial_wrapper_digest)
        );
        assert_eq!(
            workspace.selected_component_connector,
            Some(ComponentConnectorSelection::Output(
                GraphComponentOutputId::new(1)
            ))
        );

        workspace.apply_component_connector_action(ComponentConnectorUiAction::UpdateOutput {
            output: GraphComponentOutputId::new(1),
            draft: ComponentConnectorDraft {
                name: "wrapped_error_exact".to_owned(),
                endpoint: initial_wrapper
                    .document()
                    .output(GraphComponentOutputId::new(1))
                    .unwrap()
                    .source(),
            },
        });
        let renamed_session = workspace.authoring_session_encoding().unwrap();
        let renamed = workspace.component.as_ref().unwrap();
        assert_ne!(renamed_session, initial_session);
        assert_eq!(renamed.document, initial_component.document);
        assert_eq!(renamed.encoding, initial_component.encoding);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_job);
        assert_eq!(workspace.selected_hierarchy_instance, initial_root_instance);
        assert!(
            renamed
                .hierarchy
                .document
                .dependency(initial_wrapper_digest)
                .is_none()
        );
        let renamed_wrapper = renamed
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap();
        let renamed_wrapper_digest = renamed_wrapper.digest();
        assert_ne!(renamed_wrapper_digest, initial_wrapper_digest);
        assert_eq!(
            renamed_wrapper
                .document()
                .output(GraphComponentOutputId::new(1))
                .unwrap()
                .name(),
            "wrapped_error_exact"
        );
        assert_eq!(
            workspace.selected_hierarchy_component,
            Some(renamed_wrapper_digest)
        );
        assert_eq!(
            workspace.component_connector_scope,
            Some(renamed_wrapper_digest)
        );
        assert_eq!(
            renamed
                .hierarchy
                .document
                .instances()
                .iter()
                .find(|instance| instance.scope() == GraphInstanceScope::Root)
                .unwrap()
                .component(),
            renamed_wrapper_digest
        );
        let renamed_root = renamed.hierarchy.document.root();
        assert_ne!(renamed_root, &initial_root);
        assert_eq!(renamed_root.placements(), initial_root.placements());
        assert_eq!(renamed_root.next_node_id(), initial_root.next_node_id());
        assert_eq!(renamed_root.next_wire_id(), initial_root.next_wire_id());
        assert_eq!(renamed_root.graph().wires(), initial_root.graph().wires());
        assert_eq!(
            renamed_root
                .graph()
                .node(GraphNodeId::new(1))
                .unwrap()
                .outputs()[0]
                .name(),
            "wrapped_error_exact"
        );
        assert_eq!(
            workspace
                .authoring_session_document()
                .unwrap()
                .hierarchy()
                .unwrap()
                .selected_component(),
            authoritative
        );

        let history_lengths = (workspace.history.undo_len(), workspace.history.redo_len());
        workspace.apply_component_connector_action(ComponentConnectorUiAction::UpdateOutput {
            output: GraphComponentOutputId::new(1),
            draft: ComponentConnectorDraft {
                name: "wrapped_error_exact".to_owned(),
                endpoint: renamed_wrapper
                    .document()
                    .output(GraphComponentOutputId::new(1))
                    .unwrap()
                    .source(),
            },
        });
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            renamed_session
        );
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            history_lengths
        );

        workspace.apply_component_connector_action(ComponentConnectorUiAction::RemoveOutput(
            GraphComponentOutputId::new(7),
        ));
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .dependency(workspace.component_connector_scope.unwrap())
                .unwrap()
                .document()
                .next_output_id(),
            8
        );
        workspace.apply_component_connector_action(ComponentConnectorUiAction::AddOutput {
            name: "wrapped_permitted_output_final".to_owned(),
            source: retained_source,
        });
        let final_output = GraphComponentOutputId::new(8);
        let final_session = workspace.authoring_session_encoding().unwrap();
        let final_component = workspace.component.as_ref().unwrap();
        let final_wrapper_digest = workspace.component_connector_scope.unwrap();
        let final_wrapper = final_component
            .hierarchy
            .document
            .dependency(final_wrapper_digest)
            .unwrap()
            .document();
        assert_eq!(final_component.document, initial_component.document);
        assert_eq!(final_component.encoding, initial_component.encoding);
        assert_eq!(final_wrapper.next_output_id(), 9);
        assert!(
            final_wrapper
                .output(GraphComponentOutputId::new(7))
                .is_none()
        );
        assert_eq!(
            final_wrapper.output(final_output).unwrap().name(),
            "wrapped_permitted_output_final"
        );
        assert_eq!(
            workspace.selected_component_connector,
            Some(ComponentConnectorSelection::Output(final_output))
        );

        workspace.navigate_history(false);
        assert_ne!(
            workspace.authoring_session_encoding().unwrap(),
            final_session
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            final_session
        );

        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            final_session
        );
        let restored_component = restored.component.as_ref().unwrap();
        assert_eq!(restored_component.document, initial_component.document);
        assert_eq!(restored_component.encoding, initial_component.encoding);
        let restored_wrapper = restored_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .document();
        assert_eq!(restored_wrapper.next_output_id(), 9);
        assert!(
            restored_wrapper
                .output(GraphComponentOutputId::new(7))
                .is_none()
        );
        assert_eq!(
            restored_wrapper.output(final_output).unwrap().source(),
            retained_source
        );
        assert!(!restored.persistence_pending());
    }

    #[test]
    fn library_connector_edit_cannot_indirectly_rewrite_control_authority() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial = workspace.component.as_ref().unwrap().clone();
        let leaf = initial.encoding.digest();
        let wrapper = initial
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap();
        let wrapper_document = wrapper.document().clone();
        let wrapper_encoding = wrapper.encoding().clone();
        workspace.workspace = wrapper_document.workspace().clone();
        workspace.workspace_encoding = encode_graph_workspace(&workspace.workspace).unwrap();
        workspace.probes = Some(empty_probes(&workspace.workspace).unwrap());
        workspace.component = Some(ComponentPackage {
            document: wrapper_document,
            encoding: wrapper_encoding,
            hierarchy: initial.hierarchy,
        });
        workspace.selected_hierarchy_component = Some(leaf);
        workspace.component_connector_scope = Some(leaf);
        workspace.selected_panel_item = None;
        workspace.reconcile_component_connector_selection(None);

        let retained_session = workspace.authoring_session_encoding().unwrap();
        let retained_component = workspace.component.as_ref().unwrap().clone();
        let retained_history = workspace.history.clone();
        let source = retained_component
            .hierarchy
            .document
            .dependency(leaf)
            .unwrap()
            .document()
            .output(GraphComponentOutputId::new(1))
            .unwrap()
            .source();
        workspace.apply_component_connector_action(ComponentConnectorUiAction::UpdateOutput {
            output: GraphComponentOutputId::new(1),
            draft: ComponentConnectorDraft {
                name: "would_rewrite_selected_wrapper".to_owned(),
                endpoint: source,
            },
        });

        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &retained_component,
        );
        assert_eq!(workspace.history, retained_history);
        assert!(
            workspace
                .component_status
                .contains("would recursively rewrite the complete-session control ALGC")
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one definition lifecycle proves recursive identity refresh, exact control isolation, scoped selection, no-op behavior, history, and persistence together"
    )]
    fn selected_library_definition_canvas_preserves_control_authority() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_control_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().encoding.clone();
        let initial_cached_job = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let authoritative = initial_component.encoding.digest();
        let initial_root = initial_component.hierarchy.document.root().clone();
        let initial_source_map = initial_component.hierarchy.source_map.clone();
        let initial_wrapper = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap();
        let initial_wrapper_digest = initial_wrapper.digest();
        assert_eq!(initial_wrapper.document().workspace().next_node_id(), 2);

        workspace.selected_hierarchy_component = Some(initial_wrapper_digest);
        workspace.reconcile_component_definition_editor();
        assert_eq!(
            workspace.component_definition.scope,
            Some(initial_wrapper_digest)
        );
        workspace.component_definition.palette_index = 0;
        workspace.add_component_definition_node(initial_wrapper_digest);

        let added_session = workspace.authoring_session_encoding().unwrap();
        let added_scope = workspace.component_definition.scope.unwrap();
        let added_component = workspace.component.as_ref().unwrap();
        let added_wrapper = added_component
            .hierarchy
            .document
            .dependency(added_scope)
            .unwrap();
        let added_node = GraphNodeId::new(2);
        assert_ne!(added_session, initial_session);
        assert_ne!(added_scope, initial_wrapper_digest);
        assert_eq!(
            workspace.component_definition.selected_node,
            Some(added_node)
        );
        assert_eq!(added_wrapper.document().workspace().next_node_id(), 3);
        assert!(
            added_wrapper
                .document()
                .workspace()
                .graph()
                .node(added_node)
                .is_some()
        );
        assert_eq!(added_component.document, initial_component.document);
        assert_eq!(added_component.encoding, initial_component.encoding);
        assert_eq!(workspace.workspace, initial_control_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_job);
        assert_eq!(added_component.hierarchy.document.root(), &initial_root);
        assert_eq!(
            added_component.hierarchy.document.flattened_node_count(),
            initial_component.hierarchy.document.flattened_node_count() + 1
        );
        assert_ne!(added_component.hierarchy.source_map, initial_source_map);
        assert!(
            added_component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    *instance == GraphComponentInstance::root(GraphNodeId::new(1), added_scope)
                })
        );
        assert!(
            added_component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    *instance
                        == GraphComponentInstance::nested(
                            added_scope,
                            GraphNodeId::new(1),
                            authoritative,
                        )
                })
        );

        workspace.commit_component_definition_node_label(
            added_scope,
            added_node,
            "nested_setpoint_exact",
        );
        let renamed_session = workspace.authoring_session_encoding().unwrap();
        let renamed_scope = workspace.component_definition.scope.unwrap();
        assert_ne!(renamed_session, added_session);
        assert_ne!(renamed_scope, added_scope);
        assert_eq!(
            workspace.component_definition.selected_node,
            Some(added_node)
        );
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .dependency(renamed_scope)
                .unwrap()
                .document()
                .workspace()
                .graph()
                .node(added_node)
                .unwrap()
                .label(),
            "nested_setpoint_exact"
        );

        let no_op_history = (workspace.history.undo_len(), workspace.history.redo_len());
        workspace.commit_component_definition_node_label(
            renamed_scope,
            added_node,
            "nested_setpoint_exact",
        );
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            renamed_session
        );
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            no_op_history
        );

        let origin = workspace
            .component
            .as_ref()
            .unwrap()
            .hierarchy
            .document
            .dependency(renamed_scope)
            .unwrap()
            .document()
            .workspace()
            .placement(added_node)
            .unwrap();
        assert!(workspace.commit_component_definition_node_drag(
            renamed_scope,
            NodeDrag {
                node: added_node,
                origin,
                delta: egui::Vec2::ZERO,
            },
            egui::vec2(61.0, 37.0),
        ));
        let final_session = workspace.authoring_session_encoding().unwrap();
        let final_scope = workspace.component_definition.scope.unwrap();
        let final_component = workspace.component.as_ref().unwrap();
        let final_wrapper = final_component
            .hierarchy
            .document
            .dependency(final_scope)
            .unwrap();
        let final_placement = final_wrapper
            .document()
            .workspace()
            .placement(added_node)
            .unwrap();
        assert_eq!(final_placement.x(), origin.x() + 61);
        assert_eq!(final_placement.y(), origin.y() + 37);
        assert_eq!(final_component.document, initial_component.document);
        assert_eq!(final_component.encoding, initial_component.encoding);
        assert_eq!(workspace.workspace, initial_control_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_job);
        assert_eq!(final_component.hierarchy.document.root(), &initial_root);

        workspace.navigate_history(false);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            renamed_session
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            final_session
        );

        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            final_session
        );
        assert_eq!(restored.workspace, initial_control_workspace);
        assert_eq!(restored.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(restored.cached_jobs.encoding, initial_cached_job);
        let restored_component = restored.component.as_ref().unwrap();
        assert_eq!(restored_component.document, initial_component.document);
        assert_eq!(restored_component.encoding, initial_component.encoding);
        let restored_wrapper = restored_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .document();
        assert_eq!(
            restored_wrapper
                .workspace()
                .graph()
                .node(added_node)
                .unwrap()
                .label(),
            "nested_setpoint_exact"
        );
        assert_eq!(
            restored_wrapper.workspace().placement(added_node),
            Some(final_placement)
        );
        assert!(!restored.persistence_pending());
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one scoped child lifecycle proves atomic hierarchy binding, recursive identity refresh, monotonic allocation, exact history, and persistence together"
    )]
    fn selected_definition_child_occurrence_is_exact_historical_and_persistent() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_control_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().encoding.clone();
        let initial_cached_jobs = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let authoritative = initial_component.encoding.digest();
        let initial_root = initial_component.hierarchy.document.root().clone();
        let initial_root_digest = initial_component.hierarchy.document.root_digest();
        let initial_instances = initial_component
            .hierarchy
            .document
            .flattened_instance_count();
        let initial_nodes = initial_component.hierarchy.document.flattened_node_count();
        let child_nodes = initial_component
            .hierarchy
            .document
            .dependency(authoritative)
            .unwrap()
            .document()
            .workspace()
            .graph()
            .nodes()
            .len();
        let initial_wrapper = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .digest();

        workspace.selected_hierarchy_component = Some(initial_wrapper);
        workspace.reconcile_component_definition_editor();
        workspace.component_definition.child_component = Some(authoritative);
        workspace.add_component_definition_child(initial_wrapper, authoritative);

        let added_session = workspace.authoring_session_encoding().unwrap();
        let added_scope = workspace.component_definition.scope.unwrap();
        let added_component = workspace.component.as_ref().unwrap();
        let added_node = GraphNodeId::new(2);
        let added_wrapper = added_component
            .hierarchy
            .document
            .dependency(added_scope)
            .unwrap()
            .document();
        assert_ne!(added_session, initial_session);
        assert_ne!(added_scope, initial_wrapper);
        assert_eq!(
            workspace.component_definition.selected_node,
            Some(added_node)
        );
        assert_eq!(
            workspace.component_definition.child_component,
            Some(authoritative)
        );
        assert_eq!(added_wrapper.workspace().next_node_id(), 3);
        assert!(added_wrapper.workspace().graph().node(added_node).is_some());
        assert!(
            added_component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    *instance
                        == GraphComponentInstance::nested(added_scope, added_node, authoritative)
                })
        );
        assert!(
            added_component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    *instance == GraphComponentInstance::root(GraphNodeId::new(1), added_scope)
                })
        );
        assert_eq!(
            added_component
                .hierarchy
                .document
                .flattened_instance_count(),
            initial_instances + 1
        );
        assert_eq!(
            added_component.hierarchy.document.flattened_node_count(),
            initial_nodes + child_nodes
        );
        assert_eq!(added_component.document, initial_component.document);
        assert_eq!(added_component.encoding, initial_component.encoding);
        assert_eq!(added_component.hierarchy.document.root(), &initial_root);
        assert_eq!(
            added_component.hierarchy.document.root_digest(),
            initial_root_digest
        );
        assert_eq!(workspace.workspace, initial_control_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_jobs);

        workspace.remove_component_definition_child(added_scope, added_node);

        let removed_session = workspace.authoring_session_encoding().unwrap();
        let removed_scope = workspace.component_definition.scope.unwrap();
        let removed_component = workspace.component.as_ref().unwrap();
        let removed_wrapper = removed_component
            .hierarchy
            .document
            .dependency(removed_scope)
            .unwrap()
            .document();
        assert_ne!(removed_session, added_session);
        assert_ne!(removed_scope, added_scope);
        assert_eq!(workspace.component_definition.selected_node, None);
        assert_eq!(removed_wrapper.workspace().next_node_id(), 3);
        assert!(
            removed_wrapper
                .workspace()
                .graph()
                .node(added_node)
                .is_none()
        );
        assert!(
            !removed_component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Component(removed_scope)
                        && instance.node() == added_node
                })
        );
        assert_eq!(
            removed_component
                .hierarchy
                .document
                .flattened_instance_count(),
            initial_instances
        );
        assert_eq!(
            removed_component.hierarchy.document.flattened_node_count(),
            initial_nodes
        );
        assert_eq!(removed_component.document, initial_component.document);
        assert_eq!(removed_component.encoding, initial_component.encoding);
        assert_eq!(removed_component.hierarchy.document.root(), &initial_root);

        workspace.add_component_definition_child(removed_scope, authoritative);

        let final_session = workspace.authoring_session_encoding().unwrap();
        let final_scope = workspace.component_definition.scope.unwrap();
        let final_component = workspace.component.as_ref().unwrap().clone();
        let final_node = GraphNodeId::new(3);
        let final_wrapper = final_component
            .hierarchy
            .document
            .dependency(final_scope)
            .unwrap()
            .document();
        assert_ne!(final_session, removed_session);
        assert_eq!(
            workspace.component_definition.selected_node,
            Some(final_node)
        );
        assert_eq!(final_wrapper.workspace().next_node_id(), 4);
        assert!(final_wrapper.workspace().graph().node(added_node).is_none());
        assert!(final_wrapper.workspace().graph().node(final_node).is_some());
        assert!(
            final_component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    *instance
                        == GraphComponentInstance::nested(final_scope, final_node, authoritative)
                })
        );
        assert_eq!(final_component.document, initial_component.document);
        assert_eq!(final_component.encoding, initial_component.encoding);
        assert_eq!(final_component.hierarchy.document.root(), &initial_root);
        assert_eq!(workspace.workspace, initial_control_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_jobs);

        workspace.navigate_history(false);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            removed_session
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            final_session
        );

        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            final_session
        );
        assert_eq!(restored.workspace, initial_control_workspace);
        assert_eq!(restored.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(restored.cached_jobs.encoding, initial_cached_jobs);
        assert_exact_component_package_equal(
            restored.component.as_ref().unwrap(),
            &final_component,
        );
        let restored_wrapper = restored
            .component
            .as_ref()
            .unwrap()
            .hierarchy
            .document
            .dependency(final_scope)
            .unwrap()
            .document();
        assert_eq!(restored_wrapper.workspace().next_node_id(), 4);
        assert!(
            restored_wrapper
                .workspace()
                .graph()
                .node(final_node)
                .is_some()
        );
        assert!(
            restored
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    *instance
                        == GraphComponentInstance::nested(final_scope, final_node, authoritative)
                })
        );
        assert!(!restored.persistence_pending());
    }

    #[test]
    fn selected_definition_canvas_rejects_placeholder_and_control_authority_edits() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.mark_persisted();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let initial_history = workspace.history.clone();
        let authoritative = initial_component.encoding.digest();
        let wrapper = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .digest();

        workspace.selected_hierarchy_component = Some(wrapper);
        workspace.reconcile_component_definition_editor();
        workspace.delete_component_definition_node(wrapper, GraphNodeId::new(1));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            initial_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &initial_component,
        );
        assert_eq!(workspace.history, initial_history);
        assert!(!workspace.persistence_pending());
        assert!(
            workspace
                .component_definition
                .status
                .contains("ALGH owns component-instance placeholders")
        );

        workspace.selected_hierarchy_component = Some(authoritative);
        workspace.reconcile_component_definition_editor();
        workspace.add_component_definition_node(authoritative);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            initial_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &initial_component,
        );
        assert_eq!(workspace.history, initial_history);
        assert!(!workspace.persistence_pending());
        assert!(
            workspace
                .component_definition
                .status
                .contains("must be edited on the main canvas")
        );
    }

    #[test]
    fn selected_definition_child_occurrence_rejects_bindings_cycles_and_control_authority() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.mark_persisted();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let initial_history = workspace.history.clone();
        let authoritative = initial_component.encoding.digest();
        let wrapper = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .digest();

        workspace.selected_hierarchy_component = Some(wrapper);
        workspace.reconcile_component_definition_editor();
        workspace.remove_component_definition_child(wrapper, GraphNodeId::new(1));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            initial_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &initial_component,
        );
        assert_eq!(workspace.history, initial_history);
        assert!(!workspace.persistence_pending());
        assert!(
            workspace
                .component_definition
                .status
                .contains("rejected without mutation")
        );

        workspace.add_component_definition_child(wrapper, wrapper);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            initial_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &initial_component,
        );
        assert_eq!(workspace.history, initial_history);
        assert!(!workspace.persistence_pending());
        assert!(
            workspace
                .component_definition
                .status
                .contains("dependency cycle")
        );

        workspace.selected_hierarchy_component = Some(authoritative);
        workspace.reconcile_component_definition_editor();
        workspace.add_component_definition_child(authoritative, wrapper);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            initial_session
        );
        assert_exact_component_package_equal(
            workspace.component.as_ref().unwrap(),
            &initial_component,
        );
        assert_eq!(workspace.history, initial_history);
        assert!(!workspace.persistence_pending());
        assert!(
            workspace
                .component_definition
                .status
                .contains("must be edited on the main canvas")
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one typed-wire lifecycle proves duplicate rejection, disconnect, and monotonic reconnection under recursive definition replacement"
    )]
    fn selected_library_definition_typed_wires_retain_monotonic_identity() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_control = workspace.workspace.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let initial_root = initial_component.hierarchy.document.root().clone();
        let wrapper = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "control.reference_pid_wrapper")
            .unwrap()
            .digest();
        workspace.selected_hierarchy_component = Some(wrapper);
        workspace.reconcile_component_definition_editor();

        workspace.component_definition.palette_index = workspace
            .palette
            .iter()
            .position(|entry| entry.prototype.kind().name() == "control.source.value")
            .unwrap();
        workspace.add_component_definition_node(wrapper);
        let source_scope = workspace.component_definition.scope.unwrap();
        let source_node = GraphNodeId::new(2);
        workspace.component_definition.palette_index = workspace
            .palette
            .iter()
            .position(|entry| entry.prototype.kind().name() == "control.rate.value")
            .unwrap();
        workspace.add_component_definition_node(source_scope);
        let rate_scope = workspace.component_definition.scope.unwrap();
        let rate_node = GraphNodeId::new(3);
        let source = WireEndpoint {
            node: source_node,
            port: GraphPortId::new(1),
        };
        let target = WireEndpoint {
            node: rate_node,
            port: GraphPortId::new(1),
        };

        assert!(
            !workspace
                .handle_component_definition_port_edit(rate_scope, PortEdit::SelectOutput(source),)
        );
        assert!(
            workspace
                .handle_component_definition_port_edit(rate_scope, PortEdit::ConnectInput(target),)
        );
        let connected_scope = workspace.component_definition.scope.unwrap();
        let connected_session = workspace.authoring_session_encoding().unwrap();
        let connected_workspace = workspace
            .component
            .as_ref()
            .unwrap()
            .hierarchy
            .document
            .dependency(connected_scope)
            .unwrap()
            .document()
            .workspace();
        assert_eq!(connected_workspace.next_node_id(), 4);
        assert_eq!(connected_workspace.next_wire_id(), 2);
        assert_eq!(
            connected_workspace.graph().wires(),
            &[alumina_interface_core::graph::WireDefinition::new(
                GraphWireId::new(1),
                source,
                target,
            )]
        );

        let history = (workspace.history.undo_len(), workspace.history.redo_len());
        workspace.component_definition.pending_source = Some(source);
        assert!(!workspace.handle_component_definition_port_edit(
            connected_scope,
            PortEdit::ConnectInput(target),
        ));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            connected_session
        );
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            history
        );

        assert!(workspace.handle_component_definition_port_edit(
            connected_scope,
            PortEdit::DisconnectInput(target),
        ));
        let disconnected_scope = workspace.component_definition.scope.unwrap();
        let disconnected_workspace = workspace
            .component
            .as_ref()
            .unwrap()
            .hierarchy
            .document
            .dependency(disconnected_scope)
            .unwrap()
            .document()
            .workspace();
        assert!(disconnected_workspace.graph().wires().is_empty());
        assert_eq!(disconnected_workspace.next_wire_id(), 2);

        workspace.component_definition.pending_source = Some(source);
        assert!(workspace.handle_component_definition_port_edit(
            disconnected_scope,
            PortEdit::ConnectInput(target),
        ));
        let final_scope = workspace.component_definition.scope.unwrap();
        let final_component = workspace.component.as_ref().unwrap();
        let final_workspace = final_component
            .hierarchy
            .document
            .dependency(final_scope)
            .unwrap()
            .document()
            .workspace();
        assert_eq!(final_workspace.next_wire_id(), 3);
        assert_eq!(
            final_workspace.graph().wires(),
            &[alumina_interface_core::graph::WireDefinition::new(
                GraphWireId::new(2),
                source,
                target,
            )]
        );
        assert_eq!(workspace.workspace, initial_control);
        assert_eq!(final_component.document, initial_component.document);
        assert_eq!(final_component.encoding, initial_component.encoding);
        assert_eq!(final_component.hierarchy.document.root(), &initial_root);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one lifecycle test proves cross-branch isolation, exact history, monotonic identity, and persistence together"
    )]
    fn root_instance_authoring_is_exact_historical_monotonic_and_persistent() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().clone();
        let initial_cached_jobs = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let component = initial_component.encoding.digest();

        workspace.apply_hierarchy_action(HierarchyUiAction::AddRoot(component));

        let added_session = workspace.authoring_session_encoding().unwrap();
        let added_component = workspace.component.as_ref().unwrap();
        let added = GraphNodeId::new(2);
        assert_ne!(added_session, initial_session);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            initial_probes.encoding
        );
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_jobs);
        assert_eq!(added_component.document, initial_component.document);
        assert_eq!(added_component.encoding, initial_component.encoding);
        assert_ne!(
            added_component.hierarchy.encoding,
            initial_component.hierarchy.encoding
        );
        assert_ne!(
            added_component.hierarchy.source_map,
            initial_component.hierarchy.source_map
        );
        assert_eq!(added_component.hierarchy.document.root().next_node_id(), 3);
        assert_eq!(
            added_component
                .hierarchy
                .document
                .flattened_instance_count(),
            3
        );
        assert_eq!(
            added_component.hierarchy.document.flattened_node_count(),
            42
        );
        assert_eq!(
            added_component.hierarchy.document.flattened_wire_count(),
            50
        );
        assert!(
            added_component
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Root
                        && instance.node() == added
                        && instance.component() == component
                })
        );
        assert_eq!(workspace.selected_hierarchy_instance, Some(added));
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            (1, 0)
        );
        assert!(workspace.persistence_pending());

        workspace.navigate_history(false);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            initial_session
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            added_session
        );

        workspace.apply_hierarchy_action(HierarchyUiAction::RemoveRoot(added));
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .root()
                .next_node_id(),
            3
        );
        workspace.apply_hierarchy_action(HierarchyUiAction::AddRoot(component));
        let monotonic = workspace.component.as_ref().unwrap();
        assert!(
            monotonic
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Root
                        && instance.node() == GraphNodeId::new(3)
                        && instance.component() == component
                })
        );
        assert_eq!(monotonic.hierarchy.document.root().next_node_id(), 4);

        let final_session = workspace.authoring_session_encoding().unwrap();
        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            final_session
        );
        assert_eq!(
            (restored.history.undo_len(), restored.history.redo_len()),
            (0, 0)
        );
        assert!(!restored.persistence_pending());
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one root-canvas lifecycle proves exact placement, typed wiring, monotonic identity, history, branch isolation, and persistence"
    )]
    fn root_canvas_placement_and_typed_wiring_are_exact_historical_and_persistent() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_control_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().encoding.clone();
        let initial_cached_jobs = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let initial_root_digest = initial_component.hierarchy.document.root().graph_digest();
        let initial_revision = initial_component.hierarchy.document.revision();

        workspace.apply_hierarchy_action(HierarchyUiAction::MoveRoot {
            node: GraphNodeId::new(1),
            x: 173,
            y: -42,
        });
        let moved_session = workspace.authoring_session_encoding().unwrap();
        let moved = workspace.component.as_ref().unwrap();
        assert_ne!(moved_session, initial_session);
        assert_eq!(
            moved
                .hierarchy
                .document
                .root()
                .placement(GraphNodeId::new(1)),
            Some(GraphNodePlacement::new(GraphNodeId::new(1), 173, -42))
        );
        assert_eq!(
            moved.hierarchy.document.root().graph_digest(),
            initial_root_digest
        );
        assert_eq!(moved.hierarchy.document.revision(), initial_revision + 1);
        assert_eq!(workspace.workspace, initial_control_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_jobs);
        assert_eq!(moved.document, initial_component.document);
        assert_eq!(moved.encoding, initial_component.encoding);
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            (1, 0)
        );
        assert!(workspace.persistence_pending());

        workspace.mark_persisted();
        let moved_history = workspace.history.clone();
        workspace.apply_hierarchy_action(HierarchyUiAction::MoveRoot {
            node: GraphNodeId::new(1),
            x: 173,
            y: -42,
        });
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            moved_session
        );
        assert_eq!(workspace.history, moved_history);
        assert!(!workspace.persistence_pending());

        workspace.navigate_history(false);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            initial_session
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            moved_session
        );

        let (_, input_encoding) = root_input_component(&initial_component.document);
        let input_digest = input_encoding.digest();
        assert!(
            workspace
                .import_component_dependency(input_encoding.bytes())
                .unwrap()
        );
        workspace.apply_hierarchy_action(HierarchyUiAction::AddRoot(input_digest));
        let source = WireEndpoint {
            node: GraphNodeId::new(1),
            port: GraphPortId::new(1),
        };
        let target = WireEndpoint {
            node: GraphNodeId::new(2),
            port: GraphPortId::new(1),
        };
        workspace.pending_hierarchy_source = Some(source);
        workspace.apply_hierarchy_action(HierarchyUiAction::ConnectRoot { source, target });
        let connected_session = workspace.authoring_session_encoding().unwrap();
        let connected = workspace.component.as_ref().unwrap();
        assert_eq!(workspace.pending_hierarchy_source, None);
        assert_eq!(connected.hierarchy.document.root().next_wire_id(), 2);
        assert!(
            connected
                .hierarchy
                .document
                .root()
                .graph()
                .wires()
                .iter()
                .any(|wire| wire.id() == GraphWireId::new(1)
                    && wire.source() == source
                    && wire.target() == target)
        );
        assert_eq!(workspace.workspace, initial_control_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_jobs);
        assert_eq!(connected.document, initial_component.document);
        assert_eq!(connected.encoding, initial_component.encoding);
        assert_eq!(workspace.history.undo_len(), 4);

        workspace.navigate_history(false);
        assert!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .root()
                .graph()
                .wires()
                .is_empty()
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            connected_session
        );

        workspace.pending_hierarchy_source = Some(source);
        workspace.apply_hierarchy_action(HierarchyUiAction::DisconnectRoot(GraphWireId::new(1)));
        let disconnected = workspace.component.as_ref().unwrap();
        assert_eq!(workspace.pending_hierarchy_source, None);
        assert!(
            disconnected
                .hierarchy
                .document
                .root()
                .graph()
                .wires()
                .is_empty()
        );
        assert_eq!(disconnected.hierarchy.document.root().next_wire_id(), 2);

        workspace.apply_hierarchy_action(HierarchyUiAction::ConnectRoot { source, target });
        let final_session = workspace.authoring_session_encoding().unwrap();
        let final_component = workspace.component.as_ref().unwrap();
        assert!(
            final_component
                .hierarchy
                .document
                .root()
                .graph()
                .wires()
                .iter()
                .any(|wire| wire.id() == GraphWireId::new(2)
                    && wire.source() == source
                    && wire.target() == target)
        );
        assert_eq!(final_component.hierarchy.document.root().next_wire_id(), 3);

        workspace.mark_persisted();
        let retained_history = workspace.history.clone();
        workspace.pending_hierarchy_source = Some(source);
        workspace.apply_hierarchy_action(HierarchyUiAction::ConnectRoot { source, target });
        workspace.apply_hierarchy_action(HierarchyUiAction::DisconnectRoot(GraphWireId::new(99)));
        workspace.apply_hierarchy_action(HierarchyUiAction::MoveRoot {
            node: GraphNodeId::new(99),
            x: 0,
            y: 0,
        });
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            final_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert_eq!(workspace.pending_hierarchy_source, Some(source));

        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            final_session
        );
        assert_eq!(
            (restored.history.undo_len(), restored.history.redo_len()),
            (0, 0)
        );
        assert!(!restored.persistence_pending());
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the regression keeps the pre-edit hierarchy and every remapped binding visible in one exact preservation proof"
    )]
    fn control_edits_remap_selected_component_without_losing_root_authoring() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let original_component = workspace.component.as_ref().unwrap().encoding.digest();
        workspace.apply_hierarchy_action(HierarchyUiAction::AddRoot(original_component));
        let authored_session = workspace.authoring_session_encoding().unwrap();
        let authored_hierarchy = workspace
            .component
            .as_ref()
            .unwrap()
            .hierarchy
            .document
            .clone();
        let wrapper = authored_hierarchy
            .instances()
            .iter()
            .find(|instance| {
                instance.scope() == GraphInstanceScope::Root
                    && instance.node() == GraphNodeId::new(1)
            })
            .unwrap()
            .component();
        let wrapper_encoding = authored_hierarchy
            .dependency(wrapper)
            .unwrap()
            .encoding()
            .clone();

        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "23/11");

        let updated = workspace.component.as_ref().unwrap();
        let replacement = updated.encoding.digest();
        assert_ne!(replacement, original_component);
        assert_eq!(updated.hierarchy.document.root(), authored_hierarchy.root());
        assert_eq!(
            updated
                .hierarchy
                .document
                .dependency(wrapper)
                .unwrap()
                .encoding(),
            &wrapper_encoding
        );
        assert!(
            updated
                .hierarchy
                .document
                .dependency(original_component)
                .is_none()
        );
        assert!(
            updated
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Root
                        && instance.node() == GraphNodeId::new(2)
                        && instance.component() == replacement
                })
        );
        assert!(
            updated
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Component(wrapper)
                        && instance.component() == replacement
                })
        );
        assert_eq!(workspace.selected_hierarchy_component, Some(replacement));
        assert_eq!(workspace.history.undo_len(), 2);

        workspace.navigate_history(false);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            authored_session
        );
        assert!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Root
                        && instance.node() == GraphNodeId::new(2)
                        && instance.component() == original_component
                })
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .root(),
            authored_hierarchy.root()
        );
    }

    #[test]
    fn invalid_hierarchy_selections_fail_without_session_mutation() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.mark_persisted();
        let retained_session = workspace.authoring_session_encoding().unwrap();
        let retained_history = workspace.history.clone();

        workspace.apply_hierarchy_action(HierarchyUiAction::AddRoot(Digest([0xa7; 32])));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert!(
            workspace
                .component_status
                .contains("rejected without mutation")
        );

        workspace.apply_hierarchy_action(HierarchyUiAction::RemoveRoot(GraphNodeId::new(99)));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert!(
            workspace
                .component_status
                .contains("rejected without mutation")
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one empty-component lifecycle proves deterministic construction, exact no-op selection, subsequent definition authoring, history, persistence, and authority isolation"
    )]
    fn empty_library_component_creation_is_exact_editable_historical_and_persistent() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().encoding.clone();
        let initial_cached_jobs = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let initial_root = initial_component.hierarchy.document.root().clone();
        let initial_dependencies = initial_component
            .hierarchy
            .document
            .dependencies()
            .iter()
            .map(|dependency| dependency.encoding().clone())
            .collect::<Vec<_>>();

        workspace.mark_persisted();
        workspace.new_library_component_name = "user.logic_cell".to_owned();
        workspace.apply_hierarchy_action(HierarchyUiAction::CreateComponent);

        let created_session = workspace.authoring_session_encoding().unwrap();
        let created = workspace.component.as_ref().unwrap();
        let dependency = created
            .hierarchy
            .document
            .dependencies()
            .iter()
            .find(|dependency| dependency.document().name() == "user.logic_cell")
            .unwrap();
        let created_digest = dependency.digest();
        let document = dependency.document();
        assert_ne!(created_session, initial_session);
        assert_eq!(workspace.new_library_component_name, "");
        assert_eq!(document.revision(), 1);
        assert_eq!(document.component_version(), 1);
        assert_eq!(document.next_input_id(), 1);
        assert_eq!(document.next_output_id(), 1);
        assert_eq!(document.next_panel_item_id(), 1);
        assert!(document.inputs().is_empty());
        assert!(document.outputs().is_empty());
        assert!(document.panel_items().is_empty());
        assert_eq!(document.workspace().revision(), 1);
        assert_eq!(document.workspace().next_node_id(), 1);
        assert_eq!(document.workspace().next_wire_id(), 1);
        assert!(document.workspace().graph().nodes().is_empty());
        assert!(document.workspace().graph().wires().is_empty());
        assert_eq!(
            document.workspace().graph().schema(),
            initial_workspace.graph().schema()
        );
        assert_eq!(
            document.workspace().graph().clocks(),
            initial_workspace.graph().clocks()
        );
        assert_eq!(created.document, initial_component.document);
        assert_eq!(created.encoding, initial_component.encoding);
        assert_eq!(created.hierarchy.document.root(), &initial_root);
        for encoding in &initial_dependencies {
            assert!(
                created
                    .hierarchy
                    .document
                    .dependency(encoding.digest())
                    .is_some_and(|dependency| dependency.encoding() == encoding)
            );
        }
        assert_eq!(created.hierarchy.document.dependencies().len(), 3);
        assert_eq!(created.hierarchy.document.flattened_instance_count(), 2);
        assert_eq!(created.hierarchy.document.flattened_node_count(), 21);
        assert_eq!(created.hierarchy.document.flattened_wire_count(), 25);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_jobs);
        assert_eq!(workspace.selected_hierarchy_component, Some(created_digest));
        assert_eq!(workspace.component_definition.scope, Some(created_digest));
        assert_eq!(workspace.history.undo_len(), 1);
        assert!(workspace.persistence_pending());
        assert!(
            workspace
                .component_status
                .contains("created deterministic empty")
        );

        workspace.mark_persisted();
        let created_history = workspace.history.clone();
        workspace.new_library_component_name = "user.logic_cell".to_owned();
        workspace.apply_hierarchy_action(HierarchyUiAction::CreateComponent);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            created_session
        );
        assert_eq!(workspace.history, created_history);
        assert!(!workspace.persistence_pending());
        assert_eq!(workspace.new_library_component_name, "");
        assert_eq!(workspace.selected_hierarchy_component, Some(created_digest));
        assert!(workspace.component_status.contains("selected existing"));
        assert!(workspace.component_status.contains("already matched"));

        workspace.component_definition.palette_index = 0;
        workspace.add_component_definition_node(created_digest);
        let edited_session = workspace.authoring_session_encoding().unwrap();
        let edited_scope = workspace.component_definition.scope.unwrap();
        let edited = workspace
            .component
            .as_ref()
            .unwrap()
            .hierarchy
            .document
            .dependency(edited_scope)
            .unwrap()
            .document();
        assert_ne!(edited_scope, created_digest);
        assert_eq!(edited.name(), "user.logic_cell");
        assert_eq!(edited.workspace().graph().nodes().len(), 1);
        assert_eq!(edited.workspace().next_node_id(), 2);
        assert_eq!(workspace.history.undo_len(), 2);
        assert!(workspace.persistence_pending());
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_jobs);
        assert_eq!(
            workspace.component.as_ref().unwrap().document,
            initial_component.document
        );
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .root(),
            &initial_root
        );

        workspace.navigate_history(false);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            created_session
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            edited_session
        );
        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            edited_session
        );
        assert!(
            restored
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .dependencies()
                .iter()
                .any(|dependency| {
                    dependency.document().name() == "user.logic_cell"
                        && dependency.document().workspace().graph().nodes().len() == 1
                })
        );
    }

    #[test]
    fn invalid_or_ambiguous_empty_component_names_are_atomic() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.mark_persisted();
        let retained_session = workspace.authoring_session_encoding().unwrap();
        let retained_component = workspace.component.as_ref().unwrap().clone();
        let retained_history = workspace.history.clone();
        let invalid_names = [
            String::new(),
            "bad component name".to_owned(),
            "x".repeat(65),
            "control.reference_pid".to_owned(),
        ];

        for name in invalid_names {
            workspace.new_library_component_name.clone_from(&name);
            workspace.apply_hierarchy_action(HierarchyUiAction::CreateComponent);
            assert_eq!(
                workspace.authoring_session_encoding().unwrap(),
                retained_session
            );
            assert_exact_component_package_equal(
                workspace.component.as_ref().unwrap(),
                &retained_component,
            );
            assert_eq!(workspace.history, retained_history);
            assert!(!workspace.persistence_pending());
            assert_eq!(workspace.new_library_component_name, name);
            assert!(
                workspace
                    .component_status
                    .contains("rejected without mutation")
            );
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one exact library lifecycle proves import isolation, no-op behavior, in-use rejection, removal, history, and persistence"
    )]
    fn component_library_import_remove_history_and_persistence_are_exact() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_session = workspace.authoring_session_encoding().unwrap();
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().encoding.clone();
        let initial_cached_jobs = workspace.cached_jobs.encoding.clone();
        let initial_component = workspace.component.as_ref().unwrap().clone();
        let initial_root = initial_component.hierarchy.document.root().clone();
        let mut imported = initial_component.document.clone();
        let mut imported_workspace = imported.workspace().clone();
        let node = imported_workspace.graph().nodes()[0].id();
        let placement = imported_workspace.placement(node).unwrap();
        imported_workspace
            .move_node(node, placement.x() + 19, placement.y() + 23)
            .unwrap();
        imported.replace_workspace(imported_workspace).unwrap();
        let imported_encoding = encode_graph_component(&imported).unwrap();
        let imported_digest = imported_encoding.digest();

        workspace.mark_persisted();
        assert!(
            workspace
                .import_component_dependency(imported_encoding.bytes())
                .unwrap()
        );
        let imported_session = workspace.authoring_session_encoding().unwrap();
        let component = workspace.component.as_ref().unwrap();
        assert_ne!(imported_session, initial_session);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, initial_probes);
        assert_eq!(workspace.cached_jobs.encoding, initial_cached_jobs);
        assert_eq!(component.document, initial_component.document);
        assert_eq!(component.encoding, initial_component.encoding);
        assert_eq!(component.hierarchy.document.root(), &initial_root);
        assert_eq!(component.hierarchy.document.dependencies().len(), 3);
        assert_eq!(
            component
                .hierarchy
                .document
                .dependency(imported_digest)
                .unwrap()
                .encoding(),
            &imported_encoding
        );
        assert_eq!(component.hierarchy.document.flattened_instance_count(), 2);
        assert_eq!(component.hierarchy.document.flattened_node_count(), 21);
        assert_eq!(
            workspace.selected_hierarchy_component,
            Some(imported_digest)
        );
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            (1, 0)
        );
        assert!(workspace.persistence_pending());

        workspace.mark_persisted();
        let imported_history = workspace.history.clone();
        assert!(
            !workspace
                .import_component_dependency(imported_encoding.bytes())
                .unwrap()
        );
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            imported_session
        );
        assert_eq!(workspace.history, imported_history);
        assert!(!workspace.persistence_pending());
        assert!(workspace.component_status.contains("already matched"));

        workspace.apply_hierarchy_action(HierarchyUiAction::AddRoot(imported_digest));
        let referenced_session = workspace.authoring_session_encoding().unwrap();
        assert!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .instances()
                .iter()
                .any(|instance| {
                    instance.scope() == GraphInstanceScope::Root
                        && instance.node() == GraphNodeId::new(2)
                        && instance.component() == imported_digest
                })
        );
        assert_eq!(workspace.history.undo_len(), 2);

        workspace.mark_persisted();
        let referenced_history = workspace.history.clone();
        workspace.apply_hierarchy_action(HierarchyUiAction::RemoveComponent(imported_digest));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            referenced_session
        );
        assert_eq!(workspace.history, referenced_history);
        assert!(!workspace.persistence_pending());
        assert!(workspace.component_status.contains("still referenced"));

        workspace.apply_hierarchy_action(HierarchyUiAction::RemoveRoot(GraphNodeId::new(2)));
        workspace.apply_hierarchy_action(HierarchyUiAction::RemoveComponent(imported_digest));
        let removed_session = workspace.authoring_session_encoding().unwrap();
        let component = workspace.component.as_ref().unwrap();
        assert!(
            component
                .hierarchy
                .document
                .dependency(imported_digest)
                .is_none()
        );
        assert_eq!(component.hierarchy.document.root().next_node_id(), 3);
        assert_eq!(
            workspace.selected_hierarchy_component,
            Some(component.encoding.digest())
        );
        assert_eq!(workspace.history.undo_len(), 4);

        workspace.navigate_history(false);
        assert!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .hierarchy
                .document
                .dependency(imported_digest)
                .is_some()
        );
        workspace.navigate_history(false);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            referenced_session
        );
        workspace.navigate_history(true);
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            removed_session
        );

        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(
            restored.authoring_session_encoding().unwrap(),
            removed_session
        );
        assert_eq!(
            (restored.history.undo_len(), restored.history.redo_len()),
            (0, 0)
        );
    }

    #[test]
    fn invalid_or_authoritative_component_library_edits_are_atomic() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.mark_persisted();
        let retained_session = workspace.authoring_session_encoding().unwrap();
        let retained_history = workspace.history.clone();
        let authoritative = workspace.component.as_ref().unwrap().encoding.digest();

        let mut corrupt = workspace
            .component
            .as_ref()
            .unwrap()
            .encoding
            .bytes()
            .to_vec();
        corrupt[0] ^= 0xff;
        assert!(workspace.import_component_dependency(&corrupt).is_err());
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());

        let mut unreviewed = workspace.component.as_ref().unwrap().document.clone();
        let mut unreviewed_workspace = unreviewed.workspace().clone();
        let exemplar = unreviewed_workspace.graph().nodes()[0].clone();
        unreviewed_workspace
            .create_node(
                GraphNodePrototype::new(
                    NodeKind::new("control.unreviewed-component", 1),
                    "Unreviewed component behavior",
                    exemplar.domain(),
                    exemplar.inputs().to_vec(),
                    exemplar.outputs().to_vec(),
                    exemplar.parameters().to_vec(),
                ),
                9_000,
                100,
            )
            .unwrap();
        unreviewed.replace_workspace(unreviewed_workspace).unwrap();
        let unreviewed = encode_graph_component(&unreviewed).unwrap();
        assert!(
            workspace
                .import_component_dependency(unreviewed.bytes())
                .unwrap_err()
                .contains("semantics rejected")
        );
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());

        workspace.apply_hierarchy_action(HierarchyUiAction::RemoveComponent(authoritative));
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert!(workspace.component_status.contains("control authority"));
    }

    #[test]
    fn moves_and_port_edits_are_transactional_and_detach_bound_trace() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let graph_digest = workspace.workspace.graph_digest();
        let graph_revision = workspace.workspace.graph().revision();
        let origin = workspace.workspace.placement(GraphNodeId::new(1)).unwrap();
        workspace.commit_node_drag(
            NodeDrag {
                node: GraphNodeId::new(1),
                origin,
                delta: egui::Vec2::ZERO,
            },
            egui::vec2(17.4, -9.6),
        );
        assert_eq!(workspace.workspace.graph_digest(), graph_digest);
        assert_eq!(workspace.workspace.graph().revision(), graph_revision);
        assert_eq!(workspace.workspace.revision(), 2);
        assert_eq!(
            workspace.workspace.placement(GraphNodeId::new(1)),
            Some(GraphNodePlacement::new(
                GraphNodeId::new(1),
                origin.x() + 17,
                origin.y() - 10,
            ))
        );
        assert!(workspace.reference_trace_is_current());

        workspace.handle_port_edit(PortEdit::DisconnectInput(WireEndpoint {
            node: GraphNodeId::new(19),
            port: GraphPortId::new(1),
        }));
        assert_eq!(workspace.workspace.graph().wires().len(), 24);
        assert!(!workspace.reference_trace_is_current());
        assert!(workspace.edit_status.contains("draft semantic blocker"));

        workspace.reset_draft();
        let retained = workspace.workspace.clone();
        workspace.handle_port_edit(PortEdit::SelectOutput(WireEndpoint {
            node: GraphNodeId::new(6),
            port: GraphPortId::new(2),
        }));
        workspace.handle_port_edit(PortEdit::ConnectInput(WireEndpoint {
            node: GraphNodeId::new(19),
            port: GraphPortId::new(1),
        }));
        assert_eq!(workspace.workspace, retained);
        assert!(workspace.edit_status.contains("rejected without mutation"));
        assert_eq!(
            workspace.pending_source,
            Some(WireEndpoint {
                node: GraphNodeId::new(6),
                port: GraphPortId::new(2),
            })
        );
    }

    #[test]
    fn palette_node_lifecycle_and_exact_parameter_editing_are_transactional() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.palette_index = workspace
            .palette
            .iter()
            .position(|entry| entry.prototype.kind().name() == "control.exact.scale")
            .unwrap();
        workspace.add_palette_node();
        let created = GraphNodeId::new(22);
        assert_eq!(workspace.selected_node, Some(created));
        assert_eq!(workspace.workspace.graph().nodes().len(), 22);
        assert_eq!(workspace.workspace.next_node_id(), 23);
        assert_eq!(
            workspace
                .workspace
                .graph()
                .node(created)
                .unwrap()
                .kind()
                .name(),
            "control.exact.scale"
        );
        assert!(!workspace.reference_trace_is_current());
        assert!(workspace.edit_status.contains("draft semantic blocker"));

        workspace.delete_selected_node(created);
        assert_eq!(workspace.workspace.graph().nodes().len(), 21);
        assert_eq!(workspace.workspace.next_node_id(), 23);
        assert_eq!(workspace.selected_node, None);

        workspace.reset_draft();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "3/2");
        assert_eq!(
            workspace
                .workspace
                .graph()
                .node(GraphNodeId::new(8))
                .unwrap()
                .parameters()[0]
                .value()
                .value(),
            &GraphValue::ExactRational(Rational::fraction(3, 2).unwrap())
        );
        let canonical_three_halves = Rational::fraction(3, 2).unwrap().to_string();
        assert_eq!(
            workspace
                .parameter_drafts
                .get(&(GraphNodeId::new(8), 1))
                .map(String::as_str),
            Some(canonical_three_halves.as_str())
        );
        assert!(!workspace.reference_trace_is_current());

        let retained = workspace.workspace.clone();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "1/0");
        assert_eq!(workspace.workspace, retained);
        assert!(workspace.edit_status.contains("rejected without mutation"));
    }

    #[test]
    fn empty_draft_remains_renderable_and_can_accept_a_palette_node() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let mut candidate = workspace.workspace.clone();
        let ids: Vec<_> = candidate
            .graph()
            .nodes()
            .iter()
            .map(NodeDefinition::id)
            .collect();
        for id in ids {
            candidate.delete_node(id).unwrap();
        }
        assert!(workspace.commit_candidate(candidate, "deleted every draft node"));
        assert!(workspace.workspace.graph().nodes().is_empty());
        assert_eq!(workspace.presentation.nodes.len(), 0);
        assert!((workspace.presentation.size.x - EMPTY_CANVAS_WIDTH).abs() < f32::EPSILON);

        workspace.add_palette_node();
        assert_eq!(workspace.workspace.graph().nodes().len(), 1);
        assert_eq!(workspace.selected_node, Some(GraphNodeId::new(22)));
        assert_eq!(
            workspace.workspace.placement(GraphNodeId::new(22)),
            Some(GraphNodePlacement::new(
                GraphNodeId::new(22),
                NEW_NODE_ORIGIN,
                NEW_NODE_ORIGIN,
            ))
        );
    }

    #[test]
    fn canonical_history_drives_ui_undo_redo_and_clears_abandoned_redo() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().clone();
        let origin = workspace.workspace.placement(GraphNodeId::new(1)).unwrap();
        workspace.commit_node_drag(
            NodeDrag {
                node: GraphNodeId::new(1),
                origin,
                delta: egui::Vec2::ZERO,
            },
            egui::vec2(20.0, 30.0),
        );
        let moved = workspace.workspace.clone();
        let moved_probes = workspace.probes.as_ref().unwrap().clone();
        assert_ne!(moved_probes.encoding, initial_probes.encoding);
        assert_eq!(workspace.history.undo_len(), 1);
        assert_eq!(workspace.history.redo_len(), 0);

        workspace.navigate_history(false);
        assert_eq!(workspace.workspace, initial);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            initial_probes.encoding
        );
        assert_eq!(workspace.history.undo_len(), 0);
        assert_eq!(workspace.history.redo_len(), 1);
        workspace.navigate_history(true);
        assert_eq!(workspace.workspace, moved);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            moved_probes.encoding
        );

        workspace.navigate_history(false);
        let second_origin = workspace.workspace.placement(GraphNodeId::new(2)).unwrap();
        workspace.commit_node_drag(
            NodeDrag {
                node: GraphNodeId::new(2),
                origin: second_origin,
                delta: egui::Vec2::ZERO,
            },
            egui::vec2(-11.0, 7.0),
        );
        assert_eq!(workspace.history.redo_len(), 0);
        assert!(workspace.persistence_pending());
    }

    #[test]
    fn unified_authoring_history_interleaves_graph_probe_cached_job_and_hierarchy_exactly() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial = workspace.authoring_session_encoding().unwrap();

        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "17/9");
        let graph_edit = workspace.authoring_session_encoding().unwrap();
        assert_ne!(graph_edit, initial);
        assert!(workspace.component.is_some());

        workspace.set_probe_trigger(GraphProbeId::new(5), GraphProbeEdge::Rising);
        let probe_edit = workspace.authoring_session_encoding().unwrap();
        assert_ne!(probe_edit, graph_edit);

        workspace.cached_jobs.selected_entry = 1;
        workspace.apply_cached_job_action(CachedJobGraphAction::Rebind);
        let cached_job_edit = workspace.authoring_session_encoding().unwrap();
        assert_ne!(cached_job_edit, probe_edit);
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            (3, 0)
        );

        workspace.navigate_history(false);
        assert_eq!(workspace.authoring_session_encoding().unwrap(), probe_edit);
        workspace.navigate_history(false);
        assert_eq!(workspace.authoring_session_encoding().unwrap(), graph_edit);
        workspace.navigate_history(false);
        assert_eq!(workspace.authoring_session_encoding().unwrap(), initial);
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            (0, 3)
        );

        workspace.navigate_history(true);
        assert_eq!(workspace.authoring_session_encoding().unwrap(), graph_edit);
        workspace.navigate_history(true);
        assert_eq!(workspace.authoring_session_encoding().unwrap(), probe_edit);
        workspace.navigate_history(true);
        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            cached_job_edit
        );

        workspace.navigate_history(false);
        workspace.commit_node_label(GraphNodeId::new(8), "branched exact gain");
        assert_eq!(workspace.history.redo_len(), 0);
        assert_ne!(
            workspace.authoring_session_encoding().unwrap(),
            cached_job_edit
        );
    }

    #[test]
    fn cached_job_noop_does_not_dirty_or_extend_unified_history() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.mark_persisted();
        let retained_session = workspace.authoring_session_encoding().unwrap();
        let retained_history = workspace.history.clone();

        workspace.apply_cached_job_action(CachedJobGraphAction::Rebind);

        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert!(workspace.cached_jobs.status.contains("already matched"));
    }

    #[test]
    fn node_label_edits_are_bounded_session_historical_and_noop_aware() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let node = GraphNodeId::new(8);
        let initial = workspace.workspace.clone();
        let initial_encoding = workspace.workspace_encoding.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().clone();
        let initial_placement = workspace.workspace.placement(node);
        let initial_node_cursor = workspace.workspace.next_node_id();
        let initial_wire_cursor = workspace.workspace.next_wire_id();

        workspace.mark_persisted();
        workspace.commit_node_label(node, "Exact gain stage α");
        assert_eq!(
            workspace.workspace.graph().node(node).unwrap().label(),
            "Exact gain stage α"
        );
        assert_ne!(workspace.workspace_encoding, initial_encoding);
        assert_ne!(
            workspace.probes.as_ref().unwrap().encoding,
            initial_probes.encoding
        );
        assert_eq!(workspace.workspace.placement(node), initial_placement);
        assert_eq!(workspace.workspace.next_node_id(), initial_node_cursor);
        assert_eq!(workspace.workspace.next_wire_id(), initial_wire_cursor);
        assert_eq!(workspace.history.undo_len(), 1);
        assert_eq!(workspace.history.redo_len(), 0);
        assert!(workspace.persistence_pending());
        assert!(!workspace.reference_trace_is_current());
        assert_eq!(
            workspace.node_label_drafts.get(&node).map(String::as_str),
            Some("Exact gain stage α")
        );

        workspace.mark_persisted();
        let edited = workspace.workspace.clone();
        let edited_encoding = workspace.workspace_encoding.clone();
        let edited_probes = workspace.probes.as_ref().unwrap().clone();
        let retained_history = workspace.history.clone();
        workspace.commit_node_label(node, "Exact gain stage α");
        assert_eq!(workspace.workspace, edited);
        assert_eq!(workspace.workspace_encoding, edited_encoding);
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert!(workspace.edit_status.contains("already matched"));

        for invalid in [
            String::new(),
            "control\ncharacter".to_owned(),
            "x".repeat(
                workspace
                    .workspace
                    .graph()
                    .schema()
                    .limits()
                    .maximum_label_bytes
                    + 1,
            ),
        ] {
            workspace.commit_node_label(node, &invalid);
            assert_eq!(workspace.workspace, edited);
            assert_eq!(workspace.workspace_encoding, edited_encoding);
            assert_eq!(workspace.history, retained_history);
            assert!(!workspace.persistence_pending());
            assert!(workspace.edit_status.contains("rejected without mutation"));
        }

        workspace.navigate_history(false);
        assert_eq!(workspace.workspace, initial);
        assert_eq!(workspace.workspace_encoding, initial_encoding);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            initial_probes.encoding
        );
        assert!(workspace.node_label_drafts.is_empty());
        workspace.navigate_history(true);
        assert_eq!(workspace.workspace, edited);
        assert_eq!(workspace.workspace_encoding, edited_encoding);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            edited_probes.encoding
        );
        assert!(workspace.node_label_drafts.is_empty());

        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(restored.workspace, workspace.workspace);
        assert_eq!(restored.workspace_encoding, workspace.workspace_encoding);
        assert_eq!(
            restored.probes.as_ref().unwrap().encoding,
            workspace.probes.as_ref().unwrap().encoding
        );
    }

    #[test]
    fn execution_domain_choices_use_audited_families_and_known_device_identities() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let node = GraphNodeId::new(8);
        let schema = workspace
            .fixture
            .registry()
            .semantic_registry()
            .schema(workspace.workspace.graph().node(node).unwrap().kind())
            .unwrap();
        assert_eq!(
            audited_domain_choices(workspace.workspace.graph(), schema.allowed_domains()),
            vec![ExecutionDomain::HostExact]
        );

        workspace.mark_persisted();
        let retained_workspace = workspace.workspace.clone();
        let retained_encoding = workspace.workspace_encoding.clone();
        let retained_history = workspace.history.clone();
        workspace.commit_node_domain(node, ExecutionDomain::HostExact);
        assert_eq!(workspace.workspace, retained_workspace);
        assert_eq!(workspace.workspace_encoding, retained_encoding);
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert!(workspace.edit_status.contains("already matched"));

        workspace.commit_node_domain(
            node,
            ExecutionDomain::Service {
                device_id: DeviceId([0x42; 16]),
            },
        );
        assert_eq!(workspace.workspace, retained_workspace);
        assert_eq!(workspace.workspace_encoding, retained_encoding);
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert!(workspace.edit_status.contains("audited semantics rejected"));

        workspace.commit_node_domain(GraphNodeId::new(99), ExecutionDomain::HostExact);
        assert_eq!(workspace.workspace, retained_workspace);
        assert_eq!(workspace.history, retained_history);
        assert!(workspace.edit_status.contains("node 99 is unavailable"));

        let proof = tinybee_resource_proof().unwrap();
        let known_device = DeviceId([0x54; 16]);
        assert_eq!(
            audited_domain_choices(proof.workspace.graph(), ExecutionDomainSet::ALL),
            vec![
                ExecutionDomain::HostExact,
                ExecutionDomain::Service {
                    device_id: known_device,
                },
                ExecutionDomain::Realtime {
                    device_id: known_device,
                },
            ]
        );
        assert_eq!(
            audited_domain_choices(proof.workspace.graph(), ExecutionDomainSet::REALTIME),
            vec![ExecutionDomain::Realtime {
                device_id: known_device,
            }]
        );
        assert_eq!(
            domain_choice_label(ExecutionDomain::Realtime {
                device_id: known_device,
            }),
            "Realtime · device 5454545454545454…"
        );
    }

    #[test]
    fn persistence_round_trips_one_complete_exact_authoring_session() {
        let canonical = ExactControlWorkspace::try_new().unwrap();
        let canonical_encoding = canonical.authoring_session_encoding().unwrap();
        let canonical_persisted = canonical.persisted_authoring_session().unwrap();
        assert_eq!(
            decode_persisted_authoring_session(&canonical_persisted).unwrap(),
            canonical_encoding.bytes()
        );
        let canonical_restored =
            ExactControlWorkspace::try_new_with_persisted(Some(&canonical_persisted)).unwrap();
        assert_eq!(
            canonical_restored.probes.as_ref().unwrap().encoding,
            canonical.probes.as_ref().unwrap().encoding
        );
        assert_eq!(
            canonical_restored.cached_jobs.encoding,
            canonical.cached_jobs.encoding
        );
        let canonical_component = canonical.component.as_ref().unwrap();
        let restored_component = canonical_restored.component.as_ref().unwrap();
        assert_exact_component_package_equal(restored_component, canonical_component);

        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "7/3");
        workspace.set_probe_trigger(GraphProbeId::new(5), GraphProbeEdge::Rising);
        workspace.cached_jobs.selected_entry = 1;
        workspace.apply_cached_job_action(CachedJobGraphAction::Rebind);
        assert_eq!(workspace.history.undo_len(), 3);
        let persisted = workspace.persisted_authoring_session().unwrap();
        assert!(persisted.starts_with(PERSISTED_AUTHORING_SESSION_PREFIX));
        let payload = &persisted[PERSISTED_AUTHORING_SESSION_PREFIX.len()..];
        assert!(!payload.contains(':'));
        assert!(
            payload
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );

        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(restored.workspace, workspace.workspace);
        assert_eq!(restored.workspace_encoding, workspace.workspace_encoding);
        assert_eq!(
            restored.probes.as_ref().unwrap().document,
            workspace.probes.as_ref().unwrap().document
        );
        assert_eq!(
            restored.probes.as_ref().unwrap().encoding,
            workspace.probes.as_ref().unwrap().encoding
        );
        assert_eq!(
            restored.cached_jobs.workspace,
            workspace.cached_jobs.workspace
        );
        assert_eq!(
            restored.cached_jobs.encoding,
            workspace.cached_jobs.encoding
        );
        assert_eq!(restored.history.undo_len(), 0);
        assert_eq!(restored.history.redo_len(), 0);
        assert!(!restored.persistence_pending());
        let component = workspace.component.as_ref().unwrap();
        let restored_component = restored.component.as_ref().unwrap();
        assert_exact_component_package_equal(restored_component, component);

        let mut uppercase = persisted.clone();
        let offset = payload
            .bytes()
            .position(|byte| (b'a'..=b'f').contains(&byte))
            .unwrap();
        let persisted_offset = PERSISTED_AUTHORING_SESSION_PREFIX.len() + offset;
        uppercase.replace_range(
            persisted_offset..=persisted_offset,
            &payload[offset..=offset].to_ascii_uppercase(),
        );
        assert!(
            decode_persisted_authoring_session(&uppercase)
                .unwrap_err()
                .contains("lowercase hex")
        );
        assert!(
            decode_persisted_authoring_session("algwb1:0000:00:00")
                .unwrap_err()
                .contains("prefix/version is unsupported")
        );
    }

    #[test]
    fn absent_authoring_hierarchy_stays_absent_after_restore() {
        let mut detached = ExactControlWorkspace::try_new().unwrap();
        detached.component = None;
        let detached_persisted = detached.persisted_authoring_session().unwrap();
        let detached_restored =
            ExactControlWorkspace::try_new_with_persisted(Some(&detached_persisted)).unwrap();
        assert!(detached_restored.component.is_none());
        assert!(detached_restored.component_status.contains("no selected"));
    }

    #[test]
    fn hierarchy_identity_and_nested_artifact_corruption_reject_atomically() {
        let source = ExactControlWorkspace::try_new().unwrap();
        let canonical = source.authoring_session_encoding().unwrap();
        let offsets = algs_hierarchy_offsets(canonical.bytes());
        let mut target = ExactControlWorkspace::try_new().unwrap();
        target.commit_parameter_text(GraphNodeId::new(8), 1, "11/5");
        let retained = target.authoring_session_encoding().unwrap();

        let mut selected_substitution = canonical.bytes().to_vec();
        selected_substitution[offsets.selected_component.clone()].fill(0xa7);
        assert!(
            target
                .restore_authoring_session_bytes(&selected_substitution)
                .unwrap_err()
                .contains("selected component")
        );
        assert_eq!(
            target.authoring_session_encoding().unwrap().bytes(),
            retained.bytes()
        );

        let mut corrupt_hierarchy = canonical.bytes().to_vec();
        corrupt_hierarchy[offsets.hierarchy.start] ^= 0xff;
        assert!(
            target
                .restore_authoring_session_bytes(&corrupt_hierarchy)
                .unwrap_err()
                .contains("hierarchy")
        );
        assert_eq!(
            target.authoring_session_encoding().unwrap().bytes(),
            retained.bytes()
        );

        let mut corrupt_source_map = canonical.bytes().to_vec();
        corrupt_source_map[offsets.source_map.start] ^= 0xff;
        assert!(
            target
                .restore_authoring_session_bytes(&corrupt_source_map)
                .unwrap_err()
                .contains("source map")
        );
        assert_eq!(
            target.authoring_session_encoding().unwrap().bytes(),
            retained.bytes()
        );
    }

    #[test]
    fn history_target_failing_cached_job_catalog_admission_is_atomic() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "13/7");
        let retained_session = workspace.authoring_session_encoding().unwrap();

        let mut foreign_cached_job = workspace.cached_jobs.workspace.clone();
        let node = foreign_cached_job.graph().nodes()[0].id();
        let foreign_handle = JobGraphHandle {
            global_job_digest: Digest([0x6d; 32]),
            ..workspace.cached_jobs.catalog.entries()[0].handle()
        };
        let parameter = foreign_cached_job.graph().node(node).unwrap().parameters()[0]
            .value()
            .replacing_value_at_path(
                workspace.cached_jobs.registry.context_schema(),
                &CACHED_JOB_MIRROR_PATH,
                GraphValue::JobHandle(foreign_handle),
            )
            .unwrap();
        foreign_cached_job
            .set_parameter(node, CACHED_JOB_PARAMETER, parameter)
            .unwrap();
        let foreign_session = ExactControlWorkspace::authoring_session_document_from_parts(
            &workspace.workspace,
            &workspace.probes.as_ref().unwrap().document,
            &foreign_cached_job,
            workspace.component.as_ref(),
        )
        .unwrap();
        let foreign_encoding = encode_graph_authoring_session(&foreign_session).unwrap();
        workspace.history.record(foreign_encoding).unwrap();
        let retained_history = workspace.history.clone();

        workspace.navigate_history(false);

        assert_eq!(
            workspace.authoring_session_encoding().unwrap(),
            retained_session
        );
        assert_eq!(workspace.history, retained_history);
        assert!(workspace.edit_status.contains("UI/catalog admission"));
    }

    #[test]
    fn probe_edits_dirty_pair_persistence_only_when_canonical_identity_changes() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().clone();
        workspace.mark_persisted();
        assert!(!workspace.persistence_pending());

        workspace.clear_probe_trigger();
        assert!(workspace.persistence_pending());
        let cleared = workspace.probes.as_ref().unwrap().encoding.clone();
        assert_eq!(workspace.history.undo_len(), 1);
        workspace.navigate_history(false);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            initial_probes.encoding
        );
        workspace.navigate_history(true);
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, cleared);
        workspace.mark_persisted();
        let retained_history = workspace.history.clone();
        workspace.clear_probe_trigger();
        assert_eq!(workspace.probes.as_ref().unwrap().encoding, cleared);
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());

        workspace.set_probe_trigger(GraphProbeId::new(5), GraphProbeEdge::Rising);
        assert!(workspace.persistence_pending());
    }

    #[test]
    fn probe_metadata_edits_are_session_historical_bounded_and_noop_aware() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let id = GraphProbeId::new(2);
        let trigger_id = GraphProbeId::new(5);
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().clone();
        let initial_revision = initial_probes.document.revision();
        let draft = ProbeEditDraft {
            name: "integrator-state".to_owned(),
            maximum_samples: 2_048,
            sample_stride: 4,
        };

        workspace.apply_probe_metadata(id, &draft);
        let edited_probes = workspace.probes.as_ref().unwrap().clone();
        let edited = edited_probes.document.probe(id).unwrap();
        assert_eq!(edited.name(), draft.name);
        assert_eq!(
            edited.capture(),
            GraphProbeCapture::new(draft.maximum_samples, draft.sample_stride)
        );
        assert_eq!(edited_probes.document.revision(), initial_revision + 1);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(
            (workspace.history.undo_len(), workspace.history.redo_len()),
            (1, 0)
        );
        assert!(workspace.persistence_pending());

        workspace.navigate_history(false);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            initial_probes.encoding
        );
        assert_eq!(
            workspace.probe_drafts.get(&id),
            Some(&ProbeEditDraft::from_probe(
                initial_probes.document.probe(id).unwrap()
            ))
        );
        workspace.navigate_history(true);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            edited_probes.encoding
        );
        assert_eq!(workspace.probe_drafts.get(&id), Some(&draft));

        workspace.mark_persisted();
        let retained_history = workspace.history.clone();
        workspace.apply_probe_metadata(id, &draft);
        assert_eq!(workspace.history, retained_history);
        assert!(!workspace.persistence_pending());
        assert!(workspace.probe_status.contains("retained p2 metadata"));

        let retained_probes = workspace.probes.as_ref().unwrap().clone();
        let duplicate_name = ProbeEditDraft {
            name: retained_probes
                .document
                .probe(GraphProbeId::new(1))
                .unwrap()
                .name()
                .to_owned(),
            ..draft.clone()
        };
        workspace.apply_probe_metadata(id, &duplicate_name);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            retained_probes.encoding
        );
        assert_eq!(workspace.history, retained_history);
        assert!(workspace.probe_status.contains("rejected without mutation"));

        let trigger_probe = retained_probes.document.probe(trigger_id).unwrap();
        let undersized_trigger_capture = ProbeEditDraft {
            name: trigger_probe.name().to_owned(),
            maximum_samples: 4,
            sample_stride: trigger_probe.capture().sample_stride(),
        };
        workspace.apply_probe_metadata(trigger_id, &undersized_trigger_capture);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            retained_probes.encoding
        );
        assert_eq!(workspace.history, retained_history);
        assert!(workspace.probe_status.contains("trigger window"));
    }

    #[test]
    fn probe_drafts_survive_unrelated_trigger_edits_but_reset_on_history_navigation() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let id = GraphProbeId::new(2);
        let mut unsaved = workspace.probe_drafts.get(&id).unwrap().clone();
        unsaved.name = "unsaved-probe-name".to_owned();
        workspace.probe_drafts.insert(id, unsaved.clone());

        workspace.set_probe_trigger(GraphProbeId::new(5), GraphProbeEdge::Rising);
        assert_eq!(workspace.probe_drafts.get(&id), Some(&unsaved));

        workspace.navigate_history(false);
        assert_eq!(
            workspace.probe_drafts.get(&id),
            workspace
                .probes
                .as_ref()
                .unwrap()
                .document
                .probe(id)
                .map(ProbeEditDraft::from_probe)
                .as_ref()
        );
        assert_ne!(workspace.probe_drafts.get(&id), Some(&unsaved));
    }

    #[test]
    fn mixed_signal_plot_consumes_bounded_decimated_probe_projection() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.traces.clear();
        let initial = workspace.trace_projection().unwrap();
        assert_eq!(initial.exact.retained_samples(), 35);
        assert_eq!(initial.traces.len(), SIGNALS.len());
        assert!(initial.traces.iter().all(|series| series.points.len() == 5));
        assert_eq!(
            initial.source_clocks,
            BTreeSet::from([GraphClockId::new(3)])
        );
        let axis = initial.time_axis.as_ref().unwrap();
        assert_eq!(axis.minimum, Rational::from(10));
        assert_eq!(axis.maximum, Rational::from(50));

        let id = GraphProbeId::new(2);
        let mut draft = workspace.probe_drafts.get(&id).unwrap().clone();
        draft.maximum_samples = 2;
        draft.sample_stride = 2;
        workspace.apply_probe_metadata(id, &draft);
        let projected = workspace.trace_projection().unwrap();
        let integral = projected
            .traces
            .iter()
            .find(|series| {
                series.signal.representative == Some(RepresentativeControlSignal::IntegralPrior)
            })
            .unwrap();
        assert_eq!(
            integral
                .points
                .iter()
                .map(|point| point.tick)
                .collect::<Vec<_>>(),
            vec![2, 4]
        );
        assert_eq!(
            projected
                .exact
                .series()
                .iter()
                .find(|series| series.probe() == id)
                .unwrap()
                .samples()
                .len(),
            2
        );
        assert_eq!(projected.exact.retained_samples(), 32);

        workspace.clear_probe_trigger();
        let untriggered = workspace.trace_projection().unwrap();
        let integral = untriggered
            .traces
            .iter()
            .find(|series| {
                series.signal.representative == Some(RepresentativeControlSignal::IntegralPrior)
            })
            .unwrap();
        assert_eq!(integral.points.len(), 2);
        assert!(integral.points[0].tick < integral.points[1].tick);
    }

    #[test]
    fn mixed_rate_plot_includes_external_source_on_exact_shared_root_axis() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let source = WireEndpoint {
            node: GraphNodeId::new(1),
            port: GraphPortId::new(1),
        };
        workspace.add_probe(source);
        let projection = workspace.trace_projection().unwrap();
        assert_eq!(projection.exact.series().len(), 8);
        assert_eq!(projection.exact.retained_samples(), 56);
        assert_eq!(
            projection.source_clocks,
            BTreeSet::from([GraphClockId::new(2), GraphClockId::new(3)])
        );
        let source_series = projection
            .traces
            .iter()
            .find(|series| series.signal.source == source)
            .unwrap();
        let canonical_name = workspace
            .probes
            .as_ref()
            .unwrap()
            .document
            .probes()
            .iter()
            .find(|probe| probe.source() == source)
            .unwrap()
            .name();
        assert_eq!(source_series.signal.name, canonical_name);
        assert_eq!(source_series.signal.representative, None);
        assert_eq!(source_series.signal.sample_type_name, "exact.mm");
        assert_eq!(source_series.signal.unit_symbol.as_deref(), Some("mm"));
        assert_eq!(source_series.points.len(), 21);
        assert_eq!(
            (
                source_series.points[0].clock,
                source_series.points[0].tick,
                source_series.points[0].root_tick.clone(),
            ),
            (GraphClockId::new(2), 5, Rational::from(10))
        );
        assert_eq!(
            (
                source_series.points[20].clock,
                source_series.points[20].tick,
                source_series.points[20].root_tick.clone(),
            ),
            (GraphClockId::new(2), 25, Rational::from(50))
        );
        let axis = projection.time_axis.unwrap();
        assert_eq!(axis.minimum, Rational::from(10));
        assert_eq!(axis.maximum, Rational::from(50));
    }

    #[test]
    fn exact_root_time_axis_snaps_mixed_clocks_to_retained_rationals() {
        let one_third = Rational::from(1) / Rational::from(3);
        let one_half = Rational::from(1) / Rational::from(2);
        let two_thirds = Rational::from(2) / Rational::from(3);
        let traces = vec![
            TraceSeries {
                signal: TraceSignal {
                    probe: GraphProbeId::new(1),
                    name: "external-permit".to_owned(),
                    source: RepresentativeControlSignal::ExternalPermit.endpoint(),
                    representative: Some(RepresentativeControlSignal::ExternalPermit),
                    sample_type: GraphTypeId::new(1),
                    sample_type_name: "core.bool".to_owned(),
                    unit_symbol: None,
                },
                kind: TraceSeriesKind::Boolean,
                points: vec![
                    TracePoint {
                        clock: GraphClockId::new(2),
                        tick: 1,
                        sequence: 11,
                        root_tick: one_third.clone(),
                        value: TracePointValue::Boolean(false),
                    },
                    TracePoint {
                        clock: GraphClockId::new(2),
                        tick: 2,
                        sequence: 12,
                        root_tick: two_thirds.clone(),
                        value: TracePointValue::Boolean(true),
                    },
                ],
            },
            TraceSeries {
                signal: TraceSignal {
                    probe: GraphProbeId::new(2),
                    name: "combined-permit".to_owned(),
                    source: RepresentativeControlSignal::CombinedPermit.endpoint(),
                    representative: Some(RepresentativeControlSignal::CombinedPermit),
                    sample_type: GraphTypeId::new(1),
                    sample_type_name: "core.bool".to_owned(),
                    unit_symbol: None,
                },
                kind: TraceSeriesKind::Boolean,
                points: vec![TracePoint {
                    clock: GraphClockId::new(9),
                    tick: 7,
                    sequence: 13,
                    root_tick: one_half.clone(),
                    value: TracePointValue::Boolean(true),
                }],
            },
        ];
        let axis = TraceTimeAxis::try_new(&traces, Some((&one_third, &one_half, &two_thirds)))
            .unwrap()
            .unwrap();
        assert_eq!(axis.minimum, one_third);
        assert_eq!(axis.maximum, two_thirds);
        assert_eq!(
            axis.grid_ticks(),
            vec![
                Rational::from(1) / Rational::from(3),
                Rational::from(1) / Rational::from(2),
                Rational::from(2) / Rational::from(3),
            ]
        );

        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(100.0, 20.0));
        assert_eq!(axis.nearest_tick(rect, 60.0), Some(one_half.clone()));
        let held = trace_point_at_or_before(&traces[0], &one_half).unwrap();
        assert_eq!(held.clock, GraphClockId::new(2));
        assert_eq!(held.tick, 1);
        let clocks = traces
            .iter()
            .flat_map(|series| series.points.iter().map(|point| point.clock))
            .collect::<BTreeSet<_>>();
        assert_eq!(graph_clock_set_label(&clocks), "2, 9");
    }

    #[test]
    fn analog_trace_groups_overlay_only_identical_types_and_enforce_the_pane_bound() {
        let series = |probe: u32, sample_type: u32, name: &str, unit: Option<&str>| TraceSeries {
            signal: TraceSignal {
                probe: GraphProbeId::new(probe),
                name: format!("probe-{probe}"),
                source: RepresentativeControlSignal::Error.endpoint(),
                representative: None,
                sample_type: GraphTypeId::new(sample_type),
                sample_type_name: name.to_owned(),
                unit_symbol: unit.map(str::to_owned),
            },
            kind: TraceSeriesKind::Analog,
            points: Vec::new(),
        };
        let traces = vec![
            series(1, 3, "exact.percent", Some("%")),
            series(2, 2, "exact.mm", Some("mm")),
            series(3, 2, "exact.mm", Some("mm")),
        ];
        let groups = analog_trace_groups(&traces).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].sample_type, GraphTypeId::new(2));
        assert_eq!(groups[0].series.len(), 2);
        assert_eq!(groups[0].sample_type_name, "exact.mm");
        assert_eq!(groups[0].unit_symbol, "mm");
        assert_eq!(groups[0].label(), "mm\nexact.mm [t2]");
        assert_eq!(groups[1].sample_type, GraphTypeId::new(3));
        assert_eq!(groups[1].series.len(), 1);
        assert_eq!(groups[1].sample_type_name, "exact.percent");
        assert_eq!(groups[1].unit_symbol, "%");
        assert_eq!(groups[1].label(), "%\nexact.percent [t3]");

        let inconsistent = vec![
            series(1, 2, "exact.mm", Some("mm")),
            series(2, 2, "exact.mm", Some("%")),
        ];
        assert!(
            analog_trace_groups(&inconsistent)
                .unwrap_err()
                .contains("inconsistent canonical metadata")
        );
        assert!(
            analog_trace_groups(&[series(1, 2, "exact.mm", None)])
                .unwrap_err()
                .contains("no canonical unit")
        );

        let excessive = (1..=u32::try_from(MAXIMUM_ANALOG_TRACE_GROUPS).unwrap() + 1)
            .map(|sample_type| {
                series(
                    sample_type,
                    sample_type,
                    &format!("exact.type-{sample_type}"),
                    Some("u"),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            analog_trace_groups(&excessive)
                .unwrap_err()
                .contains("bounded 32-pane trace display policy")
        );
    }

    const PHYSICAL_EXACT: GraphTypeId = GraphTypeId::new(9);
    const PHYSICAL_INTERVAL: GraphTypeId = GraphTypeId::new(10);
    const PHYSICAL_SIGNED: GraphTypeId = GraphTypeId::new(11);
    const PHYSICAL_UNSIGNED: GraphTypeId = GraphTypeId::new(12);

    fn physical_trace_schema() -> GraphSchema {
        let unit = UnitId::new(1);
        GraphSchema::try_new(
            GraphLimits::interactive(),
            vec![UnitDefinition::new(
                unit,
                "mm",
                BaseDimensions::LENGTH,
                Rational::from(1),
            )],
            vec![
                TypeDefinition::new(
                    PHYSICAL_EXACT,
                    "physical.exact-mm",
                    TypeKind::ExactRational { unit },
                ),
                TypeDefinition::new(
                    PHYSICAL_INTERVAL,
                    "physical.interval-mm",
                    TypeKind::MeasurementInterval { unit },
                ),
                TypeDefinition::new(
                    PHYSICAL_SIGNED,
                    "physical.signed-mm",
                    TypeKind::CanonicalI64 {
                        unit,
                        quantum: Rational::from(1) / Rational::from(8),
                    },
                ),
                TypeDefinition::new(
                    PHYSICAL_UNSIGNED,
                    "physical.unsigned-mm",
                    TypeKind::CanonicalU64 {
                        unit,
                        quantum: Rational::from(1) / Rational::from(4),
                    },
                ),
            ],
        )
        .unwrap()
    }

    fn physical_trace_signal(sample_type: GraphTypeId, name: &str) -> TraceSignal {
        TraceSignal {
            probe: GraphProbeId::new(41),
            name: "physical-sample".to_owned(),
            source: RepresentativeControlSignal::Error.endpoint(),
            representative: None,
            sample_type,
            sample_type_name: name.to_owned(),
            unit_symbol: Some("mm".to_owned()),
        }
    }

    #[test]
    fn physical_scalar_trace_values_retain_exact_intervals_counts_and_units() {
        let schema = physical_trace_schema();
        let analog = |sample_type: GraphTypeId, value: GraphValue| {
            let definition = schema.value_type(sample_type).unwrap();
            let signal = physical_trace_signal(sample_type, definition.name());
            let value = TypedGraphValue::try_new(&schema, sample_type, value).unwrap();
            let (kind, point) = trace_sample_value(&signal, &schema, &value).unwrap();
            assert_eq!(kind, TraceSeriesKind::Analog);
            let TracePointValue::Analog { exact, enclosure } = point else {
                panic!("physical scalar did not produce an analog point");
            };
            (exact, enclosure)
        };

        let one_third = Rational::from(1) / Rational::from(3);
        let (exact, enclosure) =
            analog(PHYSICAL_EXACT, GraphValue::ExactRational(one_third.clone()));
        assert_eq!(exact, "1/3");
        assert_eq!(
            enclosure.map(f64::to_bits),
            one_third.to_f64_enclosure().unwrap().map(f64::to_bits)
        );

        let lower = -one_third;
        let upper = Rational::from(2) / Rational::from(3);
        let (exact, enclosure) = analog(
            PHYSICAL_INTERVAL,
            GraphValue::MeasurementInterval {
                lower: lower.clone(),
                upper: upper.clone(),
            },
        );
        assert_eq!(exact, "-1/3..2/3");
        assert_eq!(
            enclosure[0].to_bits(),
            lower.to_f64_enclosure().unwrap()[0].to_bits()
        );
        assert_eq!(
            enclosure[1].to_bits(),
            upper.to_f64_enclosure().unwrap()[1].to_bits()
        );

        let (exact, enclosure) = analog(PHYSICAL_SIGNED, GraphValue::CanonicalI64(-3));
        let signed_exact = -Rational::from(3) / Rational::from(8);
        assert_eq!(exact, "-3/8 [-3 lattice counts]");
        assert_eq!(
            enclosure.map(f64::to_bits),
            signed_exact.to_f64_enclosure().unwrap().map(f64::to_bits)
        );

        let (exact, enclosure) = analog(PHYSICAL_UNSIGNED, GraphValue::CanonicalU64(7));
        let unsigned_exact = Rational::from(7) / Rational::from(4);
        assert_eq!(exact, "1 3/4 [7 lattice counts]");
        assert_eq!(
            enclosure.map(f64::to_bits),
            unsigned_exact.to_f64_enclosure().unwrap().map(f64::to_bits)
        );

        let point = TracePoint {
            clock: GraphClockId::new(6),
            tick: 19,
            sequence: 23,
            root_tick: Rational::from(38),
            value: TracePointValue::Analog { exact, enclosure },
        };
        let signal = physical_trace_signal(PHYSICAL_UNSIGNED, "physical.unsigned-mm");
        assert_eq!(
            trace_cursor_label(&signal, &point),
            "physical-sample = 1 3/4 [7 lattice counts] mm @ c6:t19:s23"
        );
    }

    #[test]
    fn physical_scalar_trace_values_reject_type_substitution_and_reversal() {
        let schema = physical_trace_schema();
        let signal = physical_trace_signal(PHYSICAL_UNSIGNED, "physical.unsigned-mm");
        let signed =
            TypedGraphValue::try_new(&schema, PHYSICAL_SIGNED, GraphValue::CanonicalI64(7))
                .unwrap();
        assert!(
            trace_sample_value(&signal, &schema, &signed)
                .unwrap_err()
                .contains("changed its exact sample type from t12 to t11")
        );
        assert!(
            analog_trace_point_value(
                &signal,
                "2..1".to_owned(),
                &Rational::from(2),
                &Rational::from(1),
            )
            .unwrap_err()
            .contains("reversed exact analog interval")
        );
    }

    const STATE_BOOL: GraphTypeId = GraphTypeId::new(20);
    const STATE_TEXT: GraphTypeId = GraphTypeId::new(21);
    const STATE_BYTES: GraphTypeId = GraphTypeId::new(22);
    const STATE_ARRAY: GraphTypeId = GraphTypeId::new(23);
    const STATE_RECORD: GraphTypeId = GraphTypeId::new(24);
    const STATE_OPTION: GraphTypeId = GraphTypeId::new(25);
    const STATE_RESULT: GraphTypeId = GraphTypeId::new(26);
    const STATE_RESOURCE: GraphTypeId = GraphTypeId::new(27);
    const STATE_JOB: GraphTypeId = GraphTypeId::new(28);

    fn state_trace_schema() -> GraphSchema {
        GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![
                TypeDefinition::new(STATE_BOOL, "state.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    STATE_TEXT,
                    "state.text",
                    TypeKind::Text { maximum_bytes: 256 },
                ),
                TypeDefinition::new(
                    STATE_BYTES,
                    "state.bytes",
                    TypeKind::Bytes { maximum_bytes: 64 },
                ),
                TypeDefinition::new(
                    STATE_ARRAY,
                    "state.text-array",
                    TypeKind::Array {
                        element: STATE_TEXT,
                        maximum_items: 8,
                    },
                ),
                TypeDefinition::new(
                    STATE_RECORD,
                    "state.record",
                    TypeKind::Record {
                        fields: vec![
                            RecordField::new(RecordFieldId::new(1), "ready", STATE_BOOL),
                            RecordField::new(RecordFieldId::new(2), "message", STATE_TEXT),
                        ],
                    },
                ),
                TypeDefinition::new(
                    STATE_OPTION,
                    "state.optional-text",
                    TypeKind::Option { value: STATE_TEXT },
                ),
                TypeDefinition::new(
                    STATE_RESULT,
                    "state.text-or-bytes",
                    TypeKind::Result {
                        ok: STATE_TEXT,
                        error: STATE_BYTES,
                    },
                ),
                TypeDefinition::new(
                    STATE_RESOURCE,
                    "state.resource",
                    TypeKind::ResourceHandle {
                        class: ResourceClassId::new(7),
                    },
                ),
                TypeDefinition::new(STATE_JOB, "state.job", TypeKind::JobHandle),
            ],
        )
        .unwrap()
    }

    fn state_trace_signal(sample_type: GraphTypeId, name: &str, probe: u32) -> TraceSignal {
        TraceSignal {
            probe: GraphProbeId::new(probe),
            name: format!("state-{probe}"),
            source: RepresentativeControlSignal::Error.endpoint(),
            representative: None,
            sample_type,
            sample_type_name: name.to_owned(),
            unit_symbol: None,
        }
    }

    fn state_trace_value(
        schema: &GraphSchema,
        sample_type: GraphTypeId,
        value: GraphValue,
    ) -> TracePointValue {
        let definition = schema.value_type(sample_type).unwrap();
        let signal = state_trace_signal(sample_type, definition.name(), sample_type.get());
        let typed = TypedGraphValue::try_new(schema, sample_type, value).unwrap();
        let (kind, point) = trace_sample_value(&signal, schema, &typed).unwrap();
        assert_eq!(kind, TraceSeriesKind::State);
        point
    }

    #[test]
    fn composite_parameter_notation_is_bounded_and_handles_require_selectors() {
        let schema = state_trace_schema();
        let document = GraphDocument::try_new(
            1,
            schema,
            vec![ClockDefinition::new(
                GraphClockId::new(1),
                "host",
                ClockKind::HostMonotonic {
                    ticks_per_second: 1_000,
                },
            )],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let value = TypedGraphValue::try_new(
            document.schema(),
            STATE_RECORD,
            GraphValue::Record(vec![
                RecordValueField {
                    field: RecordFieldId::new(1),
                    value: GraphValue::Boolean(true),
                },
                RecordValueField {
                    field: RecordFieldId::new(2),
                    value: GraphValue::Text("ready\nμ".to_owned()),
                },
            ]),
        )
        .unwrap();
        assert_eq!(
            parameter_edit_text(&document, &value).as_deref(),
            Some("{ready:true,message:\"ready\\nμ\"}")
        );
        assert_eq!(
            parse_parameter_text(
                &document,
                STATE_RECORD,
                " { ready : true , message : \"ready\\nμ\" } ",
            )
            .unwrap(),
            value
        );

        let resource = TypedGraphValue::try_new(
            document.schema(),
            STATE_RESOURCE,
            GraphValue::ResourceHandle(ResourceGraphHandle {
                device_id: DeviceId([1; 16]),
                board_package_digest: Digest([2; 32]),
                class: ResourceClassId::new(7),
                resource_selector: 33,
            }),
        )
        .unwrap();
        assert_eq!(parameter_edit_text(&document, &resource), None);
        assert!(
            parse_parameter_text(&document, STATE_RESOURCE, "anything")
                .unwrap_err()
                .contains("requires an authenticated selector")
        );
    }

    #[test]
    fn non_scalar_trace_values_become_bounded_canonical_state_events() {
        let schema = state_trace_schema();
        let text = state_trace_value(
            &schema,
            STATE_TEXT,
            GraphValue::Text("ready\nμ-stage".to_owned()),
        );
        let TracePointValue::State { summary, encoding } = &text else {
            panic!("text did not produce a state point");
        };
        assert_eq!(summary, "text \"ready\\n\\u{3bc}-stage\" [14 UTF-8 bytes]");
        assert_eq!(
            encoding.digest(),
            alumina_storage::sha256(encoding.bytes()).digest
        );
        assert_eq!(&encoding.bytes()[..4], &STATE_TEXT.get().to_le_bytes());
        let mut exact_budget = MAXIMUM_STATE_IDENTITY_BYTES - encoding.bytes().len();
        retain_state_identity_bytes(&mut exact_budget, &text).unwrap();
        assert_eq!(exact_budget, MAXIMUM_STATE_IDENTITY_BYTES);
        assert!(
            retain_state_identity_bytes(&mut exact_budget, &text)
                .unwrap_err()
                .contains("canonical identity display policy")
        );
        let mut overflow = usize::MAX;
        assert_eq!(
            retain_state_identity_bytes(&mut overflow, &text),
            Err("state trace identity byte count overflowed".to_owned())
        );
        let long_text = state_trace_value(
            &schema,
            STATE_TEXT,
            GraphValue::Text("a".repeat(MAXIMUM_STATE_TEXT_PREVIEW_CHARS + 1)),
        );
        let TracePointValue::State { summary, .. } = long_text else {
            unreachable!();
        };
        assert_eq!(
            summary,
            format!(
                "text \"{}…\" [{} UTF-8 bytes]",
                "a".repeat(MAXIMUM_STATE_TEXT_PREVIEW_CHARS),
                MAXIMUM_STATE_TEXT_PREVIEW_CHARS + 1
            )
        );

        let cases = [
            (
                STATE_BYTES,
                GraphValue::Bytes(vec![0, 1, 2, 3, 4, 5, 6, 7, 8]),
                "bytes 0001020304050607… [9 bytes]",
            ),
            (
                STATE_ARRAY,
                GraphValue::Array(vec![
                    GraphValue::Text("a".to_owned()),
                    GraphValue::Text("b".to_owned()),
                ]),
                "array [2 items]",
            ),
            (
                STATE_RECORD,
                GraphValue::Record(vec![
                    RecordValueField {
                        field: RecordFieldId::new(1),
                        value: GraphValue::Boolean(true),
                    },
                    RecordValueField {
                        field: RecordFieldId::new(2),
                        value: GraphValue::Text("ready".to_owned()),
                    },
                ]),
                "record [2 fields]",
            ),
            (STATE_OPTION, GraphValue::OptionNone, "none"),
            (
                STATE_OPTION,
                GraphValue::OptionSome(Box::new(GraphValue::Text("ready".to_owned()))),
                "some(text)",
            ),
            (
                STATE_RESULT,
                GraphValue::ResultOk(Box::new(GraphValue::Text("ready".to_owned()))),
                "ok(text)",
            ),
            (
                STATE_RESULT,
                GraphValue::ResultError(Box::new(GraphValue::Bytes(vec![0xee]))),
                "error(bytes)",
            ),
        ];
        for (sample_type, value, expected) in cases {
            let point = state_trace_value(&schema, sample_type, value);
            let TracePointValue::State { summary, encoding } = point else {
                panic!("non-scalar value did not produce a state point");
            };
            assert_eq!(summary, expected);
            assert_eq!(&encoding.bytes()[..4], &sample_type.get().to_le_bytes());
        }
    }

    #[test]
    fn identity_bearing_trace_values_become_canonical_state_events() {
        let schema = state_trace_schema();
        let resource = state_trace_value(
            &schema,
            STATE_RESOURCE,
            GraphValue::ResourceHandle(ResourceGraphHandle {
                device_id: DeviceId([1; 16]),
                board_package_digest: Digest([2; 32]),
                class: ResourceClassId::new(7),
                resource_selector: 33,
            }),
        );
        let job = state_trace_value(
            &schema,
            STATE_JOB,
            GraphValue::JobHandle(JobGraphHandle {
                device_id: DeviceId([3; 16]),
                global_job_digest: Digest([4; 32]),
                partition_digest: Digest([5; 32]),
            }),
        );
        assert!(matches!(resource, TracePointValue::State { .. }));
        assert!(matches!(job, TracePointValue::State { .. }));
    }

    #[test]
    fn state_lanes_mark_byte_exact_changes_and_retain_exact_cursor_identity() {
        let schema = state_trace_schema();
        let definition = schema.value_type(STATE_TEXT).unwrap();
        let signal = state_trace_signal(STATE_TEXT, definition.name(), 9);
        assert_eq!(state_signal_label(&signal), "state-9\nstate.text [t21]");
        let ready = state_trace_value(&schema, STATE_TEXT, GraphValue::Text("ready".to_owned()));
        let waiting =
            state_trace_value(&schema, STATE_TEXT, GraphValue::Text("waiting".to_owned()));
        let series = TraceSeries {
            signal: signal.clone(),
            kind: TraceSeriesKind::State,
            points: vec![
                TracePoint {
                    clock: GraphClockId::new(3),
                    tick: 4,
                    sequence: 40,
                    root_tick: Rational::from(8),
                    value: ready.clone(),
                },
                TracePoint {
                    clock: GraphClockId::new(3),
                    tick: 5,
                    sequence: 50,
                    root_tick: Rational::from(10),
                    value: ready.clone(),
                },
                TracePoint {
                    clock: GraphClockId::new(3),
                    tick: 5,
                    sequence: 51,
                    root_tick: Rational::from(10),
                    value: ready,
                },
                TracePoint {
                    clock: GraphClockId::new(3),
                    tick: 6,
                    sequence: 60,
                    root_tick: Rational::from(12),
                    value: waiting,
                },
            ],
        };
        assert!(state_point_changed(&series, 0));
        assert!(!state_point_changed(&series, 1));
        assert!(!state_point_changed(&series, 2));
        assert!(state_point_changed(&series, 3));
        assert_eq!(state_same_time_run_end(&series, 0), 1);
        assert_eq!(state_same_time_run_end(&series, 1), 3);
        assert_eq!(state_same_time_run_end(&series, 2), 3);
        assert_eq!(state_same_time_run_end(&series, 3), 4);
        assert_eq!(state_same_time_run_end(&series, 4), 4);
        assert_eq!(
            trace_point_at_or_before(&series, &Rational::from(10))
                .unwrap()
                .sequence,
            51
        );
        let label = trace_cursor_label(&signal, &series.points[3]);
        let TracePointValue::State { encoding, .. } = &series.points[3].value else {
            unreachable!();
        };
        assert!(label.contains("text \"waiting\" [7 UTF-8 bytes]"));
        assert!(label.contains("state.text [t21]"));
        assert!(label.contains(&digest_hex(encoding.digest())));
        assert!(label.contains("canonical bytes @ c3:t6:s60"));

        let axis = TraceTimeAxis::try_new(core::slice::from_ref(&series), None)
            .unwrap()
            .unwrap();
        let context = egui::Context::default();
        let painted = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(480.0, 90.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let (_, painter) = ui.allocate_painter(
                        egui::vec2(400.0, STATE_TRACE_LANE_HEIGHT),
                        egui::Sense::hover(),
                    );
                    paint_state_trace_series(&painter, painter.clip_rect(), &[&series], &axis);
                });
            },
        );
        assert!(!painted.shapes.is_empty());
    }

    #[test]
    fn state_lane_order_and_count_are_bounded() {
        let schema = state_trace_schema();
        let definition = schema.value_type(STATE_TEXT).unwrap();
        let mut unordered = vec![
            TraceSeries {
                signal: state_trace_signal(STATE_TEXT, definition.name(), 12),
                kind: TraceSeriesKind::State,
                points: Vec::new(),
            },
            TraceSeries {
                signal: state_trace_signal(STATE_TEXT, definition.name(), 2),
                kind: TraceSeriesKind::State,
                points: Vec::new(),
            },
        ];
        let lanes = state_trace_lanes(&unordered).unwrap();
        assert_eq!(lanes[0].signal.probe, GraphProbeId::new(2));
        assert_eq!(lanes[1].signal.probe, GraphProbeId::new(12));

        unordered = (1..=u32::try_from(MAXIMUM_STATE_TRACE_LANES).unwrap() + 1)
            .map(|probe| TraceSeries {
                signal: state_trace_signal(STATE_TEXT, definition.name(), probe),
                kind: TraceSeriesKind::State,
                points: Vec::new(),
            })
            .collect();
        assert!(
            state_trace_lanes(&unordered)
                .unwrap_err()
                .contains("bounded 64-lane trace display policy")
        );
    }

    #[test]
    fn graph_edits_cannot_leave_an_unbound_probe_sidecar() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial_workspace = workspace.workspace.clone();
        let initial_probes = workspace.probes.as_ref().unwrap().clone();
        let observed_node = workspace.probes.as_ref().unwrap().document.probes()[0]
            .source()
            .node;
        workspace.delete_selected_node(observed_node);

        let probes = workspace.probes.as_ref().unwrap();
        assert!(probes.document.probes().is_empty());
        assert_eq!(
            probes.document.workspace_digest(),
            workspace.workspace_encoding.digest()
        );
        assert!(workspace.probe_status.contains("empty canonical sidecar"));
        let edited_workspace = workspace.workspace.clone();
        let edited_probes = probes.clone();
        workspace.navigate_history(false);
        assert_eq!(workspace.workspace, initial_workspace);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            initial_probes.encoding
        );
        workspace.navigate_history(true);
        assert_eq!(workspace.workspace, edited_workspace);
        assert_eq!(
            workspace.probes.as_ref().unwrap().encoding,
            edited_probes.encoding
        );
        let persisted = workspace.persisted_authoring_session().unwrap();
        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(restored.workspace_encoding, workspace.workspace_encoding);
        assert_eq!(
            restored.probes.as_ref().unwrap().encoding,
            edited_probes.encoding
        );
    }

    #[test]
    fn algp_import_is_session_historical_and_wrong_binding_fails_closed() {
        let reference = ExactControlWorkspace::try_new().unwrap();
        let mut source = ExactControlWorkspace::try_new().unwrap();
        source.commit_parameter_text(GraphNodeId::new(8), 1, "9/4");
        source.set_probe_trigger(GraphProbeId::new(5), GraphProbeEdge::Rising);
        let imported_bytes = source.workspace_encoding.bytes().to_vec();
        let mut target = ExactControlWorkspace::try_new().unwrap();
        target.import_workspace_bytes(&imported_bytes).unwrap();
        assert_eq!(target.workspace, source.workspace);
        assert_eq!(target.history.undo_len(), 1);

        let rebound_probe = target.probes.as_ref().unwrap().clone();
        let imported_probe_bytes = source.probes.as_ref().unwrap().encoding.bytes().to_vec();
        assert!(target.import_probe_bytes(&imported_probe_bytes).unwrap());
        assert_eq!(target.history.undo_len(), 2);
        assert_eq!(
            target.probes.as_ref().unwrap().encoding,
            source.probes.as_ref().unwrap().encoding
        );
        target.navigate_history(false);
        assert_eq!(target.workspace, source.workspace);
        assert_eq!(
            target.probes.as_ref().unwrap().encoding,
            rebound_probe.encoding
        );
        target.navigate_history(true);
        assert_eq!(target.workspace, source.workspace);
        assert_eq!(
            target.probes.as_ref().unwrap().encoding,
            source.probes.as_ref().unwrap().encoding
        );
        let retained_probe = target.probes.as_ref().unwrap().clone();
        let retained_history = target.history.clone();
        let wrong_workspace_probe = reference.probes.as_ref().unwrap().encoding.bytes().to_vec();
        assert!(target.import_probe_bytes(&wrong_workspace_probe).is_err());
        assert_eq!(
            target.probes.as_ref().unwrap().encoding,
            retained_probe.encoding
        );
        assert_eq!(target.workspace, source.workspace);
        assert_eq!(target.history, retained_history);
    }

    #[test]
    fn invalid_persistence_and_workspace_imports_fail_closed_without_losing_the_draft() {
        let fallback = ExactControlWorkspace::try_new_with_persisted(Some("wrong:00")).unwrap();
        assert!(
            fallback
                .edit_status
                .contains("persisted ALGS authoring session rejected")
        );
        assert!(fallback.persistence_pending());
        assert!(fallback.reference_trace_is_current());

        let reference = ExactControlWorkspace::try_new().unwrap();
        let mut source = ExactControlWorkspace::try_new().unwrap();
        source.commit_parameter_text(GraphNodeId::new(8), 1, "9/4");
        let source_session = source.authoring_session_encoding().unwrap();
        let mut mismatched_pair = source_session.bytes().to_vec();
        replace_exact_embedded_bytes(
            &mut mismatched_pair,
            source.probes.as_ref().unwrap().encoding.bytes(),
            reference.probes.as_ref().unwrap().encoding.bytes(),
        );
        let mismatched_pair = persisted_algs(&mismatched_pair);
        let pair_fallback =
            ExactControlWorkspace::try_new_with_persisted(Some(&mismatched_pair)).unwrap();
        assert_eq!(pair_fallback.workspace, reference.workspace);
        assert_eq!(
            pair_fallback.probes.as_ref().unwrap().encoding,
            reference.probes.as_ref().unwrap().encoding
        );
        assert!(pair_fallback.edit_status.contains("rejected atomically"));

        let imported_bytes = source.workspace_encoding.bytes().to_vec();
        let mut target = ExactControlWorkspace::try_new().unwrap();
        target.import_workspace_bytes(&imported_bytes).unwrap();
        assert_eq!(target.workspace, source.workspace);
        assert_eq!(target.history.undo_len(), 1);

        let retained_workspace = target.workspace.clone();
        let retained_history = target.history.clone();
        let mut corrupt = imported_bytes;
        corrupt[0] ^= 0xff;
        assert!(target.import_workspace_bytes(&corrupt).is_err());
        assert_eq!(target.workspace, retained_workspace);
        assert_eq!(target.history, retained_history);

        let exemplar = target
            .workspace
            .graph()
            .node(GraphNodeId::new(8))
            .unwrap()
            .clone();
        let mut unknown = target.workspace.clone();
        let required_wire = unknown
            .graph()
            .wires()
            .iter()
            .find(|wire| {
                wire.target()
                    == WireEndpoint {
                        node: GraphNodeId::new(19),
                        port: GraphPortId::new(1),
                    }
            })
            .unwrap()
            .id();
        unknown.disconnect(required_wire).unwrap();
        unknown
            .create_node(
                GraphNodePrototype::new(
                    NodeKind::new("control.unreviewed", 1),
                    "Unreviewed",
                    exemplar.domain(),
                    exemplar.inputs().to_vec(),
                    exemplar.outputs().to_vec(),
                    exemplar.parameters().to_vec(),
                ),
                9_000,
                100,
            )
            .unwrap();
        let unknown = encode_graph_workspace(&unknown).unwrap();
        assert!(
            target
                .import_workspace_bytes(unknown.bytes())
                .unwrap_err()
                .contains("audited semantics rejected")
        );
        assert_eq!(target.workspace, retained_workspace);
        assert_eq!(target.history, retained_history);
    }

    #[test]
    fn persisted_raw_or_foreign_cached_job_identity_rejects_the_whole_session() {
        let reference = ExactControlWorkspace::try_new().unwrap();
        let mut foreign_job_workspace = reference.cached_jobs.workspace.clone();
        let foreign_node = foreign_job_workspace.graph().nodes()[0].id();
        let foreign_handle = JobGraphHandle {
            global_job_digest: Digest([0x91; 32]),
            ..reference.cached_jobs.catalog.entries()[0].handle()
        };
        let foreign_parameter = foreign_job_workspace
            .graph()
            .node(foreign_node)
            .unwrap()
            .parameters()[0]
            .value()
            .replacing_value_at_path(
                reference.cached_jobs.registry.context_schema(),
                &CACHED_JOB_MIRROR_PATH,
                GraphValue::JobHandle(foreign_handle),
            )
            .unwrap();
        foreign_job_workspace
            .set_parameter(foreign_node, CACHED_JOB_PARAMETER, foreign_parameter)
            .unwrap();
        let foreign_job_encoding = encode_graph_workspace(&foreign_job_workspace).unwrap();
        let reference_session = reference.authoring_session_encoding().unwrap();
        let mut foreign_bundle = reference_session.bytes().to_vec();
        replace_exact_embedded_bytes(
            &mut foreign_bundle,
            reference.cached_jobs.encoding.bytes(),
            foreign_job_encoding.bytes(),
        );
        let foreign_bundle = persisted_algs(&foreign_bundle);

        let fallback =
            ExactControlWorkspace::try_new_with_persisted(Some(&foreign_bundle)).unwrap();
        assert_eq!(fallback.workspace, reference.workspace);
        assert_eq!(
            fallback.cached_jobs.workspace,
            reference.cached_jobs.workspace
        );
        assert!(fallback.edit_status.contains("rejected atomically"));
        assert!(fallback.persistence_pending());
    }

    #[test]
    fn headless_exact_control_workspace_produces_a_complete_egui_frame() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.selected_node = Some(GraphNodeId::new(8));
        let context = egui::Context::default();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_280.0, 900.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| workspace.show(ui));
            },
        );
        assert!(!output.shapes.is_empty());
        assert!(!output.textures_delta.set.is_empty());
        assert_eq!(
            workspace
                .node_label_drafts
                .get(&GraphNodeId::new(8))
                .map(String::as_str),
            Some("Proportional term")
        );

        for id in 1..=4 {
            workspace.remove_probe(GraphProbeId::new(id));
        }
        assert_eq!(
            workspace.probes.as_ref().unwrap().document.probes().len(),
            3
        );
        let digital_only = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_280.0, 360.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| workspace.show_trace(ui));
            },
        );
        assert!(!digital_only.shapes.is_empty());
    }
}
