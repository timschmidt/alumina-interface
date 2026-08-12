//! Audited node-schema admission and bounded combinational-cycle analysis.
//!
//! A structural [`super::GraphDocument`] deliberately preserves opaque node
//! kinds. This module is the separate semantic boundary: a node becomes known
//! only through an exact registry entry that declares its full shape, allowed
//! execution domains, per-output current-tick feedthrough, and optional
//! read-before-write state boundary. Admission here remains host-side analysis;
//! it emits no firmware opcode and grants no real-time authority.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use super::{
    ClockDefinition, ExecutionDomain, GraphClockId, GraphDocument, GraphNodeId, GraphPortId,
    GraphSchema, GraphTypeId, GraphWireId, NodeDefinition, NodeKind, PortDefinition, WireEndpoint,
    valid_stable_name,
};

/// Set of execution-domain families admitted by one audited node schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ExecutionDomainSet(u8);

impl ExecutionDomainSet {
    const HOST_BIT: u8 = 1 << 0;
    const SERVICE_BIT: u8 = 1 << 1;
    const REALTIME_BIT: u8 = 1 << 2;

    /// Host exact execution only.
    pub const HOST_EXACT: Self = Self(Self::HOST_BIT);
    /// Device service-core execution only.
    pub const SERVICE: Self = Self(Self::SERVICE_BIT);
    /// Whitelisted device real-time execution only.
    pub const REALTIME: Self = Self(Self::REALTIME_BIT);
    /// Host and service execution.
    pub const HOST_AND_SERVICE: Self = Self(Self::HOST_BIT | Self::SERVICE_BIT);
    /// All three domain families. This still grants no opcode authority.
    pub const ALL: Self = Self(Self::HOST_BIT | Self::SERVICE_BIT | Self::REALTIME_BIT);

    /// Construct a set from three explicit family choices.
    pub const fn new(host_exact: bool, service: bool, realtime: bool) -> Self {
        let mut bits = 0;
        if host_exact {
            bits |= Self::HOST_BIT;
        }
        if service {
            bits |= Self::SERVICE_BIT;
        }
        if realtime {
            bits |= Self::REALTIME_BIT;
        }
        Self(bits)
    }

    /// Return whether one concrete placement belongs to the set.
    pub const fn contains(self, domain: ExecutionDomain) -> bool {
        let bit = match domain {
            ExecutionDomain::HostExact => Self::HOST_BIT,
            ExecutionDomain::Service { .. } => Self::SERVICE_BIT,
            ExecutionDomain::Realtime { .. } => Self::REALTIME_BIT,
        };
        self.0 & bit != 0
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Bounded semantic-registry and analysis policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphAnalysisLimits {
    /// Maximum registered audited node kinds.
    pub maximum_registered_kinds: usize,
    /// Maximum combined wire and internal feedthrough links.
    pub maximum_dependency_links: usize,
    /// Maximum links retained in one exact cycle witness.
    pub maximum_cycle_witness_links: usize,
    /// Maximum declared state bytes for one node instance.
    pub maximum_state_bytes_per_node: usize,
    /// Maximum combined declared state bytes in one graph.
    pub maximum_total_state_bytes: usize,
}

impl GraphAnalysisLimits {
    /// First-release bounded host analysis policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_registered_kinds: 1_024,
            maximum_dependency_links: 65_536,
            maximum_cycle_witness_links: 4_096,
            maximum_state_bytes_per_node: 1024 * 1024,
            maximum_total_state_bytes: 16 * 1024 * 1024,
        }
    }

