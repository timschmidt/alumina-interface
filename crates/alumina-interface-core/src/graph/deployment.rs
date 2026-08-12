//! Fixed-memory lowering for the first resource-free Service/Realtime subset.
//!
//! This compiler consumes an already validated structural document and audited
//! semantic registry. It emits only canonical [`alumina_graph_ir`] packages;
//! firmware never receives or interprets the arbitrary graph document.

use core::fmt;
use std::collections::BTreeMap;

use alumina_graph_ir::{
    BOOLEAN_LATEST_STATE_BYTES, GraphIrChannel, GraphIrChannelOwner, GraphIrDomain, GraphIrError,
    GraphIrFullPolicy, GraphIrHeader, GraphIrNode, GraphIrOpcode, GraphIrPackage, GraphIrSchedule,
    MAX_GRAPH_IR_CHANNELS, MAX_GRAPH_IR_NODES,
};
use alumina_protocol::{DeviceId, Digest};
use alumina_storage::sha256;

use super::{
    ChannelFullPolicy, ClockKind, ExecutionDomain, ExecutionDomainSet, GraphAnalysis,
    GraphAnalysisError, GraphAnalysisLimits, GraphClockId, GraphDocument, GraphNodeId,
    GraphNodeRegistry, GraphPortId, GraphTypeId, GraphValue, GraphWireError,
    InputConnectionRequirement, NodeInputChannelKind, NodeKind, NodeRateTransitionContract,
    NodeSchema, RateTransitionKind, TypeKind, WireDefinition, analyze_graph, encode_graph_document,
};

/// First fixed firmware behavior selected for one exact audited node kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphDeploymentNodeKind {
    /// Emit one Boolean parameter as a Service-domain Stream.
    BooleanStreamConstant {
        /// Sole Stream output.
        output: GraphPortId,
        /// Required Boolean parameter.
        parameter: u32,
    },
    /// Execute the audited Boolean latest-at-or-before transition on Realtime.
    BooleanLatest {
        /// Source Stream input.
        input: GraphPortId,
        /// Target Stream output.
        output: GraphPortId,
    },
    /// Consume one Boolean Stream on Realtime without a side effect.
    BooleanStreamSink {
        /// Sole Stream input.
        input: GraphPortId,
    },
}

/// Exact kind/version to reviewed fixed opcode and schedule binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDeploymentImplementation {
    kind: NodeKind,
    behavior: GraphDeploymentNodeKind,
    schedule_clock: GraphClockId,
    wcet_cycles: u64,
}

impl GraphDeploymentImplementation {
    /// Construct one fixed deployment binding.
    pub fn new(
        kind: NodeKind,
        behavior: GraphDeploymentNodeKind,
        schedule_clock: GraphClockId,
        wcet_cycles: u64,
    ) -> Self {
        Self {
            kind,
            behavior,
            schedule_clock,
            wcet_cycles,
        }
    }

    /// Borrow the exact opaque node kind/version being implemented.
    pub const fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Return the fixed opcode family and graph-port binding.
    pub const fn behavior(&self) -> GraphDeploymentNodeKind {
        self.behavior
    }

    /// Return the exact graph clock used by the cyclic executor.
    pub const fn schedule_clock(&self) -> GraphClockId {
        self.schedule_clock
    }

    /// Return reviewed worst-case device cycles per invocation.
    pub const fn wcet_cycles(&self) -> u64 {
        self.wcet_cycles
    }
}

/// Canonical reviewed implementation registry above audited graph semantics.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphDeploymentRegistry {
    semantic: GraphNodeRegistry,
    implementations: Vec<GraphDeploymentImplementation>,
}

impl GraphDeploymentRegistry {
    /// Validate and canonicalize every fixed implementation binding.
    pub fn try_new(
        semantic: GraphNodeRegistry,
        mut implementations: Vec<GraphDeploymentImplementation>,
    ) -> Result<Self, GraphDeploymentError> {
        implementations.sort_unstable_by(|left, right| compare_kind(&left.kind, &right.kind));
        let mut previous: Option<&NodeKind> = None;
        for implementation in &implementations {
            if previous.is_some_and(|kind| compare_kind(kind, &implementation.kind).is_eq()) {
                return Err(GraphDeploymentError::DuplicateImplementation(
                    implementation.kind.clone(),
                ));
            }
            previous = Some(&implementation.kind);
            let schema = semantic.schema(&implementation.kind).ok_or_else(|| {
                GraphDeploymentError::UnknownImplementationKind(implementation.kind.clone())
            })?;
            validate_implementation(schema, implementation, semantic.context_schema())?;
        }
        Ok(Self {
            semantic,
            implementations,
        })
    }

    /// Borrow the audited semantic authority below the opcode registry.
    pub const fn semantic_registry(&self) -> &GraphNodeRegistry {
        &self.semantic
    }

    /// Borrow fixed bindings in canonical kind/version order.
    pub fn implementations(&self) -> &[GraphDeploymentImplementation] {
        &self.implementations
    }

    /// Resolve one exact kind/version.
    pub fn implementation(&self, kind: &NodeKind) -> Option<&GraphDeploymentImplementation> {
        self.implementations
            .binary_search_by(|implementation| compare_kind(&implementation.kind, kind))
            .ok()
            .map(|index| &self.implementations[index])
    }
}

/// Independent host-side lowering limits in addition to graph-IR format bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphDeploymentLimits {
    /// Maximum nodes in one MCU partition.
    pub maximum_nodes: usize,
    /// Maximum channels in one MCU partition.
    pub maximum_channels: usize,
    /// Maximum combined Service/Realtime node state.
    pub maximum_state_bytes: u32,
    /// Maximum combined channel storage.
    pub maximum_channel_bytes: u32,
    /// Maximum one-way Service-to-Realtime bridge storage.
    pub maximum_bridge_bytes: u32,
    /// Maximum reviewed WCET for one fixed node invocation.
    pub maximum_wcet_cycles_per_node: u64,
    /// Cycles reserved per active domain release for executor and queue overhead.
    pub executor_reserve_cycles: u64,
}

