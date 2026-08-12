//! Structurally typed graph documents over the bounded value registry.

use core::fmt;
use std::collections::BTreeSet;

use alumina_protocol::DeviceId;

use super::{
    GraphClockId, GraphSchema, GraphSchemaError, GraphTypeId, TypeKind, TypedGraphValue,
    device_id_is_zero, valid_stable_name,
};

/// Stable graph-local node identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphNodeId(u32);

impl GraphNodeId {
    /// Construct an identity. Zero is rejected by document validation.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identity.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable node-local port identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphPortId(u32);

impl GraphPortId {
    /// Construct an identity. Zero is rejected by document validation.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identity.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable graph-local wire identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphWireId(u32);

impl GraphWireId {
    /// Construct an identity. Zero is rejected by document validation.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identity.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Node execution ownership. This is placement intent, not proof that an
/// opaque node kind is admitted in the selected domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionDomain {
    /// Browser/native exact host executor.
    HostExact,
    /// Firmware service-core actor on one stable MCU.
    Service {
        /// Stable physical MCU identity.
        device_id: DeviceId,
    },
    /// Whitelisted firmware real-time actor on one stable MCU.
    Realtime {
        /// Stable physical MCU identity.
        device_id: DeviceId,
    },
}

impl ExecutionDomain {
    fn validate(self) -> Result<(), GraphDocumentError> {
        match self {
            Self::HostExact => Ok(()),
            Self::Service { device_id } | Self::Realtime { device_id }
                if !device_id_is_zero(device_id) =>
            {
                Ok(())
            }
            Self::Service { .. } | Self::Realtime { .. } => Err(GraphDocumentError::InvalidDomain),
        }
    }
}

/// Clock source and exact integer rate relationship. Distinct root clocks have
/// no static phase/offset relationship, even when their frequencies match.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockKind {
    /// Host monotonic counter at an explicit integer frequency.
    HostMonotonic {
        /// Counter ticks per SI second.
        ticks_per_second: u64,
    },
    /// One device's monotonic counter.
    DeviceCycle {
        /// Stable physical MCU identity.
        device_id: DeviceId,
        /// Counter ticks per SI second.
        ticks_per_second: u64,
    },
    /// Exact rational tick-rate derivation from another registered clock. Its
    /// tick zero is coincident with the source clock's tick zero.
    Derived {
        /// Source clock.
        source: GraphClockId,
        /// Derived ticks per `denominator` source ticks.
        numerator: u32,
        /// Source ticks corresponding to `numerator` derived ticks.
        denominator: u32,
    },
}

/// One stable named graph clock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockDefinition {
    id: GraphClockId,
    name: String,
    kind: ClockKind,
}

impl ClockDefinition {
    /// Construct a clock. Document construction validates and canonicalizes it.
    pub fn new(id: GraphClockId, name: impl Into<String>, kind: ClockKind) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
        }
    }

    /// Return the stable clock identity.
    pub const fn id(&self) -> GraphClockId {
        self.id
    }

    /// Return the stable clock name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact clock source description.
    pub const fn kind(&self) -> ClockKind {
        self.kind
    }
}

/// Opaque versioned node behavior identity. Unknown kinds remain structural
/// data until a later compiler registry admits them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeKind {
    name: String,
    version: u16,
}

impl NodeKind {
    /// Construct a namespaced behavior identity.
    pub fn new(name: impl Into<String>, version: u16) -> Self {
        Self {
            name: name.into(),
            version,
        }
    }

    /// Return the opaque stable behavior name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the behavior schema version.
    pub const fn version(&self) -> u16 {
        self.version
    }
}

/// One typed input or output port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortDefinition {
    id: GraphPortId,
    name: String,
    value_type: GraphTypeId,
}

impl PortDefinition {
    /// Construct a typed port. Node construction canonicalizes it by ID.
    pub fn new(id: GraphPortId, name: impl Into<String>, value_type: GraphTypeId) -> Self {
        Self {
            id,
            name: name.into(),
            value_type,
        }
    }

    /// Return the node-local port identity.
    pub const fn id(&self) -> GraphPortId {
        self.id
    }