    fn validate(self) -> Result<(), NodeRegistryError> {
        if [
            self.maximum_registered_kinds,
            self.maximum_dependency_links,
            self.maximum_cycle_witness_links,
            self.maximum_state_bytes_per_node,
            self.maximum_total_state_bytes,
        ]
        .contains(&0)
        {
            Err(NodeRegistryError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphAnalysisLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Required node-local parameter identity and registered type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeParameterContract {
    id: u32,
    name: String,
    value_type: GraphTypeId,
}

impl NodeParameterContract {
    /// Construct a required parameter contract.
    pub fn new(id: u32, name: impl Into<String>, value_type: GraphTypeId) -> Self {
        Self {
            id,
            name: name.into(),
            value_type,
        }
    }

    /// Return the node-local parameter identity.
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Return the stable parameter name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact registered value type.
    pub const fn value_type(&self) -> GraphTypeId {
        self.value_type
    }
}

/// Complete current-tick input dependency set for one output.
///
/// Every declared output must have exactly one such entry. An empty input list
/// means that output is a source or prior-state value and therefore does not
/// propagate a current-tick combinational dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeOutputDependency {
    output: GraphPortId,
    inputs: Vec<GraphPortId>,
}

impl NodeOutputDependency {
    /// Construct one explicit output feedthrough declaration.
    pub fn new(output: GraphPortId, inputs: Vec<GraphPortId>) -> Self {
        Self { output, inputs }
    }

    /// Return the output port.
    pub const fn output(&self) -> GraphPortId {
        self.output
    }

    /// Borrow current-tick input dependencies in canonical port-ID order.
    pub fn inputs(&self) -> &[GraphPortId] {
        &self.inputs
    }
}

/// Explicit read-before-write state boundary for one node kind.
///
/// `current_output` exposes the state captured before the named clock update;
/// `next_input` supplies the value captured after current-tick combinational
/// evaluation. `initial_parameter` supplies deterministic run-start state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeStateContract {
    clock: GraphClockId,
    value_type: GraphTypeId,
    initial_parameter: u32,
    next_input: GraphPortId,
    current_output: GraphPortId,
    declared_storage_bytes: u32,
}

impl NodeStateContract {
    /// Construct a read-before-write state declaration.
    pub const fn new(
        clock: GraphClockId,
        value_type: GraphTypeId,
        initial_parameter: u32,
        next_input: GraphPortId,
        current_output: GraphPortId,
        declared_storage_bytes: u32,
    ) -> Self {
        Self {
            clock,
            value_type,
            initial_parameter,
            next_input,
            current_output,
            declared_storage_bytes,
        }
    }

    /// Return the state update clock.
    pub const fn clock(self) -> GraphClockId {
        self.clock
    }

    /// Return the registered state value type.
    pub const fn value_type(self) -> GraphTypeId {
        self.value_type
    }

    /// Return the required run-start parameter identity.
    pub const fn initial_parameter(self) -> u32 {
        self.initial_parameter
    }

    /// Return the next-state input port.
    pub const fn next_input(self) -> GraphPortId {
        self.next_input
    }

    /// Return the prior/current-state output port.
    pub const fn current_output(self) -> GraphPortId {
        self.current_output
    }

    /// Return the declared storage ceiling. Static type-size proof is later work.
    pub const fn declared_storage_bytes(self) -> u32 {
        self.declared_storage_bytes
    }
}

/// One audited semantic schema for an otherwise opaque node kind.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSchema {
    kind: NodeKind,
    allowed_domains: ExecutionDomainSet,
    inputs: Vec<PortDefinition>,
    outputs: Vec<PortDefinition>,
    parameters: Vec<NodeParameterContract>,
    output_dependencies: Vec<NodeOutputDependency>,
    state: Option<NodeStateContract>,
}

impl NodeSchema {
    /// Construct a semantic schema. Registry construction canonicalizes and validates it.
    #[allow(
        clippy::too_many_arguments,
        reason = "node shape, placement, feedthrough, and state authority remain explicit"
    )]
    pub fn new(
        kind: NodeKind,
        allowed_domains: ExecutionDomainSet,
        inputs: Vec<PortDefinition>,
        outputs: Vec<PortDefinition>,
        parameters: Vec<NodeParameterContract>,
        output_dependencies: Vec<NodeOutputDependency>,
        state: Option<NodeStateContract>,
    ) -> Self {
        Self {
            kind,
            allowed_domains,
            inputs,
            outputs,
            parameters,
            output_dependencies,
            state,
        }
    }

    /// Return the opaque kind/version this schema resolves.
    pub const fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Return allowed execution-domain families.
    pub const fn allowed_domains(&self) -> ExecutionDomainSet {
        self.allowed_domains
    }

    /// Borrow exact required inputs in canonical local-ID order.
    pub fn inputs(&self) -> &[PortDefinition] {
        &self.inputs
    }

    /// Borrow exact required outputs in canonical local-ID order.
    pub fn outputs(&self) -> &[PortDefinition] {
        &self.outputs
    }

    /// Borrow exact required parameters in canonical local-ID order.
    pub fn parameters(&self) -> &[NodeParameterContract] {
        &self.parameters
    }

    /// Borrow complete per-output feedthrough declarations.
    pub fn output_dependencies(&self) -> &[NodeOutputDependency] {
        &self.output_dependencies
    }

    /// Return the optional explicit state boundary.
    pub const fn state(&self) -> Option<NodeStateContract> {
        self.state
    }
}

/// Canonical audited-node registry. Presence here resolves shape only; later
/// compilers still require an implementation/opcode registry and static proof.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNodeRegistry {
    limits: GraphAnalysisLimits,
    context_schema: GraphSchema,
    context_clocks: Vec<ClockDefinition>,
    schemas: Vec<NodeSchema>,
}

impl GraphNodeRegistry {
    /// Canonicalize and validate one complete semantic schema registry.
    pub fn try_new(
        limits: GraphAnalysisLimits,
        context: &GraphDocument,
        mut schemas: Vec<NodeSchema>,
    ) -> Result<Self, NodeRegistryError> {
        limits.validate()?;
        if schemas.len() > limits.maximum_registered_kinds {
            return Err(NodeRegistryError::LimitExceeded("registered kind count"));
        }
        schemas.sort_unstable_by(compare_schema_kind);
        for schema in &mut schemas {
            schema.inputs.sort_unstable_by_key(PortDefinition::id);
            schema.outputs.sort_unstable_by_key(PortDefinition::id);
            schema
                .parameters
                .sort_unstable_by_key(NodeParameterContract::id);
            schema
                .output_dependencies
                .sort_unstable_by_key(NodeOutputDependency::output);
            for dependency in &mut schema.output_dependencies {
                dependency.inputs.sort_unstable();
            }
        }
        let registry = Self {
            limits,
            context_schema: context.schema().clone(),
            context_clocks: context.clocks().to_vec(),
            schemas,
        };
        registry.validate()?;
        Ok(registry)
    }

    /// Return the retained bounded analysis policy.
    pub const fn limits(&self) -> GraphAnalysisLimits {
        self.limits
    }

    /// Borrow schemas in canonical kind-name/version order.
    pub fn schemas(&self) -> &[NodeSchema] {
        &self.schemas
    }