impl GraphDeploymentLimits {
    /// First resource-free deployment policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_nodes: MAX_GRAPH_IR_NODES,
            maximum_channels: MAX_GRAPH_IR_CHANNELS,
            maximum_state_bytes: 64 * 1024,
            maximum_channel_bytes: 1024 * 1024,
            maximum_bridge_bytes: 256 * 1024,
            maximum_wcet_cycles_per_node: 1_000_000,
            executor_reserve_cycles: 100,
        }
    }

    fn validate(self) -> Result<(), GraphDeploymentError> {
        if self.maximum_nodes == 0
            || self.maximum_nodes > MAX_GRAPH_IR_NODES
            || self.maximum_channels == 0
            || self.maximum_channels > MAX_GRAPH_IR_CHANNELS
            || self.maximum_state_bytes == 0
            || self.maximum_channel_bytes == 0
            || self.maximum_bridge_bytes == 0
            || self.maximum_wcet_cycles_per_node == 0
            || self.executor_reserve_cycles == 0
        {
            Err(GraphDeploymentError::InvalidLimits)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphDeploymentLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Exact target identities bound into one deployed package.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphDeploymentTarget {
    /// Stable target MCU.
    pub device_id: DeviceId,
    /// Target capability ledger.
    pub capability_digest: Digest,
    /// Active stored configuration.
    pub config_digest: Digest,
}

/// Successful fixed lowering and all host-side admission evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphDeploymentReport {
    analysis: GraphAnalysis,
    implementation_digest: Digest,
    package: GraphIrPackage,
}

impl GraphDeploymentReport {
    /// Borrow the complete audited semantic analysis used by lowering.
    pub const fn analysis(&self) -> &GraphAnalysis {
        &self.analysis
    }

    /// Return the graph-bound fixed implementation descriptor identity.
    pub const fn implementation_digest(&self) -> Digest {
        self.implementation_digest
    }

    /// Borrow the independently replayed firmware package.
    pub const fn package(&self) -> &GraphIrPackage {
        &self.package
    }

    /// Return all fixed state plus channel bytes, excluding package storage.
    pub fn fixed_runtime_bytes(&self) -> Result<u32, GraphDeploymentError> {
        self.package
            .header()
            .total_state_bytes
            .checked_add(self.package.header().channel_storage_bytes)
            .ok_or(GraphDeploymentError::Arithmetic)
    }
}

/// Failure at fixed implementation admission or graph-to-firmware lowering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphDeploymentError {
    /// Lowering policy was zero or exceeded the graph-IR format.
    InvalidLimits,
    /// A target identity was all zero.
    MissingIdentity(&'static str),
    /// Canonical graph encoding failed.
    Graph(GraphWireError),
    /// Audited semantic analysis failed.
    Analysis(GraphAnalysisError),
    /// Canonical graph-IR construction or replay failed.
    Ir(GraphIrError),
    /// Two implementations claimed one exact kind/version.
    DuplicateImplementation(NodeKind),
    /// A binding named no audited semantic schema.
    UnknownImplementationKind(NodeKind),
    /// A fixed implementation contradicted its audited schema.
    InvalidImplementation {
        /// Exact kind/version.
        kind: NodeKind,
        /// Rejected implementation fact.
        aspect: &'static str,
    },
    /// A document node had no reviewed fixed implementation.
    UnimplementedNode {
        /// Exact node instance.
        node: GraphNodeId,
        /// Exact opaque kind/version.
        kind: NodeKind,
    },
    /// Host, foreign-device, or otherwise unsupported placement was requested.
    UnsupportedDomain {
        /// Exact node instance.
        node: GraphNodeId,
        /// Requested placement.
        domain: ExecutionDomain,
    },
    /// A node schedule did not resolve to an integer period on the target root.
    InvalidSchedule {
        /// Exact node instance.
        node: GraphNodeId,
        /// Rejected schedule fact.
        aspect: &'static str,
    },
    /// One domain selected more than one cyclic-executive clock/period.
    MixedDomainSchedule(GraphIrDomain),
    /// A per-node or aggregate WCET exceeded its admitted cycle window.
    WcetExceeded(GraphIrDomain),
    /// A fixed collection or arena exceeded host policy.
    LimitExceeded(&'static str),
    /// A fixed node could not be lowered to its exact opcode immediate/state.
    InvalidNode {
        /// Exact node instance.
        node: GraphNodeId,
        /// Rejected fact.
        aspect: &'static str,
    },
    /// A structural wire/channel could not enter the fixed subset.
    InvalidChannel {
        /// Original wire identity.
        wire: super::GraphWireId,
        /// Rejected fact.
        aspect: &'static str,
    },
    /// Structural topological ordering could not cover every node.
    Topology,
    /// Canonical implementation-identity encoding could not represent a value.
    RegistryEncoding,
    /// Checked integer arithmetic overflowed.
    Arithmetic,
}

impl From<GraphWireError> for GraphDeploymentError {
    fn from(value: GraphWireError) -> Self {
        Self::Graph(value)
    }
}

impl From<GraphAnalysisError> for GraphDeploymentError {
    fn from(value: GraphAnalysisError) -> Self {
        Self::Analysis(value)
    }
}

impl From<GraphIrError> for GraphDeploymentError {
    fn from(value: GraphIrError) -> Self {
        Self::Ir(value)
    }
}

impl fmt::Display for GraphDeploymentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("graph deployment limits are invalid"),
            Self::MissingIdentity(name) => write!(formatter, "graph deployment {name} is missing"),
            Self::Graph(error) => write!(formatter, "graph deployment encoding rejected: {error}"),
            Self::Analysis(error) => {
                write!(formatter, "graph deployment analysis rejected: {error}")
            }
            Self::Ir(error) => write!(formatter, "graph deployment IR rejected: {error}"),
            Self::DuplicateImplementation(kind) => write!(
                formatter,
                "graph deployment implementation {}@{} is duplicated",
                kind.name(),
                kind.version()
            ),
            Self::UnknownImplementationKind(kind) => write!(
                formatter,
                "graph deployment implementation {}@{} has no audited schema",
                kind.name(),
                kind.version()
            ),
            Self::InvalidImplementation { kind, aspect } => write!(
                formatter,
                "graph deployment implementation {}@{} contradicts {aspect}",
                kind.name(),
                kind.version()
            ),
            Self::UnimplementedNode { node, kind } => write!(
                formatter,
                "graph node {node:?} kind {}@{} has no fixed deployment implementation",
                kind.name(),
                kind.version()
            ),
            Self::UnsupportedDomain { node, domain } => {
                write!(
                    formatter,
                    "graph node {node:?} has unsupported domain {domain:?}"
                )
            }
            Self::InvalidSchedule { node, aspect } => {
                write!(
                    formatter,
                    "graph node {node:?} has invalid schedule {aspect}"
                )
            }
            Self::MixedDomainSchedule(domain) => {
                write!(formatter, "graph deployment {domain:?} clocks differ")
            }
            Self::WcetExceeded(domain) => {
                write!(
                    formatter,
                    "graph deployment {domain:?} WCET exceeds its period"
                )
            }
            Self::LimitExceeded(name) => {
                write!(formatter, "graph deployment {name} exceeds policy")
            }
            Self::InvalidNode { node, aspect } => {
                write!(formatter, "graph node {node:?} cannot lower {aspect}")
            }
            Self::InvalidChannel { wire, aspect } => {
                write!(formatter, "graph wire {wire:?} cannot lower {aspect}")
            }
            Self::Topology => formatter.write_str("graph deployment topology is not acyclic"),
            Self::RegistryEncoding => {
                formatter.write_str("graph deployment registry identity cannot encode")
            }
            Self::Arithmetic => formatter.write_str("graph deployment arithmetic overflowed"),
        }
    }
}

impl std::error::Error for GraphDeploymentError {}