    /// Return the stable port name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the registered value type.
    pub const fn value_type(&self) -> GraphTypeId {
        self.value_type
    }
}

/// One exact node parameter retained for known and unknown node kinds.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeParameter {
    id: u32,
    name: String,
    value: TypedGraphValue,
}

impl NodeParameter {
    /// Construct a parameter. Zero IDs and invalid names reject with the node.
    pub fn new(id: u32, name: impl Into<String>, value: TypedGraphValue) -> Self {
        Self {
            id,
            name: name.into(),
            value,
        }
    }

    /// Return the node-local stable parameter identity.
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Return the stable parameter name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the exact typed parameter value.
    pub const fn value(&self) -> &TypedGraphValue {
        &self.value
    }
}

/// One structurally described node.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDefinition {
    id: GraphNodeId,
    kind: NodeKind,
    label: String,
    domain: ExecutionDomain,
    inputs: Vec<PortDefinition>,
    outputs: Vec<PortDefinition>,
    parameters: Vec<NodeParameter>,
}

impl NodeDefinition {
    /// Construct a node; the complete document performs validation.
    #[allow(
        clippy::too_many_arguments,
        reason = "node identity, placement, ports, and retained parameters remain explicit"
    )]
    pub fn new(
        id: GraphNodeId,
        kind: NodeKind,
        label: impl Into<String>,
        domain: ExecutionDomain,
        inputs: Vec<PortDefinition>,
        outputs: Vec<PortDefinition>,
        parameters: Vec<NodeParameter>,
    ) -> Self {
        Self {
            id,
            kind,
            label: label.into(),
            domain,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Return the graph-local node identity.
    pub const fn id(&self) -> GraphNodeId {
        self.id
    }

    /// Return the opaque versioned behavior identity.
    pub const fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Return the user-facing label. It is never behavior identity.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Return requested execution ownership.
    pub const fn domain(&self) -> ExecutionDomain {
        self.domain
    }

    /// Borrow inputs in canonical local-ID order.
    pub fn inputs(&self) -> &[PortDefinition] {
        &self.inputs
    }

    /// Borrow outputs in canonical local-ID order.
    pub fn outputs(&self) -> &[PortDefinition] {
        &self.outputs
    }

    /// Borrow parameters in canonical local-ID order.
    pub fn parameters(&self) -> &[NodeParameter] {
        &self.parameters
    }
}

/// Node/port pair used by a wire endpoint.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WireEndpoint {
    /// Node identity.
    pub node: GraphNodeId,
    /// Node-local port identity.
    pub port: GraphPortId,
}

/// One output-to-input typed wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireDefinition {
    id: GraphWireId,
    source: WireEndpoint,
    target: WireEndpoint,
}

impl WireDefinition {
    /// Construct a wire. The complete document resolves both directions/types.
    pub const fn new(id: GraphWireId, source: WireEndpoint, target: WireEndpoint) -> Self {
        Self { id, source, target }
    }

    /// Return the graph-local wire identity.
    pub const fn id(self) -> GraphWireId {
        self.id
    }

    /// Return the required output endpoint.
    pub const fn source(self) -> WireEndpoint {
        self.source
    }

    /// Return the uniquely owned input endpoint.
    pub const fn target(self) -> WireEndpoint {
        self.target
    }
}

/// Canonical structural graph document. Node registry admission, domain
/// partitioning, WCET, state/cycle semantics, and resource claims remain later
/// compiler obligations.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphDocument {
    revision: u64,
    schema: GraphSchema,
    clocks: Vec<ClockDefinition>,
    nodes: Vec<NodeDefinition>,
    wires: Vec<WireDefinition>,
}