    /// Borrow the exact unit/type registry this semantic authority resolves.
    pub const fn context_schema(&self) -> &GraphSchema {
        &self.context_schema
    }

    /// Borrow the exact clock set this semantic authority resolves.
    pub fn context_clocks(&self) -> &[ClockDefinition] {
        &self.context_clocks
    }

    /// Resolve one exact opaque kind/version.
    pub fn schema(&self, kind: &NodeKind) -> Option<&NodeSchema> {
        self.schemas
            .binary_search_by(|schema| compare_kind(schema.kind(), kind))
            .ok()
            .map(|index| &self.schemas[index])
    }

    fn validate(&self) -> Result<(), NodeRegistryError> {
        let mut previous: Option<&NodeKind> = None;
        for schema in &self.schemas {
            if let Some(previous) = previous
                && compare_kind(previous, &schema.kind).is_eq()
            {
                return Err(NodeRegistryError::DuplicateKind(schema.kind.clone()));
            }
            previous = Some(&schema.kind);
            validate_node_schema(
                schema,
                self.limits,
                &self.context_schema,
                &self.context_clocks,
            )?;
        }
        Ok(())
    }
}

fn compare_schema_kind(left: &NodeSchema, right: &NodeSchema) -> core::cmp::Ordering {
    compare_kind(left.kind(), right.kind())
}

fn compare_kind(left: &NodeKind, right: &NodeKind) -> core::cmp::Ordering {
    left.name()
        .cmp(right.name())
        .then_with(|| left.version().cmp(&right.version()))
}

fn validate_node_schema(
    schema: &NodeSchema,
    limits: GraphAnalysisLimits,
    context_schema: &GraphSchema,
    context_clocks: &[ClockDefinition],
) -> Result<(), NodeRegistryError> {
    if !valid_stable_name(schema.kind.name()) || schema.kind.version() == 0 {
        return Err(invalid_schema(schema, "kind identity"));
    }
    if schema.allowed_domains.is_empty() {
        return Err(invalid_schema(schema, "allowed domains"));
    }
    let mut port_ids = BTreeSet::new();
    let mut port_names = BTreeSet::new();
    for port in schema.inputs.iter().chain(&schema.outputs) {
        if port.id().get() == 0
            || port.value_type().get() == 0
            || !valid_stable_name(port.name())
            || !port_ids.insert(port.id())
            || !port_names.insert(port.name())
        {
            return Err(invalid_schema(schema, "port contract"));
        }
        if context_schema.value_type(port.value_type()).is_none() {
            return Err(invalid_schema(schema, "port value type"));
        }
    }
    let mut parameter_names = BTreeSet::new();
    let mut previous_parameter = None;
    for parameter in &schema.parameters {
        if parameter.id == 0
            || parameter.value_type.get() == 0
            || previous_parameter == Some(parameter.id)
            || !valid_stable_name(&parameter.name)
            || !parameter_names.insert(parameter.name.as_str())
        {
            return Err(invalid_schema(schema, "parameter contract"));
        }
        if context_schema.value_type(parameter.value_type).is_none() {
            return Err(invalid_schema(schema, "parameter value type"));
        }
        previous_parameter = Some(parameter.id);
    }
    if schema.output_dependencies.len() != schema.outputs.len() {
        return Err(invalid_schema(schema, "output dependency coverage"));
    }
    let input_ids: BTreeSet<_> = schema.inputs.iter().map(PortDefinition::id).collect();
    for (output, dependency) in schema.outputs.iter().zip(&schema.output_dependencies) {
        if output.id() != dependency.output {
            return Err(invalid_schema(schema, "output dependency coverage"));
        }
        let mut previous_input = None;
        for input in &dependency.inputs {
            if previous_input == Some(*input) || !input_ids.contains(input) {
                return Err(invalid_schema(schema, "output dependency input"));
            }
            previous_input = Some(*input);
        }
    }
    if let Some(state) = schema.state {
        let storage = usize::try_from(state.declared_storage_bytes)
            .map_err(|_| invalid_schema(schema, "state storage"))?;
        if state.clock.get() == 0
            || state.value_type.get() == 0
            || state.initial_parameter == 0
            || storage == 0
            || storage > limits.maximum_state_bytes_per_node
        {
            return Err(invalid_schema(schema, "state contract"));
        }
        if context_schema.value_type(state.value_type).is_none() {
            return Err(invalid_schema(schema, "state value type"));
        }
        if !context_clocks.iter().any(|clock| clock.id() == state.clock) {
            return Err(invalid_schema(schema, "state clock"));
        }
        let initial = schema
            .parameters
            .iter()
            .find(|parameter| parameter.id == state.initial_parameter);
        let next = schema
            .inputs
            .iter()
            .find(|port| port.id() == state.next_input);
        let current = schema
            .outputs
            .iter()
            .find(|port| port.id() == state.current_output);
        if initial.map(NodeParameterContract::value_type) != Some(state.value_type)
            || next.map(PortDefinition::value_type) != Some(state.value_type)
            || current.map(PortDefinition::value_type) != Some(state.value_type)
        {
            return Err(invalid_schema(schema, "state value path"));
        }
        let current_dependency = schema
            .output_dependencies
            .iter()
            .find(|dependency| dependency.output == state.current_output);
        if !matches!(current_dependency, Some(dependency) if dependency.inputs.is_empty()) {
            return Err(invalid_schema(schema, "state output feedthrough"));
        }
    }
    Ok(())
}