/// Lower one single-device fixed subset into independently replayed firmware IR.
pub fn lower_graph_deployment(
    document: &GraphDocument,
    registry: &GraphDeploymentRegistry,
    target: GraphDeploymentTarget,
    limits: GraphDeploymentLimits,
) -> Result<GraphDeploymentReport, GraphDeploymentError> {
    limits.validate()?;
    validate_target(target)?;
    if document.nodes().len() > limits.maximum_nodes {
        return Err(GraphDeploymentError::LimitExceeded("node count"));
    }
    if document.wires().len() > limits.maximum_channels {
        return Err(GraphDeploymentError::LimitExceeded("channel count"));
    }
    let analysis = analyze_graph(document, &registry.semantic)?;
    if !analysis.state_allocations().is_empty() {
        return Err(GraphDeploymentError::LimitExceeded(
            "explicit state in fixed V1 subset",
        ));
    }
    let graph_digest = encode_graph_document(document)?.digest();
    let implementation_digest = deployment_digest(registry, graph_digest, limits)?;
    let order = topological_order(document)?;
    let mut topological_indices = BTreeMap::new();
    for (index, node) in order.iter().enumerate() {
        topological_indices.insert(
            *node,
            u16::try_from(index).map_err(|_| GraphDeploymentError::Arithmetic)?,
        );
    }

    let mut service_schedule = ScheduleBuilder::new(limits.executor_reserve_cycles);
    let mut realtime_schedule = ScheduleBuilder::new(limits.executor_reserve_cycles);
    let mut service_state = 0_u32;
    let mut realtime_state = 0_u32;
    let mut nodes = Vec::with_capacity(order.len());
    for node_id in &order {
        let node = document
            .node(*node_id)
            .ok_or(GraphDeploymentError::Topology)?;
        let domain = deployed_domain(node.id(), node.domain(), target.device_id)?;
        let implementation = registry.implementation(node.kind()).ok_or_else(|| {
            GraphDeploymentError::UnimplementedNode {
                node: node.id(),
                kind: node.kind().clone(),
            }
        })?;
        if implementation_domain(implementation.behavior) != domain {
            return Err(GraphDeploymentError::InvalidNode {
                node: node.id(),
                aspect: "opcode execution domain",
            });
        }
        if implementation.wcet_cycles == 0
            || implementation.wcet_cycles > limits.maximum_wcet_cycles_per_node
        {
            return Err(GraphDeploymentError::InvalidSchedule {
                node: node.id(),
                aspect: "per-node WCET",
            });
        }
        let period = integer_device_period(
            document,
            &analysis,
            implementation.schedule_clock,
            target.device_id,
            node.id(),
        )?;
        let schedule = match domain {
            GraphIrDomain::Service => &mut service_schedule,
            GraphIrDomain::Realtime => &mut realtime_schedule,
        };
        schedule.add(
            domain,
            implementation.schedule_clock,
            period,
            implementation.wcet_cycles,
        )?;
        let (opcode, state_bytes, parameter) = lower_node(node, implementation, &analysis)?;
        let state_offset = match domain {
            GraphIrDomain::Service => {
                let offset = service_state;
                service_state = service_state
                    .checked_add(state_bytes)
                    .ok_or(GraphDeploymentError::Arithmetic)?;
                offset
            }
            GraphIrDomain::Realtime => {
                let offset = realtime_state;
                realtime_state = realtime_state
                    .checked_add(state_bytes)
                    .ok_or(GraphDeploymentError::Arithmetic)?;
                offset
            }
        };
        nodes.push(GraphIrNode {
            graph_node_id: node.id().get(),
            domain,
            opcode,
            schedule_clock_id: implementation.schedule_clock.get(),
            period_cycles: period,
            wcet_cycles: implementation.wcet_cycles,
            state_offset,
            state_bytes,
            parameter,
        });
    }
    let total_state = service_state
        .checked_add(realtime_state)
        .ok_or(GraphDeploymentError::Arithmetic)?;
    if total_state > limits.maximum_state_bytes {
        return Err(GraphDeploymentError::LimitExceeded("state bytes"));
    }

    let mut ordered_wires = document.wires().to_vec();
    ordered_wires.sort_unstable_by_key(|wire| {
        (
            topological_indices
                .get(&wire.target().node)
                .copied()
                .unwrap_or(u16::MAX),
            wire.id(),
        )
    });
    let mut owner_offsets = [0_u32; 3];
    let mut channels = Vec::with_capacity(ordered_wires.len());
    for wire in ordered_wires {
        channels.push(lower_channel(
            document,
            &analysis,
            wire,
            &topological_indices,
            &mut owner_offsets,
        )?);
    }
    let channel_storage = owner_offsets
        .iter()
        .try_fold(0_u32, |sum, bytes| sum.checked_add(*bytes))
        .ok_or(GraphDeploymentError::Arithmetic)?;
    let bridge_storage = owner_offsets[2];
    if channel_storage > limits.maximum_channel_bytes {
        return Err(GraphDeploymentError::LimitExceeded("channel bytes"));
    }
    if bridge_storage > limits.maximum_bridge_bytes {
        return Err(GraphDeploymentError::LimitExceeded("bridge bytes"));
    }
    let package = GraphIrPackage::encode(
        GraphIrHeader {
            device_id: target.device_id,
            graph_digest,
            implementation_digest,
            capability_digest: target.capability_digest,
            config_digest: target.config_digest,
            service_schedule: service_schedule.finish(),
            realtime_schedule: realtime_schedule.finish(),
            total_state_bytes: total_state,
            service_state_bytes: service_state,
            realtime_state_bytes: realtime_state,
            channel_storage_bytes: channel_storage,
            bridge_storage_bytes: bridge_storage,
        },
        &nodes,
        &channels,
    )?;
    Ok(GraphDeploymentReport {
        analysis,
        implementation_digest,
        package,
    })
}

struct ScheduleBuilder {
    clock: Option<GraphClockId>,
    period: u64,
    total_wcet: u64,
    executor_reserve: u64,
    nodes: u16,
}

impl ScheduleBuilder {
    const fn new(executor_reserve: u64) -> Self {
        Self {
            clock: None,
            period: 0,
            total_wcet: 0,
            executor_reserve,
            nodes: 0,
        }
    }