impl GraphDocument {
    /// Canonicalize and validate one complete structural document.
    pub fn try_new(
        revision: u64,
        schema: GraphSchema,
        mut clocks: Vec<ClockDefinition>,
        mut nodes: Vec<NodeDefinition>,
        mut wires: Vec<WireDefinition>,
    ) -> Result<Self, GraphDocumentError> {
        let limits = schema.limits();
        validate_count(clocks.len(), limits.maximum_clocks, "clock count")?;
        validate_count(nodes.len(), limits.maximum_nodes, "node count")?;
        validate_count(wires.len(), limits.maximum_wires, "wire count")?;
        clocks.sort_unstable_by_key(ClockDefinition::id);
        nodes.sort_unstable_by_key(NodeDefinition::id);
        wires.sort_unstable_by_key(|wire| wire.id());
        for node in &mut nodes {
            node.inputs.sort_unstable_by_key(PortDefinition::id);
            node.outputs.sort_unstable_by_key(PortDefinition::id);
            node.parameters.sort_unstable_by_key(NodeParameter::id);
        }
        let document = Self {
            revision,
            schema,
            clocks,
            nodes,
            wires,
        };
        document.validate_clocks()?;
        document.validate_type_clocks()?;
        document.validate_nodes()?;
        document.validate_wires()?;
        Ok(document)
    }

    /// Return monotonic editor/document revision metadata.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Borrow the exact value/type registry.
    pub const fn schema(&self) -> &GraphSchema {
        &self.schema
    }

    /// Borrow clocks in canonical ID order.
    pub fn clocks(&self) -> &[ClockDefinition] {
        &self.clocks
    }

    /// Borrow nodes in canonical ID order.
    pub fn nodes(&self) -> &[NodeDefinition] {
        &self.nodes
    }

    /// Borrow wires in canonical ID order.
    pub fn wires(&self) -> &[WireDefinition] {
        &self.wires
    }

    /// Resolve one graph clock.
    pub fn clock(&self, id: GraphClockId) -> Option<&ClockDefinition> {
        self.clock_index(id).map(|index| &self.clocks[index])
    }

    /// Resolve one graph node.
    pub fn node(&self, id: GraphNodeId) -> Option<&NodeDefinition> {
        self.node_index(id).map(|index| &self.nodes[index])
    }

    fn validate_clocks(&self) -> Result<(), GraphDocumentError> {
        let mut names = BTreeSet::new();
        let mut previous = None;
        for clock in &self.clocks {
            if clock.id.get() == 0 {
                return Err(GraphDocumentError::ZeroIdentifier("clock"));
            }
            if previous == Some(clock.id) {
                return Err(GraphDocumentError::DuplicateIdentifier("clock"));
            }
            previous = Some(clock.id);
            if !valid_stable_name(&clock.name) {
                return Err(GraphDocumentError::InvalidName("clock"));
            }
            if !names.insert(clock.name.as_str()) {
                return Err(GraphDocumentError::DuplicateName("clock"));
            }
            match clock.kind {
                ClockKind::HostMonotonic { ticks_per_second } => {
                    if ticks_per_second == 0 {
                        return Err(GraphDocumentError::InvalidClockRate(clock.id));
                    }
                }
                ClockKind::DeviceCycle {
                    device_id,
                    ticks_per_second,
                } => {
                    if device_id_is_zero(device_id) || ticks_per_second == 0 {
                        return Err(GraphDocumentError::InvalidClockRate(clock.id));
                    }
                }
                ClockKind::Derived {
                    source,
                    numerator,
                    denominator,
                } => {
                    if source.get() == 0
                        || numerator == 0
                        || denominator == 0
                        || gcd_u32(numerator, denominator) != 1
                    {
                        return Err(GraphDocumentError::InvalidClockRate(clock.id));
                    }
                    self.require_clock(source)?;
                }
            }
        }
        let mut states = vec![0_u8; self.clocks.len()];
        for index in 0..self.clocks.len() {
            self.visit_clock(index, &mut states)?;
        }
        Ok(())
    }

    fn visit_clock(&self, index: usize, states: &mut [u8]) -> Result<(), GraphDocumentError> {
        match states[index] {
            2 => return Ok(()),
            1 => return Err(GraphDocumentError::RecursiveClock(self.clocks[index].id)),
            _ => {}
        }
        states[index] = 1;
        if let ClockKind::Derived { source, .. } = self.clocks[index].kind {
            let source = self.require_clock(source)?;
            self.visit_clock(source, states)?;
        }
        states[index] = 2;
        Ok(())
    }