fn invalid_schema(schema: &NodeSchema, aspect: &'static str) -> NodeRegistryError {
    NodeRegistryError::InvalidSchema {
        kind: schema.kind.clone(),
        aspect,
    }
}

/// Failure while constructing an audited node-schema registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NodeRegistryError {
    /// One semantic analysis limit was zero.
    ZeroLimit,
    /// A registry collection exceeded policy.
    LimitExceeded(&'static str),
    /// The same opaque kind/version had multiple authorities.
    DuplicateKind(NodeKind),
    /// One schema was internally inconsistent.
    InvalidSchema {
        /// Rejected opaque kind/version.
        kind: NodeKind,
        /// Exact schema aspect that failed.
        aspect: &'static str,
    },
}

impl fmt::Display for NodeRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph analysis limit is zero"),
            Self::LimitExceeded(name) => write!(formatter, "graph registry {name} exceeds policy"),
            Self::DuplicateKind(kind) => {
                write!(
                    formatter,
                    "node kind {}@{} is duplicated",
                    kind.name(),
                    kind.version()
                )
            }
            Self::InvalidSchema { kind, aspect } => write!(
                formatter,
                "node schema {}@{} has invalid {aspect}",
                kind.name(),
                kind.version()
            ),
        }
    }
}

impl std::error::Error for NodeRegistryError {}

/// One link in an exact combinational-cycle witness.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyLink {
    /// A structural document wire from an output to an input.
    Wire(GraphWireId),
    /// One audited current-tick input-to-output feedthrough.
    Feedthrough {
        /// Node carrying the internal dependency.
        node: GraphNodeId,
        /// Current-tick input.
        input: GraphPortId,
        /// Dependent current-tick output.
        output: GraphPortId,
    },
}

/// Deterministic exact witness for one forbidden current-tick cycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CombinationalCycle {
    links: Vec<DependencyLink>,
}

impl CombinationalCycle {
    /// Borrow cycle links in traversal order.
    pub fn links(&self) -> &[DependencyLink] {
        &self.links
    }
}

/// One admitted explicit state allocation declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeStateAllocation {
    node: GraphNodeId,
    domain: ExecutionDomain,
    clock: GraphClockId,
    value_type: GraphTypeId,
    declared_storage_bytes: u32,
}

impl NodeStateAllocation {
    /// Return the stateful node instance.
    pub const fn node(self) -> GraphNodeId {
        self.node
    }

    /// Return its requested placement.
    pub const fn domain(self) -> ExecutionDomain {
        self.domain
    }

    /// Return its explicit update clock.
    pub const fn clock(self) -> GraphClockId {
        self.clock
    }

    /// Return its registered state type.
    pub const fn value_type(self) -> GraphTypeId {
        self.value_type
    }

    /// Return declared bytes; static representability remains a later proof.
    pub const fn declared_storage_bytes(self) -> u32 {
        self.declared_storage_bytes
    }
}

/// Successful semantic shape/domain/state/cycle analysis report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphAnalysis {
    admitted_nodes: usize,
    dependency_links: usize,
    total_declared_state_bytes: usize,
    state_allocations: Vec<NodeStateAllocation>,
}

impl GraphAnalysis {
    /// Return the number of resolved node instances.
    pub const fn admitted_nodes(&self) -> usize {
        self.admitted_nodes
    }

    /// Return the number of wire plus internal feedthrough links analyzed.
    pub const fn dependency_links(&self) -> usize {
        self.dependency_links
    }

    /// Return the summed declared state bytes.
    pub const fn total_declared_state_bytes(&self) -> usize {
        self.total_declared_state_bytes
    }

    /// Borrow state declarations in canonical node-ID order.
    pub fn state_allocations(&self) -> &[NodeStateAllocation] {
        &self.state_allocations
    }
}

/// Failure at audited node admission or current-tick dependency analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphAnalysisError {
    /// The document's unit/type registry or clocks differ from registry authority.
    SemanticContextMismatch,
    /// An opaque structural node had no audited semantic schema.
    UnresolvedNode {
        /// Exact node instance.
        node: GraphNodeId,
        /// Preserved unresolved kind/version.
        kind: NodeKind,
    },
    /// A node instance contradicted its audited shape.
    NodeShape {
        /// Exact node instance.
        node: GraphNodeId,
        /// Mismatched collection or reference.
        aspect: &'static str,
    },
    /// Requested placement was absent from the audited schema.
    DomainNotAllowed {
        /// Exact node instance.
        node: GraphNodeId,
        /// Rejected concrete placement.
        domain: ExecutionDomain,
    },
    /// Dependency or declared-state policy was exceeded.
    LimitExceeded(&'static str),
    /// Internal DFS ancestry could not reproduce a detected cycle.
    InvalidCycleWitness,
    /// Current-tick feedthrough and wires formed a forbidden cycle.
    CombinationalCycle(CombinationalCycle),
}

impl fmt::Display for GraphAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SemanticContextMismatch => formatter.write_str(
                "graph unit/type registry or clocks differ from node-registry authority",
            ),
            Self::UnresolvedNode { node, kind } => write!(
                formatter,
                "graph node {node:?} kind {}@{} is unresolved",
                kind.name(),
                kind.version()
            ),
            Self::NodeShape { node, aspect } => {
                write!(formatter, "graph node {node:?} contradicts {aspect}")
            }
            Self::DomainNotAllowed { node, domain } => {
                write!(formatter, "graph node {node:?} rejects domain {domain:?}")
            }
            Self::LimitExceeded(name) => write!(formatter, "graph analysis {name} exceeds policy"),
            Self::InvalidCycleWitness => {
                formatter.write_str("graph analysis could not reconstruct a cycle witness")
            }
            Self::CombinationalCycle(cycle) => write!(
                formatter,
                "graph has a {}-link combinational cycle",
                cycle.links.len()
            ),
        }
    }
}