    fn add(
        &mut self,
        domain: GraphIrDomain,
        clock: GraphClockId,
        period: u64,
        wcet: u64,
    ) -> Result<(), GraphDeploymentError> {
        match self.clock {
            None => {
                self.clock = Some(clock);
                self.period = period;
            }
            Some(existing) if existing == clock && self.period == period => {}
            Some(_) => return Err(GraphDeploymentError::MixedDomainSchedule(domain)),
        }
        self.total_wcet = self
            .total_wcet
            .checked_add(wcet)
            .ok_or(GraphDeploymentError::Arithmetic)?;
        if self
            .total_wcet
            .checked_add(self.executor_reserve)
            .is_none_or(|required| required > self.period)
        {
            return Err(GraphDeploymentError::WcetExceeded(domain));
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or(GraphDeploymentError::Arithmetic)?;
        Ok(())
    }

    fn finish(self) -> GraphIrSchedule {
        match self.clock {
            None => GraphIrSchedule::EMPTY,
            Some(clock) => GraphIrSchedule {
                clock_id: clock.get(),
                period_cycles: self.period,
                total_wcet_cycles: self.total_wcet,
                executor_reserve_cycles: self.executor_reserve,
                node_count: self.nodes,
            },
        }
    }
}

fn lower_node(
    node: &super::NodeDefinition,
    implementation: &GraphDeploymentImplementation,
    analysis: &GraphAnalysis,
) -> Result<(GraphIrOpcode, u32, u64), GraphDeploymentError> {
    match implementation.behavior {
        GraphDeploymentNodeKind::BooleanStreamConstant { parameter, .. } => {
            let value = node
                .parameters()
                .iter()
                .find(|candidate| candidate.id() == parameter)
                .ok_or(GraphDeploymentError::InvalidNode {
                    node: node.id(),
                    aspect: "constant parameter",
                })?;
            let GraphValue::Boolean(value) = value.value().value() else {
                return Err(GraphDeploymentError::InvalidNode {
                    node: node.id(),
                    aspect: "constant Boolean",
                });
            };
            Ok((GraphIrOpcode::BooleanStreamConstant, 0, u64::from(*value)))
        }
        GraphDeploymentNodeKind::BooleanLatest { input, output } => {
            let transition = analysis
                .rate_transitions()
                .iter()
                .find(|transition| {
                    transition.node() == node.id()
                        && transition.input() == input
                        && transition.output() == output
                })
                .copied()
                .ok_or(GraphDeploymentError::InvalidNode {
                    node: node.id(),
                    aspect: "audited rate transition",
                })?;
            let retained = u32::try_from(transition.retained_sample_bytes()).map_err(|_| {
                GraphDeploymentError::InvalidNode {
                    node: node.id(),
                    aspect: "retained state width",
                }
            })?;
            if retained != BOOLEAN_LATEST_STATE_BYTES {
                return Err(GraphDeploymentError::InvalidNode {
                    node: node.id(),
                    aspect: "Boolean retained state width",
                });
            }
            Ok((GraphIrOpcode::BooleanLatest, retained, 0))
        }
        GraphDeploymentNodeKind::BooleanStreamSink { .. } => {
            Ok((GraphIrOpcode::BooleanStreamSink, 0, 0))
        }
    }
}

fn lower_channel(
    document: &GraphDocument,
    analysis: &GraphAnalysis,
    wire: WireDefinition,
    indices: &BTreeMap<GraphNodeId, u16>,
    owner_offsets: &mut [u32; 3],
) -> Result<GraphIrChannel, GraphDeploymentError> {
    let source_node = document
        .node(wire.source().node)
        .ok_or(GraphDeploymentError::Topology)?;
    let target_node = document
        .node(wire.target().node)
        .ok_or(GraphDeploymentError::Topology)?;
    let source_domain =
        ir_domain(source_node.domain()).ok_or(GraphDeploymentError::InvalidChannel {
            wire: wire.id(),
            aspect: "source domain",
        })?;
    let target_domain =
        ir_domain(target_node.domain()).ok_or(GraphDeploymentError::InvalidChannel {
            wire: wire.id(),
            aspect: "target domain",
        })?;
    let owner = match (source_domain, target_domain) {
        (GraphIrDomain::Service, GraphIrDomain::Service) => GraphIrChannelOwner::Service,
        (GraphIrDomain::Realtime, GraphIrDomain::Realtime) => GraphIrChannelOwner::Realtime,
        (GraphIrDomain::Service, GraphIrDomain::Realtime) => GraphIrChannelOwner::ServiceToRealtime,
        (GraphIrDomain::Realtime, GraphIrDomain::Service) => {
            return Err(GraphDeploymentError::InvalidChannel {
                wire: wire.id(),
                aspect: "Realtime-to-Service direction",
            });
        }
    };
    let allocation = analysis
        .channel_allocations()
        .iter()
        .find(|allocation| allocation.target() == wire.target())
        .copied()
        .ok_or(GraphDeploymentError::InvalidChannel {
            wire: wire.id(),
            aspect: "audited allocation",
        })?;
    if allocation.source() != wire.source() {
        return Err(GraphDeploymentError::InvalidChannel {
            wire: wire.id(),
            aspect: "audited source",
        });
    }
    let (capacity, full_policy) = match allocation.kind() {
        NodeInputChannelKind::StreamQueue {
            capacity,
            full_policy,
        } => (capacity, lower_full_policy(full_policy)),
        NodeInputChannelKind::Synchronous | NodeInputChannelKind::EventQueue { .. } => {
            return Err(GraphDeploymentError::InvalidChannel {
                wire: wire.id(),
                aspect: "non-Stream delivery",
            });
        }
    };
    if target_domain == GraphIrDomain::Realtime && full_policy != GraphIrFullPolicy::Fault {
        return Err(GraphDeploymentError::InvalidChannel {
            wire: wire.id(),
            aspect: "Realtime queue must fault when full",
        });
    }
    let item_bytes = u32::try_from(allocation.maximum_item_bytes()).map_err(|_| {
        GraphDeploymentError::InvalidChannel {
            wire: wire.id(),
            aspect: "item byte width",
        }
    })?;
    let storage_bytes = u32::try_from(allocation.maximum_total_bytes()).map_err(|_| {
        GraphDeploymentError::InvalidChannel {
            wire: wire.id(),
            aspect: "storage byte width",
        }
    })?;
    let arena = channel_owner_index(owner);
    let storage_offset = owner_offsets[arena];
    owner_offsets[arena] = owner_offsets[arena]
        .checked_add(storage_bytes)
        .ok_or(GraphDeploymentError::Arithmetic)?;
    Ok(GraphIrChannel {
        graph_wire_id: wire.id().get(),
        source_node: *indices
            .get(&wire.source().node)
            .ok_or(GraphDeploymentError::Topology)?,
        target_node: *indices
            .get(&wire.target().node)
            .ok_or(GraphDeploymentError::Topology)?,
        owner,
        full_policy,
        capacity,
        item_bytes,
        storage_offset,
        storage_bytes,
    })
}

fn integer_device_period(
    document: &GraphDocument,
    analysis: &GraphAnalysis,
    clock: GraphClockId,
    device: DeviceId,
    node: GraphNodeId,
) -> Result<u64, GraphDeploymentError> {
    let rate = analysis
        .clock_rate(clock)
        .ok_or(GraphDeploymentError::InvalidSchedule {
            node,
            aspect: "missing clock",
        })?;
    let root = analysis
        .clock_rate(rate.root())
        .ok_or(GraphDeploymentError::InvalidSchedule {
            node,
            aspect: "missing root",
        })?;
    let definition = document
        .clock(rate.root())
        .ok_or(GraphDeploymentError::InvalidSchedule {
            node,
            aspect: "missing root definition",
        })?;
    if !matches!(
        definition.kind(),
        ClockKind::DeviceCycle { device_id, .. } if device_id == device
    ) {
        return Err(GraphDeploymentError::InvalidSchedule {
            node,
            aspect: "foreign or non-device root",
        });
    }
    let period = root.ticks_per_second().clone() / rate.ticks_per_second().clone();
    if !period.is_integer() {
        return Err(GraphDeploymentError::InvalidSchedule {
            node,
            aspect: "noninteger device-cycle period",
        });
    }
    u64::try_from(period.numerator().clone()).map_err(|_| GraphDeploymentError::InvalidSchedule {
        node,
        aspect: "period width",
    })
}

fn topological_order(document: &GraphDocument) -> Result<Vec<GraphNodeId>, GraphDeploymentError> {
    let mut indegree = vec![0_usize; document.nodes().len()];
    for wire in document.wires() {
        let target = document
            .nodes()
            .iter()
            .position(|node| node.id() == wire.target().node)
            .ok_or(GraphDeploymentError::Topology)?;
        indegree[target] = indegree[target]
            .checked_add(1)
            .ok_or(GraphDeploymentError::Arithmetic)?;
    }
    let mut emitted = vec![false; document.nodes().len()];
    let mut order = Vec::with_capacity(document.nodes().len());
    while order.len() < document.nodes().len() {
        let Some(index) =
            (0..document.nodes().len()).find(|index| !emitted[*index] && indegree[*index] == 0)
        else {
            return Err(GraphDeploymentError::Topology);
        };
        emitted[index] = true;
        let source = document.nodes()[index].id();
        order.push(source);
        for wire in document
            .wires()
            .iter()
            .filter(|wire| wire.source().node == source)
        {
            let target = document
                .nodes()
                .iter()
                .position(|node| node.id() == wire.target().node)
                .ok_or(GraphDeploymentError::Topology)?;
            indegree[target] = indegree[target]
                .checked_sub(1)
                .ok_or(GraphDeploymentError::Topology)?;
        }
    }
    Ok(order)
}

fn validate_implementation(
    schema: &NodeSchema,
    implementation: &GraphDeploymentImplementation,
    values: &super::GraphSchema,
) -> Result<(), GraphDeploymentError> {
    let invalid = |aspect| GraphDeploymentError::InvalidImplementation {
        kind: schema.kind().clone(),
        aspect,
    };
    if implementation.schedule_clock.get() == 0 || implementation.wcet_cycles == 0 {
        return Err(invalid("schedule"));
    }
    if schema.state().is_some() {
        return Err(invalid("explicit state-free V1 subset"));
    }
    match implementation.behavior {
        GraphDeploymentNodeKind::BooleanStreamConstant { output, parameter } => {
            if !allows_domain(schema.allowed_domains(), ExecutionDomainSet::SERVICE)
                || !schema.inputs().is_empty()
                || schema.outputs().len() != 1
                || schema.outputs()[0].id() != output
                || boolean_stream_clock(values, schema.outputs()[0].value_type())
                    != Some(implementation.schedule_clock)
                || schema.parameters().len() != 1
                || schema.parameters()[0].id() != parameter
                || !is_boolean(values, schema.parameters()[0].value_type())
                || schema.output_dependencies().len() != 1
                || !schema.output_dependencies()[0].inputs().is_empty()
                || !schema.rate_transitions().is_empty()
            {
                return Err(invalid("Boolean Stream constant shape"));
            }
        }
        GraphDeploymentNodeKind::BooleanLatest { input, output } => {
            if !allows_domain(schema.allowed_domains(), ExecutionDomainSet::REALTIME)
                || schema.inputs().len() != 1
                || schema.outputs().len() != 1
                || schema.inputs()[0].id() != input
                || schema.outputs()[0].id() != output
                || boolean_stream_clock(values, schema.outputs()[0].value_type())
                    != Some(implementation.schedule_clock)
                || boolean_stream_clock(values, schema.inputs()[0].value_type()).is_none()
                || !schema.parameters().is_empty()
                || !required_fault_stream(schema, input)
                || schema.output_dependencies().len() != 1
                || schema.output_dependencies()[0].inputs() != [input]
                || schema.rate_transitions()
                    != [NodeRateTransitionContract::new(
                        input,
                        output,
                        RateTransitionKind::LatestAtOrBeforeSourceFirst,
                    )]
            {
                return Err(invalid("Boolean latest transition shape"));
            }
        }
        GraphDeploymentNodeKind::BooleanStreamSink { input } => {
            if !allows_domain(schema.allowed_domains(), ExecutionDomainSet::REALTIME)
                || schema.inputs().len() != 1
                || schema.inputs()[0].id() != input
                || boolean_stream_clock(values, schema.inputs()[0].value_type())
                    != Some(implementation.schedule_clock)
                || !required_fault_stream(schema, input)
                || !schema.outputs().is_empty()
                || !schema.parameters().is_empty()
                || !schema.output_dependencies().is_empty()
                || !schema.rate_transitions().is_empty()
            {
                return Err(invalid("Boolean Stream sink shape"));
            }
        }
    }
    Ok(())
}

const fn allows_domain(set: ExecutionDomainSet, required: ExecutionDomainSet) -> bool {
    set.bits() & required.bits() != 0
}

fn required_fault_stream(schema: &NodeSchema, input: GraphPortId) -> bool {
    schema.input_channels().iter().any(|channel| {
        channel.port() == input
            && channel.requirement() == InputConnectionRequirement::Required
            && matches!(
                channel.kind(),
                NodeInputChannelKind::StreamQueue {
                    full_policy: ChannelFullPolicy::Fault,
                    ..
                }
            )
    })
}

fn boolean_stream_clock(
    schema: &super::GraphSchema,
    value_type: GraphTypeId,
) -> Option<GraphClockId> {
    let TypeKind::Stream { sample, clock, .. } = schema.value_type(value_type)?.kind() else {
        return None;
    };
    is_boolean(schema, *sample).then_some(*clock)
}

fn is_boolean(schema: &super::GraphSchema, value_type: GraphTypeId) -> bool {
    matches!(
        schema
            .value_type(value_type)
            .map(super::TypeDefinition::kind),
        Some(TypeKind::Boolean)
    )
}

fn deployed_domain(
    node: GraphNodeId,
    domain: ExecutionDomain,
    target: DeviceId,
) -> Result<GraphIrDomain, GraphDeploymentError> {
    match domain {
        ExecutionDomain::Service { device_id } if device_id == target => Ok(GraphIrDomain::Service),
        ExecutionDomain::Realtime { device_id } if device_id == target => {
            Ok(GraphIrDomain::Realtime)
        }
        ExecutionDomain::HostExact
        | ExecutionDomain::Service { .. }
        | ExecutionDomain::Realtime { .. } => {
            Err(GraphDeploymentError::UnsupportedDomain { node, domain })
        }
    }
}

const fn implementation_domain(behavior: GraphDeploymentNodeKind) -> GraphIrDomain {
    match behavior {
        GraphDeploymentNodeKind::BooleanStreamConstant { .. } => GraphIrDomain::Service,
        GraphDeploymentNodeKind::BooleanLatest { .. }
        | GraphDeploymentNodeKind::BooleanStreamSink { .. } => GraphIrDomain::Realtime,
    }
}

fn ir_domain(domain: ExecutionDomain) -> Option<GraphIrDomain> {
    match domain {
        ExecutionDomain::HostExact => None,
        ExecutionDomain::Service { .. } => Some(GraphIrDomain::Service),
        ExecutionDomain::Realtime { .. } => Some(GraphIrDomain::Realtime),
    }
}

fn lower_full_policy(policy: ChannelFullPolicy) -> GraphIrFullPolicy {
    match policy {
        ChannelFullPolicy::Backpressure => GraphIrFullPolicy::Backpressure,
        ChannelFullPolicy::Fault => GraphIrFullPolicy::Fault,
        ChannelFullPolicy::DropNewest => GraphIrFullPolicy::DropNewest,
        ChannelFullPolicy::DropOldest => GraphIrFullPolicy::DropOldest,
    }
}

const fn channel_owner_index(owner: GraphIrChannelOwner) -> usize {
    match owner {
        GraphIrChannelOwner::Service => 0,
        GraphIrChannelOwner::Realtime => 1,
        GraphIrChannelOwner::ServiceToRealtime => 2,
    }
}

fn validate_target(target: GraphDeploymentTarget) -> Result<(), GraphDeploymentError> {
    if target.device_id.0.iter().all(|byte| *byte == 0) {
        return Err(GraphDeploymentError::MissingIdentity("device"));
    }
    if target.capability_digest.is_zero() {
        return Err(GraphDeploymentError::MissingIdentity("capability digest"));
    }
    if target.config_digest.is_zero() {
        return Err(GraphDeploymentError::MissingIdentity(
            "configuration digest",
        ));
    }
    Ok(())
}

fn deployment_digest(
    registry: &GraphDeploymentRegistry,
    graph_digest: Digest,
    limits: GraphDeploymentLimits,
) -> Result<Digest, GraphDeploymentError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ALDI");
    put_u16(&mut bytes, 1);
    put_u16(&mut bytes, 0);
    bytes.extend_from_slice(&graph_digest.0);
    for value in [
        u64::try_from(limits.maximum_nodes).map_err(|_| GraphDeploymentError::RegistryEncoding)?,
        u64::try_from(limits.maximum_channels)
            .map_err(|_| GraphDeploymentError::RegistryEncoding)?,
        u64::from(limits.maximum_state_bytes),
        u64::from(limits.maximum_channel_bytes),
        u64::from(limits.maximum_bridge_bytes),
        limits.maximum_wcet_cycles_per_node,
        limits.executor_reserve_cycles,
    ] {
        put_u64(&mut bytes, value);
    }
    encode_analysis_limits(&mut bytes, registry.semantic.limits())?;
    encode_semantic_registry(&mut bytes, &registry.semantic)?;
    put_count(&mut bytes, registry.implementations.len())?;
    for implementation in &registry.implementations {
        put_text(&mut bytes, implementation.kind.name())?;
        put_u16(&mut bytes, implementation.kind.version());
        put_u32(&mut bytes, implementation.schedule_clock.get());
        put_u64(&mut bytes, implementation.wcet_cycles);
        match implementation.behavior {
            GraphDeploymentNodeKind::BooleanStreamConstant { output, parameter } => {
                bytes.push(0);
                put_u32(&mut bytes, output.get());
                put_u32(&mut bytes, parameter);
            }
            GraphDeploymentNodeKind::BooleanLatest { input, output } => {
                bytes.push(1);
                put_u32(&mut bytes, input.get());
                put_u32(&mut bytes, output.get());
            }
            GraphDeploymentNodeKind::BooleanStreamSink { input } => {
                bytes.push(2);
                put_u32(&mut bytes, input.get());
            }
        }
    }
    Ok(sha256(&bytes).digest)
}