    fn validate_type_clocks(&self) -> Result<(), GraphDocumentError> {
        for definition in self.schema.types() {
            let clock = match definition.kind() {
                TypeKind::Event { clock, .. } | TypeKind::Stream { clock, .. } => Some(*clock),
                _ => None,
            };
            if let Some(clock) = clock {
                self.require_clock(clock)?;
            }
        }
        Ok(())
    }

    fn validate_nodes(&self) -> Result<(), GraphDocumentError> {
        let limits = self.schema.limits();
        let mut previous = None;
        for node in &self.nodes {
            if node.id.get() == 0 {
                return Err(GraphDocumentError::ZeroIdentifier("node"));
            }
            if previous == Some(node.id) {
                return Err(GraphDocumentError::DuplicateIdentifier("node"));
            }
            previous = Some(node.id);
            if !valid_stable_name(&node.kind.name) || node.kind.version == 0 {
                return Err(GraphDocumentError::InvalidName("node kind"));
            }
            if !valid_label(&node.label, limits.maximum_label_bytes) {
                return Err(GraphDocumentError::InvalidName("node label"));
            }
            node.domain.validate()?;
            let port_count = node
                .inputs
                .len()
                .checked_add(node.outputs.len())
                .ok_or(GraphDocumentError::LimitExceeded("node port count"))?;
            validate_count(port_count, limits.maximum_ports_per_node, "node port count")?;
            validate_count(
                node.parameters.len(),
                limits.maximum_parameters_per_node,
                "node parameter count",
            )?;
            let mut port_ids = BTreeSet::new();
            let mut port_names = BTreeSet::new();
            for port in node.inputs.iter().chain(&node.outputs) {
                if port.id.get() == 0 {
                    return Err(GraphDocumentError::ZeroIdentifier("port"));
                }
                if !port_ids.insert(port.id) {
                    return Err(GraphDocumentError::DuplicateIdentifier("port"));
                }
                if !valid_stable_name(&port.name) {
                    return Err(GraphDocumentError::InvalidName("port"));
                }
                if !port_names.insert(port.name.as_str()) {
                    return Err(GraphDocumentError::DuplicateName("port"));
                }
                if self.schema.value_type(port.value_type).is_none() {
                    return Err(GraphSchemaError::UnknownType(port.value_type).into());
                }
            }
            let mut parameter_names = BTreeSet::new();
            let mut prior_parameter = None;
            for parameter in &node.parameters {
                if parameter.id == 0 {
                    return Err(GraphDocumentError::ZeroIdentifier("parameter"));
                }
                if prior_parameter == Some(parameter.id) {
                    return Err(GraphDocumentError::DuplicateIdentifier("parameter"));
                }
                prior_parameter = Some(parameter.id);
                if !valid_stable_name(&parameter.name) {
                    return Err(GraphDocumentError::InvalidName("parameter"));
                }
                if !parameter_names.insert(parameter.name.as_str()) {
                    return Err(GraphDocumentError::DuplicateName("parameter"));
                }
                self.schema.validate_typed_value(&parameter.value)?;
            }
        }
        Ok(())
    }

    fn validate_wires(&self) -> Result<(), GraphDocumentError> {
        let mut previous = None;
        let mut targets = BTreeSet::new();
        let mut endpoint_pairs = BTreeSet::new();
        for wire in &self.wires {
            if wire.id.get() == 0 {
                return Err(GraphDocumentError::ZeroIdentifier("wire"));
            }
            if previous == Some(wire.id) {
                return Err(GraphDocumentError::DuplicateIdentifier("wire"));
            }
            previous = Some(wire.id);
            let source_node = self
                .node(wire.source.node)
                .ok_or(GraphDocumentError::UnknownNode(wire.source.node))?;
            let target_node = self
                .node(wire.target.node)
                .ok_or(GraphDocumentError::UnknownNode(wire.target.node))?;
            let source = find_port(&source_node.outputs, wire.source.port)
                .ok_or(GraphDocumentError::UnknownOutput(wire.source))?;
            let target = find_port(&target_node.inputs, wire.target.port)
                .ok_or(GraphDocumentError::UnknownInput(wire.target))?;
            if source.value_type != target.value_type {
                return Err(GraphDocumentError::WireTypeMismatch {
                    source: source.value_type,
                    target: target.value_type,
                });
            }
            if !endpoint_pairs.insert((wire.source, wire.target)) {
                return Err(GraphDocumentError::DuplicateEndpoints);
            }
            if !targets.insert(wire.target) {
                return Err(GraphDocumentError::InputAlreadyConnected(wire.target));
            }
        }
        Ok(())
    }