impl std::error::Error for GraphAnalysisError {}

/// Resolve every opaque node through an audited schema, verify exact instance
/// shape/domain/state facts, and reject current-tick combinational cycles.
pub fn analyze_graph(
    document: &GraphDocument,
    registry: &GraphNodeRegistry,
) -> Result<GraphAnalysis, GraphAnalysisError> {
    if document.schema() != registry.context_schema()
        || document.clocks() != registry.context_clocks()
    {
        return Err(GraphAnalysisError::SemanticContextMismatch);
    }
    let mut schemas = Vec::with_capacity(document.nodes().len());
    let mut state_allocations = Vec::new();
    let mut total_declared_state_bytes = 0_usize;
    for node in document.nodes() {
        let schema =
            registry
                .schema(node.kind())
                .ok_or_else(|| GraphAnalysisError::UnresolvedNode {
                    node: node.id(),
                    kind: node.kind().clone(),
                })?;
        validate_instance(node, schema)?;
        if let Some(state) = schema.state {
            let bytes = usize::try_from(state.declared_storage_bytes)
                .map_err(|_| GraphAnalysisError::LimitExceeded("state byte count"))?;
            total_declared_state_bytes = total_declared_state_bytes
                .checked_add(bytes)
                .ok_or(GraphAnalysisError::LimitExceeded("total state bytes"))?;
            if total_declared_state_bytes > registry.limits.maximum_total_state_bytes {
                return Err(GraphAnalysisError::LimitExceeded("total state bytes"));
            }
            state_allocations.push(NodeStateAllocation {
                node: node.id(),
                domain: node.domain(),
                clock: state.clock,
                value_type: state.value_type,
                declared_storage_bytes: state.declared_storage_bytes,
            });
        }
        schemas.push(schema);
    }

    let (adjacency, dependency_links) =
        dependency_graph(document, &schemas, registry.limits.maximum_dependency_links)?;
    if let Some(cycle) = find_cycle(&adjacency, registry.limits.maximum_cycle_witness_links)? {
        return Err(GraphAnalysisError::CombinationalCycle(cycle));
    }
    Ok(GraphAnalysis {
        admitted_nodes: document.nodes().len(),
        dependency_links,
        total_declared_state_bytes,
        state_allocations,
    })
}

fn validate_instance(node: &NodeDefinition, schema: &NodeSchema) -> Result<(), GraphAnalysisError> {
    if !schema.allowed_domains.contains(node.domain()) {
        return Err(GraphAnalysisError::DomainNotAllowed {
            node: node.id(),
            domain: node.domain(),
        });
    }
    validate_instance_ports(node.id(), "input contract", node.inputs(), &schema.inputs)?;
    validate_instance_ports(
        node.id(),
        "output contract",
        node.outputs(),
        &schema.outputs,
    )?;
    if node.parameters().len() != schema.parameters.len() {
        return Err(GraphAnalysisError::NodeShape {
            node: node.id(),
            aspect: "parameter contract",
        });
    }
    for (actual, expected) in node.parameters().iter().zip(&schema.parameters) {
        if actual.id() != expected.id
            || actual.name() != expected.name
            || actual.value().value_type() != expected.value_type
        {
            return Err(GraphAnalysisError::NodeShape {
                node: node.id(),
                aspect: "parameter contract",
            });
        }
    }
    Ok(())
}