fn encode_semantic_registry(
    bytes: &mut Vec<u8>,
    registry: &GraphNodeRegistry,
) -> Result<(), GraphDeploymentError> {
    put_count(bytes, registry.schemas().len())?;
    for schema in registry.schemas() {
        put_text(bytes, schema.kind().name())?;
        put_u16(bytes, schema.kind().version());
        bytes.push(schema.allowed_domains().bits());
        put_ports(bytes, schema.inputs())?;
        put_count(bytes, schema.input_channels().len())?;
        for channel in schema.input_channels() {
            put_u32(bytes, channel.port().get());
            bytes.push(match channel.requirement() {
                InputConnectionRequirement::Required => 0,
                InputConnectionRequirement::Optional => 1,
            });
            match channel.kind() {
                NodeInputChannelKind::Synchronous => bytes.push(0),
                NodeInputChannelKind::EventQueue {
                    capacity,
                    full_policy,
                } => {
                    bytes.push(1);
                    put_u32(bytes, capacity);
                    bytes.push(full_policy_tag(full_policy));
                }
                NodeInputChannelKind::StreamQueue {
                    capacity,
                    full_policy,
                } => {
                    bytes.push(2);
                    put_u32(bytes, capacity);
                    bytes.push(full_policy_tag(full_policy));
                }
            }
        }
        put_ports(bytes, schema.outputs())?;
        put_count(bytes, schema.parameters().len())?;
        for parameter in schema.parameters() {
            put_u32(bytes, parameter.id());
            put_text(bytes, parameter.name())?;
            put_u32(bytes, parameter.value_type().get());
        }
        put_count(bytes, schema.output_dependencies().len())?;
        for dependency in schema.output_dependencies() {
            put_u32(bytes, dependency.output().get());
            put_count(bytes, dependency.inputs().len())?;
            for input in dependency.inputs() {
                put_u32(bytes, input.get());
            }
        }
        put_count(bytes, schema.rate_transitions().len())?;
        for transition in schema.rate_transitions() {
            put_u32(bytes, transition.input().get());
            put_u32(bytes, transition.output().get());
            bytes.push(match transition.kind() {
                RateTransitionKind::LatestAtOrBeforeSourceFirst => 0,
            });
        }
        match schema.state() {
            None => bytes.push(0),
            Some(state) => {
                bytes.push(1);
                put_u32(bytes, state.clock().get());
                put_u32(bytes, state.value_type().get());
                put_u32(bytes, state.initial_parameter());
                put_u32(bytes, state.next_input().get());
                put_u32(bytes, state.current_output().get());
                put_u32(bytes, state.declared_storage_bytes());
            }
        }
    }
    Ok(())
}