    fn require_clock(&self, id: GraphClockId) -> Result<usize, GraphDocumentError> {
        self.clock_index(id)
            .ok_or(GraphDocumentError::UnknownClock(id))
    }

    fn clock_index(&self, id: GraphClockId) -> Option<usize> {
        self.clocks
            .binary_search_by_key(&id, ClockDefinition::id)
            .ok()
    }

    fn node_index(&self, id: GraphNodeId) -> Option<usize> {
        self.nodes
            .binary_search_by_key(&id, NodeDefinition::id)
            .ok()
    }
}

fn find_port(ports: &[PortDefinition], id: GraphPortId) -> Option<&PortDefinition> {
    ports
        .binary_search_by_key(&id, PortDefinition::id)
        .ok()
        .map(|index| &ports[index])
}

fn validate_count(
    received: usize,
    maximum: usize,
    name: &'static str,
) -> Result<(), GraphDocumentError> {
    if received > maximum {
        Err(GraphDocumentError::LimitExceeded(name))
    } else {
        Ok(())
    }
}

fn valid_label(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.chars().all(|character| !character.is_control())
}

const fn gcd_u32(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// Rejection at the structural graph-document boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphDocumentError {
    /// Value/type registry rejection.
    Schema(GraphSchemaError),
    /// A graph collection exceeded its explicit policy.
    LimitExceeded(&'static str),
    /// A structural identifier was zero.
    ZeroIdentifier(&'static str),
    /// A structural identifier was duplicated.
    DuplicateIdentifier(&'static str),
    /// A stable structural name was duplicated.
    DuplicateName(&'static str),
    /// A stable name, opaque kind, or display label was malformed.
    InvalidName(&'static str),
    /// A service/realtime placement omitted physical device identity.
    InvalidDomain,
    /// A clock reference was absent.
    UnknownClock(GraphClockId),
    /// A clock rate/ratio/device identity was invalid or noncanonical.
    InvalidClockRate(GraphClockId),
    /// Derived clocks formed a cycle.
    RecursiveClock(GraphClockId),
    /// A wire referenced an absent node.
    UnknownNode(GraphNodeId),
    /// A wire source was not a declared output.
    UnknownOutput(WireEndpoint),
    /// A wire target was not a declared input.
    UnknownInput(WireEndpoint),
    /// Connected ports had distinct registered types.
    WireTypeMismatch {
        /// Output type.
        source: GraphTypeId,
        /// Input type.
        target: GraphTypeId,
    },
    /// More than one wire attempted to own an input.
    InputAlreadyConnected(WireEndpoint),
    /// Two wire identities represented the same endpoint pair.
    DuplicateEndpoints,
}

impl From<GraphSchemaError> for GraphDocumentError {
    fn from(value: GraphSchemaError) -> Self {
        Self::Schema(value)
    }
}

impl fmt::Display for GraphDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(error) => write!(formatter, "graph schema rejected: {error}"),
            Self::LimitExceeded(name) => write!(formatter, "graph {name} exceeds policy"),
            Self::ZeroIdentifier(kind) => write!(formatter, "graph {kind} identity is zero"),
            Self::DuplicateIdentifier(kind) => {
                write!(formatter, "graph {kind} identity is duplicated")
            }
            Self::DuplicateName(kind) => write!(formatter, "graph {kind} name is duplicated"),
            Self::InvalidName(kind) => write!(formatter, "graph {kind} name is invalid"),
            Self::InvalidDomain => formatter.write_str("graph execution domain has no device"),
            Self::UnknownClock(id) => write!(formatter, "graph clock {id:?} is unknown"),
            Self::InvalidClockRate(id) => write!(formatter, "graph clock {id:?} rate is invalid"),
            Self::RecursiveClock(id) => write!(formatter, "graph clock {id:?} is recursive"),
            Self::UnknownNode(id) => write!(formatter, "graph node {id:?} is unknown"),
            Self::UnknownOutput(endpoint) => {
                write!(formatter, "graph output {endpoint:?} is unknown")
            }
            Self::UnknownInput(endpoint) => {
                write!(formatter, "graph input {endpoint:?} is unknown")
            }
            Self::WireTypeMismatch { source, target } => {
                write!(
                    formatter,
                    "graph wire type {source:?} does not match {target:?}"
                )
            }
            Self::InputAlreadyConnected(endpoint) => {
                write!(formatter, "graph input {endpoint:?} already has a source")
            }
            Self::DuplicateEndpoints => formatter.write_str("graph wire endpoints are duplicated"),
        }
    }
}

impl std::error::Error for GraphDocumentError {}

#[cfg(test)]
mod tests {
    use hyperreal::Rational;

    use super::*;
    use crate::graph::{
        BaseDimensions, GraphLimits, GraphValue, TypeDefinition, UnitDefinition, UnitId,
    };

    const BOOL: GraphTypeId = GraphTypeId::new(1);
    const EXACT: GraphTypeId = GraphTypeId::new(2);
    const EVENT: GraphTypeId = GraphTypeId::new(3);
    const CLOCK: GraphClockId = GraphClockId::new(1);

    fn schema() -> GraphSchema {
        GraphSchema::try_new(
            GraphLimits::interactive(),
            vec![UnitDefinition::new(
                UnitId::new(1),
                "mm",
                BaseDimensions::LENGTH,
                Rational::fraction(1, 1_000).unwrap(),
            )],
            vec![
                TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    EXACT,
                    "exact.mm",
                    TypeKind::ExactRational {
                        unit: UnitId::new(1),
                    },
                ),
                TypeDefinition::new(
                    EVENT,
                    "event.bool",
                    TypeKind::Event {
                        payload: BOOL,
                        clock: CLOCK,
                    },
                ),
            ],
        )
        .unwrap()
    }

    fn clock() -> ClockDefinition {
        ClockDefinition::new(
            CLOCK,
            "host.monotonic",
            ClockKind::HostMonotonic {
                ticks_per_second: 1_000_000_000,
            },
        )
    }

    fn node(
        id: u32,
        kind: &str,
        inputs: Vec<PortDefinition>,
        outputs: Vec<PortDefinition>,
    ) -> NodeDefinition {
        NodeDefinition::new(
            GraphNodeId::new(id),
            NodeKind::new(kind, 1),
            format!("node {id}"),
            ExecutionDomain::HostExact,
            inputs,
            outputs,
            Vec::new(),
        )
    }

    fn endpoint(node: u32, port: u32) -> WireEndpoint {
        WireEndpoint {
            node: GraphNodeId::new(node),
            port: GraphPortId::new(port),
        }
    }

    #[test]
    fn document_canonicalizes_unknown_nodes_and_connects_exact_types() {
        let source = node(
            2,
            "unknown.vendor.source",
            Vec::new(),
            vec![PortDefinition::new(GraphPortId::new(1), "out", EXACT)],
        );
        let target = node(
            1,
            "org.alumina.sink",
            vec![PortDefinition::new(GraphPortId::new(2), "in", EXACT)],
            Vec::new(),
        );
        let document = GraphDocument::try_new(
            7,
            schema(),
            vec![clock()],
            vec![source, target],
            vec![WireDefinition::new(
                GraphWireId::new(1),
                endpoint(2, 1),
                endpoint(1, 2),
            )],
        )
        .unwrap();
        assert_eq!(document.revision(), 7);
        assert_eq!(document.nodes()[0].id(), GraphNodeId::new(1));
        assert_eq!(document.nodes()[1].kind().name(), "unknown.vendor.source");
        assert_eq!(document.wires()[0].source(), endpoint(2, 1));
    }

    #[test]
    fn clocks_are_explicit_reduced_and_acyclic() {
        assert_eq!(
            GraphDocument::try_new(0, schema(), Vec::new(), Vec::new(), Vec::new()),
            Err(GraphDocumentError::UnknownClock(CLOCK))
        );

        let nonreduced = ClockDefinition::new(
            GraphClockId::new(2),
            "derived",
            ClockKind::Derived {
                source: CLOCK,
                numerator: 2,
                denominator: 2,
            },
        );
        assert_eq!(
            GraphDocument::try_new(
                0,
                schema(),
                vec![clock(), nonreduced],
                Vec::new(),
                Vec::new()
            ),
            Err(GraphDocumentError::InvalidClockRate(GraphClockId::new(2)))
        );

        let cycle_schema = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean)],
        )
        .unwrap();
        let left = ClockDefinition::new(
            CLOCK,
            "left",
            ClockKind::Derived {
                source: GraphClockId::new(2),
                numerator: 1,
                denominator: 1,
            },
        );
        let right = ClockDefinition::new(
            GraphClockId::new(2),
            "right",
            ClockKind::Derived {
                source: CLOCK,
                numerator: 1,
                denominator: 1,
            },
        );
        assert_eq!(
            GraphDocument::try_new(0, cycle_schema, vec![left, right], Vec::new(), Vec::new()),
            Err(GraphDocumentError::RecursiveClock(CLOCK))
        );
    }

    #[test]
    fn wire_direction_type_and_single_input_ownership_fail_closed() {
        let source = node(
            1,
            "source",
            Vec::new(),
            vec![PortDefinition::new(GraphPortId::new(1), "out", EXACT)],
        );
        let wrong_target = node(
            2,
            "target",
            vec![PortDefinition::new(GraphPortId::new(1), "in", BOOL)],
            Vec::new(),
        );
        assert_eq!(
            GraphDocument::try_new(
                0,
                schema(),
                vec![clock()],
                vec![source.clone(), wrong_target],
                vec![WireDefinition::new(
                    GraphWireId::new(1),
                    endpoint(1, 1),
                    endpoint(2, 1)
                )]
            ),
            Err(GraphDocumentError::WireTypeMismatch {
                source: EXACT,
                target: BOOL,
            })
        );

        let target = node(
            2,
            "target",
            vec![PortDefinition::new(GraphPortId::new(1), "in", EXACT)],
            Vec::new(),
        );
        let other_source = node(
            3,
            "other-source",
            Vec::new(),
            vec![PortDefinition::new(GraphPortId::new(1), "out", EXACT)],
        );
        assert_eq!(
            GraphDocument::try_new(
                0,
                schema(),
                vec![clock()],
                vec![source, target, other_source],
                vec![
                    WireDefinition::new(GraphWireId::new(1), endpoint(1, 1), endpoint(2, 1)),
                    WireDefinition::new(GraphWireId::new(2), endpoint(3, 1), endpoint(2, 1)),
                ]
            ),
            Err(GraphDocumentError::InputAlreadyConnected(endpoint(2, 1)))
        );
    }

    #[test]
    fn node_domains_ports_and_parameters_replay_schema_authority() {
        let parameter = NodeParameter::new(
            1,
            "initial",
            TypedGraphValue::try_new(
                &schema(),
                EXACT,
                GraphValue::ExactRational(Rational::zero()),
            )
            .unwrap(),
        );
        let invalid_domain = NodeDefinition::new(
            GraphNodeId::new(1),
            NodeKind::new("device.node", 1),
            "device node",
            ExecutionDomain::Realtime {
                device_id: DeviceId([0; 16]),
            },
            Vec::new(),
            Vec::new(),
            vec![parameter],
        );
        assert_eq!(
            GraphDocument::try_new(0, schema(), vec![clock()], vec![invalid_domain], Vec::new()),
            Err(GraphDocumentError::InvalidDomain)
        );

        let duplicate_ports = node(
            1,
            "duplicate.ports",
            vec![PortDefinition::new(GraphPortId::new(1), "in", BOOL)],
            vec![PortDefinition::new(GraphPortId::new(1), "out", BOOL)],
        );
        assert_eq!(
            GraphDocument::try_new(
                0,
                schema(),
                vec![clock()],
                vec![duplicate_ports],
                Vec::new()
            ),
            Err(GraphDocumentError::DuplicateIdentifier("port"))
        );
    }
}