fn validate_instance_ports(
    node: GraphNodeId,
    aspect: &'static str,
    actual: &[PortDefinition],
    expected: &[PortDefinition],
) -> Result<(), GraphAnalysisError> {
    if actual.len() != expected.len()
        || actual.iter().zip(expected).any(|(actual, expected)| {
            actual.id() != expected.id()
                || actual.name() != expected.name()
                || actual.value_type() != expected.value_type()
        })
    {
        Err(GraphAnalysisError::NodeShape { node, aspect })
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct DependencyEdge {
    target: usize,
    link: DependencyLink,
}

fn dependency_graph(
    document: &GraphDocument,
    schemas: &[&NodeSchema],
    maximum_links: usize,
) -> Result<(Vec<Vec<DependencyEdge>>, usize), GraphAnalysisError> {
    let mut endpoints = BTreeSet::new();
    for node in document.nodes() {
        for port in node.inputs().iter().chain(node.outputs()) {
            endpoints.insert(WireEndpoint {
                node: node.id(),
                port: port.id(),
            });
        }
    }
    let endpoint_indices: BTreeMap<_, _> = endpoints
        .into_iter()
        .enumerate()
        .map(|(index, endpoint)| (endpoint, index))
        .collect();
    let mut adjacency = vec![Vec::new(); endpoint_indices.len()];
    let mut link_count = 0_usize;
    for (node, schema) in document.nodes().iter().zip(schemas) {
        for dependency in &schema.output_dependencies {
            let target = endpoint_indices[&WireEndpoint {
                node: node.id(),
                port: dependency.output,
            }];
            for input in &dependency.inputs {
                let source = endpoint_indices[&WireEndpoint {
                    node: node.id(),
                    port: *input,
                }];
                add_dependency(
                    &mut adjacency,
                    source,
                    DependencyEdge {
                        target,
                        link: DependencyLink::Feedthrough {
                            node: node.id(),
                            input: *input,
                            output: dependency.output,
                        },
                    },
                    &mut link_count,
                    maximum_links,
                )?;
            }
        }
    }
    for wire in document.wires() {
        let source = endpoint_indices[&wire.source()];
        let target = endpoint_indices[&wire.target()];
        add_dependency(
            &mut adjacency,
            source,
            DependencyEdge {
                target,
                link: DependencyLink::Wire(wire.id()),
            },
            &mut link_count,
            maximum_links,
        )?;
    }
    for edges in &mut adjacency {
        edges.sort_unstable_by_key(|edge| (edge.target, edge.link));
    }
    Ok((adjacency, link_count))
}

fn add_dependency(
    adjacency: &mut [Vec<DependencyEdge>],
    source: usize,
    edge: DependencyEdge,
    count: &mut usize,
    maximum: usize,
) -> Result<(), GraphAnalysisError> {
    *count = count
        .checked_add(1)
        .ok_or(GraphAnalysisError::LimitExceeded("dependency link count"))?;
    if *count > maximum {
        return Err(GraphAnalysisError::LimitExceeded("dependency link count"));
    }
    adjacency[source].push(edge);
    Ok(())
}

fn find_cycle(
    adjacency: &[Vec<DependencyEdge>],
    maximum_witness_links: usize,
) -> Result<Option<CombinationalCycle>, GraphAnalysisError> {
    let mut states = vec![0_u8; adjacency.len()];
    let mut parents: Vec<Option<(usize, DependencyLink)>> = vec![None; adjacency.len()];
    for start in 0..adjacency.len() {
        if states[start] != 0 {
            continue;
        }
        states[start] = 1;
        let mut stack = vec![(start, 0_usize)];
        while let Some((vertex, next_edge)) = stack.last_mut() {
            if *next_edge == adjacency[*vertex].len() {
                states[*vertex] = 2;
                stack.pop();
                continue;
            }
            let source = *vertex;
            let edge = adjacency[source][*next_edge];
            *next_edge += 1;
            match states[edge.target] {
                0 => {
                    states[edge.target] = 1;
                    parents[edge.target] = Some((source, edge.link));
                    stack.push((edge.target, 0));
                }
                1 => {
                    let mut links = Vec::new();
                    let mut cursor = source;
                    while cursor != edge.target {
                        let Some((parent, link)) = parents[cursor] else {
                            return Err(GraphAnalysisError::InvalidCycleWitness);
                        };
                        links.push(link);
                        if links.len() >= maximum_witness_links {
                            return Err(GraphAnalysisError::LimitExceeded(
                                "cycle witness link count",
                            ));
                        }
                        cursor = parent;
                    }
                    links.reverse();
                    links.push(edge.link);
                    if links.len() > maximum_witness_links {
                        return Err(GraphAnalysisError::LimitExceeded(
                            "cycle witness link count",
                        ));
                    }
                    return Ok(Some(CombinationalCycle { links }));
                }
                _ => {}
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use alumina_protocol::DeviceId;
    use hyperreal::Rational;

    use super::*;
    use crate::graph::{
        BaseDimensions, GraphLimits, GraphSchema, GraphValue, NodeParameter, TypeDefinition,
        TypeKind, TypedGraphValue, UnitDefinition, UnitId, WireDefinition,
    };
    use crate::graph::{ClockDefinition, ClockKind};

    const EXACT: GraphTypeId = GraphTypeId::new(1);
    const CLOCK: GraphClockId = GraphClockId::new(1);

    fn graph_schema() -> GraphSchema {
        GraphSchema::try_new(
            GraphLimits::interactive(),
            vec![UnitDefinition::new(
                UnitId::new(1),
                "mm",
                BaseDimensions::LENGTH,
                Rational::fraction(1, 1_000).unwrap(),
            )],
            vec![TypeDefinition::new(
                EXACT,
                "exact.mm",
                TypeKind::ExactRational {
                    unit: UnitId::new(1),
                },
            )],
        )
        .unwrap()
    }

    fn clock() -> ClockDefinition {
        ClockDefinition::new(
            CLOCK,
            "host.tick",
            ClockKind::HostMonotonic {
                ticks_per_second: 1_000,
            },
        )
    }

    fn port(id: u32, name: &str) -> PortDefinition {
        PortDefinition::new(GraphPortId::new(id), name, EXACT)
    }

    fn dependency(output: u32, inputs: &[u32]) -> NodeOutputDependency {
        NodeOutputDependency::new(
            GraphPortId::new(output),
            inputs.iter().copied().map(GraphPortId::new).collect(),
        )
    }

    fn source_schema() -> NodeSchema {
        NodeSchema::new(
            NodeKind::new("test.source", 1),
            ExecutionDomainSet::HOST_EXACT,
            Vec::new(),
            vec![port(1, "value")],
            Vec::new(),
            vec![dependency(1, &[])],
            None,
        )
    }

    fn add_schema() -> NodeSchema {
        NodeSchema::new(
            NodeKind::new("test.add", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "left"), port(2, "right")],
            vec![port(3, "sum")],
            Vec::new(),
            vec![dependency(3, &[2, 1])],
            None,
        )
    }

    fn delay_schema() -> NodeSchema {
        NodeSchema::new(
            NodeKind::new("test.delay", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "next")],
            vec![port(2, "current")],
            vec![NodeParameterContract::new(1, "initial", EXACT)],
            vec![dependency(2, &[])],
            Some(NodeStateContract::new(
                CLOCK,
                EXACT,
                1,
                GraphPortId::new(1),
                GraphPortId::new(2),
                64,
            )),
        )
    }

    fn sink_schema() -> NodeSchema {
        NodeSchema::new(
            NodeKind::new("test.sink", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "value")],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
    }

    fn registry(context: &GraphDocument) -> GraphNodeRegistry {
        GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            context,
            vec![delay_schema(), sink_schema(), add_schema(), source_schema()],
        )
        .unwrap()
    }

    fn initial_parameter(schema: &GraphSchema) -> NodeParameter {
        NodeParameter::new(
            1,
            "initial",
            TypedGraphValue::try_new(schema, EXACT, GraphValue::ExactRational(Rational::zero()))
                .unwrap(),
        )
    }

    fn node(
        id: u32,
        kind: &str,
        inputs: Vec<PortDefinition>,
        outputs: Vec<PortDefinition>,
        parameters: Vec<NodeParameter>,
    ) -> NodeDefinition {
        NodeDefinition::new(
            GraphNodeId::new(id),
            NodeKind::new(kind, 1),
            format!("node {id}"),
            ExecutionDomain::HostExact,
            inputs,
            outputs,
            parameters,
        )
    }

    fn endpoint(node: u32, port: u32) -> WireEndpoint {
        WireEndpoint {
            node: GraphNodeId::new(node),
            port: GraphPortId::new(port),
        }
    }

    fn wire(id: u32, source: (u32, u32), target: (u32, u32)) -> WireDefinition {
        WireDefinition::new(
            GraphWireId::new(id),
            endpoint(source.0, source.1),
            endpoint(target.0, target.1),
        )
    }

    fn self_cycle_document() -> GraphDocument {
        GraphDocument::try_new(
            1,
            graph_schema(),
            vec![clock()],
            vec![
                node(
                    1,
                    "test.source",
                    Vec::new(),
                    vec![port(1, "value")],
                    Vec::new(),
                ),
                node(
                    2,
                    "test.add",
                    vec![port(1, "left"), port(2, "right")],
                    vec![port(3, "sum")],
                    Vec::new(),
                ),
            ],
            vec![wire(1, (2, 3), (2, 1)), wire(2, (1, 1), (2, 2))],
        )
        .unwrap()
    }

    #[test]
    fn explicit_delay_breaks_feedback_and_retains_state_report() {
        let schema = graph_schema();
        let document = GraphDocument::try_new(
            1,
            schema.clone(),
            vec![clock()],
            vec![
                node(
                    1,
                    "test.source",
                    Vec::new(),
                    vec![port(1, "value")],
                    Vec::new(),
                ),
                node(
                    2,
                    "test.add",
                    vec![port(2, "right"), port(1, "left")],
                    vec![port(3, "sum")],
                    Vec::new(),
                ),
                node(
                    3,
                    "test.delay",
                    vec![port(1, "next")],
                    vec![port(2, "current")],
                    vec![initial_parameter(&schema)],
                ),
                node(
                    4,
                    "test.sink",
                    vec![port(1, "value")],
                    Vec::new(),
                    Vec::new(),
                ),
            ],
            vec![
                wire(1, (1, 1), (2, 1)),
                wire(2, (3, 2), (2, 2)),
                wire(3, (2, 3), (3, 1)),
                wire(4, (2, 3), (4, 1)),
            ],
        )
        .unwrap();

        let report = analyze_graph(&document, &registry(&document)).unwrap();
        assert_eq!(report.admitted_nodes(), 4);
        assert_eq!(report.dependency_links(), 6);
        assert_eq!(report.total_declared_state_bytes(), 64);
        assert_eq!(report.state_allocations().len(), 1);
        assert_eq!(report.state_allocations()[0].node(), GraphNodeId::new(3));
        assert_eq!(report.state_allocations()[0].clock(), CLOCK);
        assert_eq!(report.state_allocations()[0].value_type(), EXACT);
    }

    #[test]
    fn pure_self_feedback_returns_exact_wire_and_feedthrough_cycle() {
        let document = self_cycle_document();
        let GraphAnalysisError::CombinationalCycle(cycle) =
            analyze_graph(&document, &registry(&document)).unwrap_err()
        else {
            panic!("combinational cycle expected");
        };
        assert_eq!(
            cycle.links(),
            &[
                DependencyLink::Wire(GraphWireId::new(1)),
                DependencyLink::Feedthrough {
                    node: GraphNodeId::new(2),
                    input: GraphPortId::new(1),
                    output: GraphPortId::new(3),
                },
            ]
        );
    }

    #[test]
    fn unresolved_shape_domain_and_semantic_context_fail_at_exact_node() {
        let unknown = GraphDocument::try_new(
            0,
            graph_schema(),
            vec![clock()],
            vec![node(7, "unknown.node", Vec::new(), Vec::new(), Vec::new())],
            Vec::new(),
        )
        .unwrap();
        assert!(matches!(
            analyze_graph(&unknown, &registry(&unknown)),
            Err(GraphAnalysisError::UnresolvedNode {
                node,
                ..
            }) if node == GraphNodeId::new(7)
        ));

        let wrong_shape = GraphDocument::try_new(
            0,
            graph_schema(),
            vec![clock()],
            vec![node(
                2,
                "test.add",
                vec![port(1, "left")],
                vec![port(3, "sum")],
                Vec::new(),
            )],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&wrong_shape, &registry(&wrong_shape)),
            Err(GraphAnalysisError::NodeShape {
                node: GraphNodeId::new(2),
                aspect: "input contract",
            })
        );

        let service_source = NodeDefinition::new(
            GraphNodeId::new(1),
            NodeKind::new("test.source", 1),
            "source",
            ExecutionDomain::Service {
                device_id: DeviceId([1; 16]),
            },
            Vec::new(),
            vec![port(1, "value")],
            Vec::new(),
        );
        let wrong_domain = GraphDocument::try_new(
            0,
            graph_schema(),
            vec![clock()],
            vec![service_source],
            Vec::new(),
        )
        .unwrap();
        assert!(matches!(
            analyze_graph(&wrong_domain, &registry(&wrong_domain)),
            Err(GraphAnalysisError::DomainNotAllowed {
                node,
                ..
            }) if node == GraphNodeId::new(1)
        ));

        let schema = graph_schema();
        let missing_clock = GraphDocument::try_new(
            0,
            schema.clone(),
            Vec::new(),
            vec![node(
                3,
                "test.delay",
                vec![port(1, "next")],
                vec![port(2, "current")],
                vec![initial_parameter(&schema)],
            )],
            Vec::new(),
        )
        .unwrap();
        let context = self_cycle_document();
        assert_eq!(
            analyze_graph(&missing_clock, &registry(&context)),
            Err(GraphAnalysisError::SemanticContextMismatch)
        );
        assert!(matches!(
            GraphNodeRegistry::try_new(
                GraphAnalysisLimits::interactive(),
                &missing_clock,
                vec![delay_schema()]
            ),
            Err(NodeRegistryError::InvalidSchema {
                aspect: "state clock",
                ..
            })
        ));

        let reinterpreted_schema = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![TypeDefinition::new(EXACT, "core.bool", TypeKind::Boolean)],
        )
        .unwrap();
        let reinterpreted_type = GraphDocument::try_new(
            0,
            reinterpreted_schema,
            vec![clock()],
            vec![node(
                1,
                "test.source",
                Vec::new(),
                vec![port(1, "value")],
                Vec::new(),
            )],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&reinterpreted_type, &registry(&context)),
            Err(GraphAnalysisError::SemanticContextMismatch)
        );
    }

    #[test]
    fn registry_rejects_duplicate_kinds_and_state_feedthrough() {
        let context = self_cycle_document();
        assert!(matches!(
            GraphNodeRegistry::try_new(
                GraphAnalysisLimits::interactive(),
                &context,
                vec![source_schema(), source_schema()]
            ),
            Err(NodeRegistryError::DuplicateKind(_))
        ));

        let invalid_delay = NodeSchema::new(
            NodeKind::new("test.delay", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "next")],
            vec![port(2, "current")],
            vec![NodeParameterContract::new(1, "initial", EXACT)],
            vec![dependency(2, &[1])],
            Some(NodeStateContract::new(
                CLOCK,
                EXACT,
                1,
                GraphPortId::new(1),
                GraphPortId::new(2),
                64,
            )),
        );
        assert!(matches!(
            GraphNodeRegistry::try_new(
                GraphAnalysisLimits::interactive(),
                &context,
                vec![invalid_delay]
            ),
            Err(NodeRegistryError::InvalidSchema {
                aspect: "state output feedthrough",
                ..
            })
        ));
    }

    #[test]
    fn dependency_witness_and_total_state_limits_fail_before_reports_exist() {
        let cycle_document = self_cycle_document();
        let mut link_limits = GraphAnalysisLimits::interactive();
        link_limits.maximum_dependency_links = 1;
        let link_registry = GraphNodeRegistry::try_new(
            link_limits,
            &cycle_document,
            vec![delay_schema(), sink_schema(), add_schema(), source_schema()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&cycle_document, &link_registry),
            Err(GraphAnalysisError::LimitExceeded("dependency link count"))
        );

        let mut witness_limits = GraphAnalysisLimits::interactive();
        witness_limits.maximum_cycle_witness_links = 1;
        let witness_registry = GraphNodeRegistry::try_new(
            witness_limits,
            &cycle_document,
            vec![delay_schema(), sink_schema(), add_schema(), source_schema()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&cycle_document, &witness_registry),
            Err(GraphAnalysisError::LimitExceeded(
                "cycle witness link count"
            ))
        );

        let schema = graph_schema();
        let state_document = GraphDocument::try_new(
            0,
            schema.clone(),
            vec![clock()],
            vec![node(
                3,
                "test.delay",
                vec![port(1, "next")],
                vec![port(2, "current")],
                vec![initial_parameter(&schema)],
            )],
            Vec::new(),
        )
        .unwrap();
        let mut state_limits = GraphAnalysisLimits::interactive();
        state_limits.maximum_total_state_bytes = 32;
        let state_registry =
            GraphNodeRegistry::try_new(state_limits, &state_document, vec![delay_schema()])
                .unwrap();
        assert_eq!(
            analyze_graph(&state_document, &state_registry),
            Err(GraphAnalysisError::LimitExceeded("total state bytes"))
        );
    }
}