fn encode_analysis_limits(
    bytes: &mut Vec<u8>,
    limits: GraphAnalysisLimits,
) -> Result<(), GraphDeploymentError> {
    for value in [
        limits.maximum_registered_kinds,
        limits.maximum_dependency_links,
        limits.maximum_cycle_witness_links,
        limits.maximum_state_bytes_per_node,
        limits.maximum_total_state_bytes,
        limits.maximum_queue_items_per_input,
        limits.maximum_rate_transitions,
    ] {
        put_u64(
            bytes,
            u64::try_from(value).map_err(|_| GraphDeploymentError::RegistryEncoding)?,
        );
    }
    for value in [
        limits.maximum_channel_bytes_per_input,
        limits.maximum_total_channel_bytes,
        limits.maximum_rate_pattern_ticks,
        limits.maximum_total_rate_transition_state_bytes,
    ] {
        put_u64(bytes, value);
    }
    Ok(())
}

fn put_count(bytes: &mut Vec<u8>, value: usize) -> Result<(), GraphDeploymentError> {
    put_u32(
        bytes,
        u32::try_from(value).map_err(|_| GraphDeploymentError::RegistryEncoding)?,
    );
    Ok(())
}

fn put_ports(
    bytes: &mut Vec<u8>,
    ports: &[super::PortDefinition],
) -> Result<(), GraphDeploymentError> {
    put_count(bytes, ports.len())?;
    for port in ports {
        put_u32(bytes, port.id().get());
        put_text(bytes, port.name())?;
        put_u32(bytes, port.value_type().get());
    }
    Ok(())
}

fn put_text(bytes: &mut Vec<u8>, value: &str) -> Result<(), GraphDeploymentError> {
    put_count(bytes, value.len())?;
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

const fn full_policy_tag(policy: ChannelFullPolicy) -> u8 {
    match policy {
        ChannelFullPolicy::Backpressure => 0,
        ChannelFullPolicy::Fault => 1,
        ChannelFullPolicy::DropNewest => 2,
        ChannelFullPolicy::DropOldest => 3,
    }
}

fn compare_kind(left: &NodeKind, right: &NodeKind) -> core::cmp::Ordering {
    left.name()
        .cmp(right.name())
        .then_with(|| left.version().cmp(&right.version()))
}

#[cfg(test)]
mod tests {
    use alumina_graph_ir::{BOOLEAN_STREAM_ITEM_BYTES, GraphIrChannelOwner};

    use super::*;
    use crate::graph::{
        ClockDefinition, GraphLimits, GraphPortId, GraphSchema, GraphTypeId, GraphValue,
        GraphWireId, NodeDefinition, NodeInputChannelContract, NodeOutputDependency, NodeParameter,
        NodeParameterContract, PortDefinition, TypeDefinition, TypedGraphValue, WireEndpoint,
    };

    const BOOL: GraphTypeId = GraphTypeId::new(1);
    const SOURCE_STREAM: GraphTypeId = GraphTypeId::new(2);
    const TARGET_STREAM: GraphTypeId = GraphTypeId::new(3);
    const ROOT: GraphClockId = GraphClockId::new(1);
    const SOURCE_CLOCK: GraphClockId = GraphClockId::new(2);
    const TARGET_CLOCK: GraphClockId = GraphClockId::new(3);
    const DEVICE: DeviceId = DeviceId([7; 16]);

    fn port(id: u32, name: &str, value_type: GraphTypeId) -> PortDefinition {
        PortDefinition::new(GraphPortId::new(id), name, value_type)
    }

    fn endpoint(node: u32, port: u32) -> WireEndpoint {
        WireEndpoint {
            node: GraphNodeId::new(node),
            port: GraphPortId::new(port),
        }
    }

    fn implementation_bindings(latest_wcet: u64) -> Vec<GraphDeploymentImplementation> {
        vec![
            GraphDeploymentImplementation::new(
                NodeKind::new("deploy.sink", 1),
                GraphDeploymentNodeKind::BooleanStreamSink {
                    input: GraphPortId::new(1),
                },
                TARGET_CLOCK,
                20,
            ),
            GraphDeploymentImplementation::new(
                NodeKind::new("deploy.constant", 1),
                GraphDeploymentNodeKind::BooleanStreamConstant {
                    output: GraphPortId::new(1),
                    parameter: 1,
                },
                SOURCE_CLOCK,
                20,
            ),
            GraphDeploymentImplementation::new(
                NodeKind::new("deploy.latest", 1),
                GraphDeploymentNodeKind::BooleanLatest {
                    input: GraphPortId::new(1),
                    output: GraphPortId::new(2),
                },
                TARGET_CLOCK,
                latest_wcet,
            ),
        ]
    }

    fn fixture(
        source_numerator: u32,
        source_denominator: u32,
        transition_capacity: u32,
        latest_wcet: u64,
    ) -> (GraphDocument, GraphDeploymentRegistry) {
        fixture_with_constant_domains(
            source_numerator,
            source_denominator,
            transition_capacity,
            latest_wcet,
            ExecutionDomainSet::SERVICE,
        )
    }

    fn fixture_with_constant_domains(
        source_numerator: u32,
        source_denominator: u32,
        transition_capacity: u32,
        latest_wcet: u64,
        constant_domains: ExecutionDomainSet,
    ) -> (GraphDocument, GraphDeploymentRegistry) {
        let schema = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![
                TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    SOURCE_STREAM,
                    "stream.source",
                    TypeKind::Stream {
                        sample: BOOL,
                        clock: SOURCE_CLOCK,
                        capacity: 8,
                    },
                ),
                TypeDefinition::new(
                    TARGET_STREAM,
                    "stream.target",
                    TypeKind::Stream {
                        sample: BOOL,
                        clock: TARGET_CLOCK,
                        capacity: 8,
                    },
                ),
            ],
        )
        .unwrap();
        let parameter = TypedGraphValue::try_new(&schema, BOOL, GraphValue::Boolean(true)).unwrap();
        let clocks = vec![
            ClockDefinition::new(
                ROOT,
                "device.root",
                ClockKind::DeviceCycle {
                    device_id: DEVICE,
                    ticks_per_second: 1_000_000,
                },
            ),
            ClockDefinition::new(
                SOURCE_CLOCK,
                "device.source",
                ClockKind::Derived {
                    source: ROOT,
                    numerator: source_numerator,
                    denominator: source_denominator,
                },
            ),
            ClockDefinition::new(
                TARGET_CLOCK,
                "device.target",
                ClockKind::Derived {
                    source: ROOT,
                    numerator: 1,
                    denominator: 2_000,
                },
            ),
        ];
        let constant = NodeDefinition::new(
            GraphNodeId::new(10),
            NodeKind::new("deploy.constant", 1),
            "constant",
            ExecutionDomain::Service { device_id: DEVICE },
            Vec::new(),
            vec![port(1, "samples", SOURCE_STREAM)],
            vec![NodeParameter::new(1, "value", parameter)],
        );
        let latest = NodeDefinition::new(
            GraphNodeId::new(20),
            NodeKind::new("deploy.latest", 1),
            "latest",
            ExecutionDomain::Realtime { device_id: DEVICE },
            vec![port(1, "source", SOURCE_STREAM)],
            vec![port(2, "target", TARGET_STREAM)],
            Vec::new(),
        );
        let sink = NodeDefinition::new(
            GraphNodeId::new(30),
            NodeKind::new("deploy.sink", 1),
            "sink",
            ExecutionDomain::Realtime { device_id: DEVICE },
            vec![port(1, "samples", TARGET_STREAM)],
            Vec::new(),
            Vec::new(),
        );
        let document = GraphDocument::try_new(
            11,
            schema,
            clocks,
            vec![sink, constant, latest],
            vec![
                WireDefinition::new(GraphWireId::new(2), endpoint(20, 2), endpoint(30, 1)),
                WireDefinition::new(GraphWireId::new(1), endpoint(10, 1), endpoint(20, 1)),
            ],
        )
        .unwrap();
        let dependency = |output, inputs: &[u32]| {
            NodeOutputDependency::new(
                GraphPortId::new(output),
                inputs.iter().copied().map(GraphPortId::new).collect(),
            )
        };
        let stream_channel = |port, capacity| {
            NodeInputChannelContract::new(
                GraphPortId::new(port),
                InputConnectionRequirement::Required,
                NodeInputChannelKind::StreamQueue {
                    capacity,
                    full_policy: ChannelFullPolicy::Fault,
                },
            )
        };
        let constant_schema = NodeSchema::new(
            NodeKind::new("deploy.constant", 1),
            constant_domains,
            Vec::new(),
            Vec::new(),
            vec![port(1, "samples", SOURCE_STREAM)],
            vec![NodeParameterContract::new(1, "value", BOOL)],
            vec![dependency(1, &[])],
            Vec::new(),
            None,
        );
        let latest_schema = NodeSchema::new(
            NodeKind::new("deploy.latest", 1),
            ExecutionDomainSet::REALTIME,
            vec![port(1, "source", SOURCE_STREAM)],
            vec![stream_channel(1, transition_capacity)],
            vec![port(2, "target", TARGET_STREAM)],
            Vec::new(),
            vec![dependency(2, &[1])],
            vec![NodeRateTransitionContract::new(
                GraphPortId::new(1),
                GraphPortId::new(2),
                RateTransitionKind::LatestAtOrBeforeSourceFirst,
            )],
            None,
        );
        let sink_schema = NodeSchema::new(
            NodeKind::new("deploy.sink", 1),
            ExecutionDomainSet::REALTIME,
            vec![port(1, "samples", TARGET_STREAM)],
            vec![stream_channel(1, 1)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        let semantic = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &document,
            vec![sink_schema, constant_schema, latest_schema],
        )
        .unwrap();
        let registry =
            GraphDeploymentRegistry::try_new(semantic, implementation_bindings(latest_wcet))
                .unwrap();
        (document, registry)
    }

    fn target() -> GraphDeploymentTarget {
        GraphDeploymentTarget {
            device_id: DEVICE,
            capability_digest: Digest([8; 32]),
            config_digest: Digest([9; 32]),
        }
    }

    #[test]
    fn fixed_service_realtime_graph_lowers_to_replayed_ir() {
        let (document, registry) = fixture(1, 1_000, 2, 40);
        let report = lower_graph_deployment(
            &document,
            &registry,
            target(),
            GraphDeploymentLimits::interactive(),
        )
        .unwrap();
        let replay = GraphIrPackage::from_slice(report.package().bytes()).unwrap();
        assert_eq!(&replay, report.package());
        assert_eq!(report.fixed_runtime_bytes().unwrap(), 68);
        assert_eq!(
            report.package().header().service_schedule.period_cycles,
            1_000
        );
        assert_eq!(
            report.package().header().service_schedule.total_wcet_cycles,
            20
        );
        assert_eq!(
            report
                .package()
                .header()
                .service_schedule
                .executor_reserve_cycles,
            100
        );
        assert_eq!(
            report.package().header().realtime_schedule.period_cycles,
            2_000
        );
        assert_eq!(
            report
                .package()
                .header()
                .realtime_schedule
                .total_wcet_cycles,
            60
        );
        assert_eq!(
            report
                .package()
                .header()
                .realtime_schedule
                .executor_reserve_cycles,
            100
        );
        assert_eq!(report.package().summary().bridge_count, 1);
        assert_eq!(report.package().summary().bridge_storage_bytes, 42);
        assert_eq!(report.package().summary().channel_storage_bytes, 63);
        assert_eq!(
            report
                .package()
                .nodes()
                .map(|node| (node.graph_node_id, node.opcode))
                .collect::<Vec<_>>(),
            vec![
                (10, GraphIrOpcode::BooleanStreamConstant),
                (20, GraphIrOpcode::BooleanLatest),
                (30, GraphIrOpcode::BooleanStreamSink),
            ]
        );
        let channels: Vec<_> = report.package().channels().collect();
        assert_eq!(channels[0].owner, GraphIrChannelOwner::ServiceToRealtime);
        assert_eq!(channels[0].item_bytes, BOOLEAN_STREAM_ITEM_BYTES);
        assert_eq!(channels[0].storage_bytes, 42);
        assert_eq!(channels[1].owner, GraphIrChannelOwner::Realtime);
        assert_eq!(channels[1].storage_bytes, 21);
        assert_eq!(
            report.package().header().graph_digest,
            encode_graph_document(&document).unwrap().digest()
        );
        assert_eq!(
            report.package().header().implementation_digest,
            report.implementation_digest()
        );
        assert_eq!(
            report.package().digest(),
            Digest([
                0x80, 0x2d, 0x6a, 0x2f, 0x9b, 0x8d, 0x29, 0x58, 0x05, 0x55, 0x32, 0xae, 0xcd, 0xec,
                0x7b, 0x6d, 0xbe, 0xd6, 0x02, 0xc4, 0x4f, 0xb3, 0xf8, 0x0a, 0x17, 0x55, 0xd8, 0xf8,
                0x41, 0x2a, 0xca, 0x67,
            ])
        );
    }

    #[test]
    fn deployment_registry_order_does_not_change_package_identity() {
        let (document, registry) = fixture(1, 1_000, 2, 40);
        let first = lower_graph_deployment(
            &document,
            &registry,
            target(),
            GraphDeploymentLimits::interactive(),
        )
        .unwrap();
        let mut implementations = registry.implementations().to_vec();
        implementations.reverse();
        let rebuilt =
            GraphDeploymentRegistry::try_new(registry.semantic_registry().clone(), implementations)
                .unwrap();
        let second = lower_graph_deployment(
            &document,
            &rebuilt,
            target(),
            GraphDeploymentLimits::interactive(),
        )
        .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn implementation_identity_commits_to_complete_audited_semantics() {
        let (document, registry) = fixture(1, 1_000, 2, 40);
        let (broad_document, broad_registry) =
            fixture_with_constant_domains(1, 1_000, 2, 40, ExecutionDomainSet::ALL);
        assert_eq!(
            encode_graph_document(&document).unwrap().digest(),
            encode_graph_document(&broad_document).unwrap().digest()
        );
        let first = lower_graph_deployment(
            &document,
            &registry,
            target(),
            GraphDeploymentLimits::interactive(),
        )
        .unwrap();
        let broad = lower_graph_deployment(
            &broad_document,
            &broad_registry,
            target(),
            GraphDeploymentLimits::interactive(),
        )
        .unwrap();
        assert_ne!(first.implementation_digest(), broad.implementation_digest());
        assert_ne!(first.package().digest(), broad.package().digest());
    }

    #[test]
    fn noninteger_period_wcet_identity_and_arena_limits_fail_closed() {
        let (document, registry) = fixture(3, 2_000, 3, 40);
        assert!(matches!(
            lower_graph_deployment(
                &document,
                &registry,
                target(),
                GraphDeploymentLimits::interactive(),
            ),
            Err(GraphDeploymentError::InvalidSchedule {
                node,
                aspect: "noninteger device-cycle period",
            }) if node == GraphNodeId::new(10)
        ));

        let (document, registry) = fixture(1, 1_000, 2, 1_881);
        assert_eq!(
            lower_graph_deployment(
                &document,
                &registry,
                target(),
                GraphDeploymentLimits::interactive(),
            ),
            Err(GraphDeploymentError::WcetExceeded(GraphIrDomain::Realtime))
        );

        let (document, registry) = fixture(1, 1_000, 2, 40);
        let mut foreign = target();
        foreign.device_id = DeviceId([6; 16]);
        assert!(matches!(
            lower_graph_deployment(
                &document,
                &registry,
                foreign,
                GraphDeploymentLimits::interactive(),
            ),
            Err(GraphDeploymentError::UnsupportedDomain { node, .. })
                if node == GraphNodeId::new(10)
        ));

        let mut limits = GraphDeploymentLimits::interactive();
        limits.maximum_bridge_bytes = 41;
        assert_eq!(
            lower_graph_deployment(&document, &registry, target(), limits),
            Err(GraphDeploymentError::LimitExceeded("bridge bytes"))
        );
        let mut limits = GraphDeploymentLimits::interactive();
        limits.maximum_state_bytes = 4;
        assert_eq!(
            lower_graph_deployment(&document, &registry, target(), limits),
            Err(GraphDeploymentError::LimitExceeded("state bytes"))
        );
    }

    #[test]
    fn every_structural_node_requires_a_fixed_opcode_binding() {
        let (document, registry) = fixture(1, 1_000, 2, 40);
        let implementations: Vec<_> = registry
            .implementations()
            .iter()
            .filter(|implementation| implementation.kind().name() != "deploy.sink")
            .cloned()
            .collect();
        let partial =
            GraphDeploymentRegistry::try_new(registry.semantic_registry().clone(), implementations)
                .unwrap();
        assert!(matches!(
            lower_graph_deployment(
                &document,
                &partial,
                target(),
                GraphDeploymentLimits::interactive(),
            ),
            Err(GraphDeploymentError::UnimplementedNode { node, .. })
                if node == GraphNodeId::new(30)
        ));
    }
}
