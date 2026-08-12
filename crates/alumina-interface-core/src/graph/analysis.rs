//! Audited node-schema admission and bounded combinational-cycle analysis.
//!
//! A structural [`super::GraphDocument`] deliberately preserves opaque node
//! kinds. This module is the separate semantic boundary: a node becomes known
//! only through an exact registry entry that declares its full shape, allowed
//! execution domains, per-output current-tick feedthrough, bounded input
//! delivery, explicit rate transitions, and optional read-before-write state
//! boundary. Admission here remains host-side analysis; it emits no firmware
//! opcode and grants no real-time authority.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use hyperreal::Rational;

use super::storage::{analyze_type_storage, literal_storage_bytes};
use super::{
    ClockDefinition, ExecutionDomain, GraphClockId, GraphDocument, GraphNodeId, GraphPortId,
    GraphSchema, GraphTypeId, GraphWireId, NodeDefinition, NodeKind, PortDefinition, TypeKind,
    WireEndpoint, valid_stable_name,
};
use super::{GraphStorageError, GraphTypeStorageBound, GraphTypeStorageKind};

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

    /// Return the canonical Host/Service/Realtime membership bitset.
    pub const fn bits(self) -> u8 {
        self.0
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
    /// Maximum queued event/stream items at one input.
    pub maximum_queue_items_per_input: usize,
    /// Maximum canonical bytes reserved for one connected input.
    pub maximum_channel_bytes_per_input: u64,
    /// Maximum combined canonical input-channel bytes.
    pub maximum_total_channel_bytes: u64,
    /// Maximum admitted rate-transition instances in one graph.
    pub maximum_rate_transitions: usize,
    /// Maximum source or target ticks in one exact repeating rate pattern.
    pub maximum_rate_pattern_ticks: u64,
    /// Maximum combined canonical latest-sample retention for rate transitions.
    pub maximum_total_rate_transition_state_bytes: u64,
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
            maximum_queue_items_per_input: 4_096,
            maximum_channel_bytes_per_input: 64 * 1024 * 1024,
            maximum_total_channel_bytes: 256 * 1024 * 1024,
            maximum_rate_transitions: 8_192,
            maximum_rate_pattern_ticks: 1_000_000,
            maximum_total_rate_transition_state_bytes: 64 * 1024 * 1024,
        }
    }

    fn validate(self) -> Result<(), NodeRegistryError> {
        if [
            self.maximum_registered_kinds,
            self.maximum_dependency_links,
            self.maximum_cycle_witness_links,
            self.maximum_state_bytes_per_node,
            self.maximum_total_state_bytes,
            self.maximum_queue_items_per_input,
            self.maximum_rate_transitions,
        ]
        .contains(&0)
            || self.maximum_channel_bytes_per_input == 0
            || self.maximum_total_channel_bytes == 0
            || self.maximum_rate_pattern_ticks == 0
            || self.maximum_total_rate_transition_state_bytes == 0
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

/// Canonical queue-item overhead reserved by the first host analysis contract:
/// one `u64` source-clock tick and one `u64` monotonic sequence.
pub const GRAPH_CHANNEL_ENVELOPE_BYTES: u64 = 16;

/// Whether one declared input must have a structural source wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputConnectionRequirement {
    /// Analysis fails when no wire owns this input.
    Required,
    /// The audited implementation explicitly handles absence.
    Optional,
}

/// Deterministic action when a bounded event/stream queue is full.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelFullPolicy {
    /// Refuse producer progress until capacity exists.
    Backpressure,
    /// Enter typed graph fault flow; later lowering must define the safe action.
    Fault,
    /// Preserve queued history and discard the arriving item.
    DropNewest,
    /// Preserve the arriving item and discard the oldest queued item.
    DropOldest,
}

/// Input delivery and bounded buffering semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeInputChannelKind {
    /// One current synchronous typed value; no queue envelope.
    Synchronous,
    /// Bounded timestamped event queue.
    EventQueue {
        /// Exact item capacity.
        capacity: u32,
        /// Explicit full-queue action.
        full_policy: ChannelFullPolicy,
    },
    /// Bounded timestamped stream-sample queue.
    StreamQueue {
        /// Exact item capacity, no greater than the registered stream type.
        capacity: u32,
        /// Explicit full-queue action.
        full_policy: ChannelFullPolicy,
    },
}

/// Audited delivery contract for one exact input port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeInputChannelContract {
    port: GraphPortId,
    requirement: InputConnectionRequirement,
    kind: NodeInputChannelKind,
}

impl NodeInputChannelContract {
    /// Construct one input delivery contract.
    pub const fn new(
        port: GraphPortId,
        requirement: InputConnectionRequirement,
        kind: NodeInputChannelKind,
    ) -> Self {
        Self {
            port,
            requirement,
            kind,
        }
    }

    /// Return the exact input port.
    pub const fn port(self) -> GraphPortId {
        self.port
    }

    /// Return whether a structural connection is mandatory.
    pub const fn requirement(self) -> InputConnectionRequirement {
        self.requirement
    }

    /// Return synchronous or bounded queued delivery semantics.
    pub const fn kind(self) -> NodeInputChannelKind {
        self.kind
    }
}

/// Deterministic first-release resampling semantics between two Stream clocks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateTransitionKind {
    /// At each target tick, consume all source samples due at or before that
    /// instant and emit the newest. Every coincident source/target tick,
    /// including run start, processes the source sample first. No implicit
    /// initial value exists.
    LatestAtOrBeforeSourceFirst,
}

/// Audited cross-clock dependency between one Stream input and Stream output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeRateTransitionContract {
    input: GraphPortId,
    output: GraphPortId,
    kind: RateTransitionKind,
}

impl NodeRateTransitionContract {
    /// Construct one explicit Stream rate transition.
    pub const fn new(input: GraphPortId, output: GraphPortId, kind: RateTransitionKind) -> Self {
        Self {
            input,
            output,
            kind,
        }
    }

    /// Return the source Stream input.
    pub const fn input(self) -> GraphPortId {
        self.input
    }

    /// Return the target Stream output.
    pub const fn output(self) -> GraphPortId {
        self.output
    }

    /// Return the complete deterministic sample-selection policy.
    pub const fn kind(self) -> RateTransitionKind {
        self.kind
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
/// Each state port may carry the literal directly or wrap that literal in a
/// Stream on the same update clock. Storage always holds one literal value,
/// never the Stream envelope or history.
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

    /// Return the declared storage ceiling, checked during graph analysis.
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
    input_channels: Vec<NodeInputChannelContract>,
    outputs: Vec<PortDefinition>,
    parameters: Vec<NodeParameterContract>,
    output_dependencies: Vec<NodeOutputDependency>,
    rate_transitions: Vec<NodeRateTransitionContract>,
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
        input_channels: Vec<NodeInputChannelContract>,
        outputs: Vec<PortDefinition>,
        parameters: Vec<NodeParameterContract>,
        output_dependencies: Vec<NodeOutputDependency>,
        rate_transitions: Vec<NodeRateTransitionContract>,
        state: Option<NodeStateContract>,
    ) -> Self {
        Self {
            kind,
            allowed_domains,
            inputs,
            input_channels,
            outputs,
            parameters,
            output_dependencies,
            rate_transitions,
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

    /// Borrow exact delivery contracts in canonical input-port order.
    pub fn input_channels(&self) -> &[NodeInputChannelContract] {
        &self.input_channels
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

    /// Borrow explicit cross-clock dependencies in canonical output/input order.
    pub fn rate_transitions(&self) -> &[NodeRateTransitionContract] {
        &self.rate_transitions
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
            schema
                .input_channels
                .sort_unstable_by_key(|channel| channel.port);
            schema.outputs.sort_unstable_by_key(PortDefinition::id);
            schema
                .parameters
                .sort_unstable_by_key(NodeParameterContract::id);
            schema
                .output_dependencies
                .sort_unstable_by_key(NodeOutputDependency::output);
            schema
                .rate_transitions
                .sort_unstable_by_key(|transition| (transition.output, transition.input));
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
    if schema.input_channels.len() != schema.inputs.len() {
        return Err(invalid_schema(schema, "input channel coverage"));
    }
    for (input, channel) in schema.inputs.iter().zip(&schema.input_channels) {
        if input.id() != channel.port {
            return Err(invalid_schema(schema, "input channel coverage"));
        }
        let Some(definition) = context_schema.value_type(input.value_type()) else {
            return Err(invalid_schema(schema, "input channel type"));
        };
        let kind = definition.kind();
        match (kind, channel.kind) {
            (TypeKind::Event { .. }, NodeInputChannelKind::EventQueue { capacity, .. }) => {
                validate_queue_capacity(schema, capacity, limits.maximum_queue_items_per_input)?;
            }
            (
                TypeKind::Stream {
                    capacity: type_capacity,
                    ..
                },
                NodeInputChannelKind::StreamQueue { capacity, .. },
            ) => {
                validate_queue_capacity(schema, capacity, limits.maximum_queue_items_per_input)?;
                if capacity > *type_capacity {
                    return Err(invalid_schema(schema, "stream queue capacity"));
                }
            }
            (
                TypeKind::Boolean
                | TypeKind::ExactRational { .. }
                | TypeKind::MeasurementInterval { .. }
                | TypeKind::CanonicalI64 { .. }
                | TypeKind::CanonicalU64 { .. }
                | TypeKind::Text { .. }
                | TypeKind::Bytes { .. }
                | TypeKind::Array { .. }
                | TypeKind::Record { .. }
                | TypeKind::Option { .. }
                | TypeKind::Result { .. }
                | TypeKind::ResourceHandle { .. }
                | TypeKind::JobHandle,
                NodeInputChannelKind::Synchronous,
            ) => {}
            _ => return Err(invalid_schema(schema, "input channel type")),
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
    validate_rate_transition_contracts(schema, context_schema)?;
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
            || next.and_then(|port| state_path_value_type(context_schema, state.clock, port))
                != Some(state.value_type)
            || current.and_then(|port| state_path_value_type(context_schema, state.clock, port))
                != Some(state.value_type)
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

fn state_path_value_type(
    schema: &GraphSchema,
    state_clock: GraphClockId,
    port: &PortDefinition,
) -> Option<GraphTypeId> {
    match schema.value_type(port.value_type())?.kind() {
        TypeKind::Stream { sample, clock, .. } if *clock == state_clock => Some(*sample),
        TypeKind::Stream { .. } | TypeKind::Event { .. } => None,
        _ => Some(port.value_type()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimePortKind {
    Event,
    Stream,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimePort {
    kind: RuntimePortKind,
    payload: GraphTypeId,
    clock: GraphClockId,
}

fn runtime_port(schema: &GraphSchema, port: &PortDefinition) -> Option<RuntimePort> {
    match schema.value_type(port.value_type())?.kind() {
        TypeKind::Event { payload, clock } => Some(RuntimePort {
            kind: RuntimePortKind::Event,
            payload: *payload,
            clock: *clock,
        }),
        TypeKind::Stream { sample, clock, .. } => Some(RuntimePort {
            kind: RuntimePortKind::Stream,
            payload: *sample,
            clock: *clock,
        }),
        _ => None,
    }
}

fn validate_rate_transition_contracts(
    schema: &NodeSchema,
    context_schema: &GraphSchema,
) -> Result<(), NodeRegistryError> {
    let mut pairs = BTreeSet::new();
    for transition in &schema.rate_transitions {
        if transition.input.get() == 0
            || transition.output.get() == 0
            || !pairs.insert((transition.output, transition.input))
        {
            return Err(invalid_schema(schema, "rate transition identity"));
        }
        let Some(input) = schema
            .inputs
            .iter()
            .find(|port| port.id() == transition.input)
        else {
            return Err(invalid_schema(schema, "rate transition input"));
        };
        let Some(output) = schema
            .outputs
            .iter()
            .find(|port| port.id() == transition.output)
        else {
            return Err(invalid_schema(schema, "rate transition output"));
        };
        let Some(source) = runtime_port(context_schema, input) else {
            return Err(invalid_schema(schema, "rate transition input type"));
        };
        let Some(target) = runtime_port(context_schema, output) else {
            return Err(invalid_schema(schema, "rate transition output type"));
        };
        if source.kind != RuntimePortKind::Stream
            || target.kind != RuntimePortKind::Stream
            || source.payload != target.payload
            || source.clock == target.clock
        {
            return Err(invalid_schema(schema, "rate transition type"));
        }
        let input_channel = schema
            .input_channels
            .iter()
            .find(|channel| channel.port == transition.input);
        if !matches!(
            input_channel,
            Some(NodeInputChannelContract {
                requirement: InputConnectionRequirement::Required,
                kind: NodeInputChannelKind::StreamQueue { .. },
                ..
            })
        ) {
            return Err(invalid_schema(schema, "rate transition input delivery"));
        }
        let dependency = schema
            .output_dependencies
            .iter()
            .find(|dependency| dependency.output == transition.output);
        if !matches!(dependency, Some(dependency) if dependency.inputs.contains(&transition.input))
        {
            return Err(invalid_schema(schema, "rate transition dependency"));
        }
    }

    for dependency in &schema.output_dependencies {
        let output = schema
            .outputs
            .iter()
            .find(|port| port.id() == dependency.output)
            .ok_or_else(|| invalid_schema(schema, "output dependency coverage"))?;
        let Some(target) = runtime_port(context_schema, output) else {
            continue;
        };
        for input_id in &dependency.inputs {
            let input = schema
                .inputs
                .iter()
                .find(|port| port.id() == *input_id)
                .ok_or_else(|| invalid_schema(schema, "output dependency input"))?;
            let Some(source) = runtime_port(context_schema, input) else {
                continue;
            };
            if source.clock == target.clock {
                continue;
            }
            if source.kind != RuntimePortKind::Stream
                || target.kind != RuntimePortKind::Stream
                || source.payload != target.payload
                || !pairs.contains(&(dependency.output, *input_id))
            {
                return Err(invalid_schema(schema, "cross-clock dependency"));
            }
        }
    }
    Ok(())
}

fn validate_queue_capacity(
    schema: &NodeSchema,
    capacity: u32,
    maximum: usize,
) -> Result<(), NodeRegistryError> {
    if capacity == 0 || capacity as usize > maximum {
        Err(invalid_schema(schema, "queue capacity"))
    } else {
        Ok(())
    }
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
    required_canonical_bytes: u64,
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

    /// Return declared bytes, validated against canonical type storage.
    pub const fn declared_storage_bytes(self) -> u32 {
        self.declared_storage_bytes
    }

    /// Return the proven maximum canonical typed-value representation.
    pub const fn required_canonical_bytes(self) -> u64 {
        self.required_canonical_bytes
    }
}

/// One connected input's proven canonical slot/queue allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphChannelAllocation {
    source: WireEndpoint,
    target: WireEndpoint,
    kind: NodeInputChannelKind,
    maximum_item_bytes: u64,
    maximum_total_bytes: u64,
}

impl GraphChannelAllocation {
    /// Return the structural output source.
    pub const fn source(self) -> WireEndpoint {
        self.source
    }

    /// Return the uniquely owned input target.
    pub const fn target(self) -> WireEndpoint {
        self.target
    }

    /// Return exact delivery/full-queue semantics.
    pub const fn kind(self) -> NodeInputChannelKind {
        self.kind
    }

    /// Return one complete value or timestamped queue-item ceiling.
    pub const fn maximum_item_bytes(self) -> u64 {
        self.maximum_item_bytes
    }

    /// Return the complete slot/queue ceiling for this connected input.
    pub const fn maximum_total_bytes(self) -> u64 {
        self.maximum_total_bytes
    }
}

/// One registered clock's exact frequency and shared tick-zero root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphClockRate {
    clock: GraphClockId,
    root: GraphClockId,
    ticks_per_second: Rational,
}

impl GraphClockRate {
    /// Return the resolved clock.
    pub const fn clock(&self) -> GraphClockId {
        self.clock
    }

    /// Return the independent HostMonotonic or DeviceCycle root clock.
    pub const fn root(&self) -> GraphClockId {
        self.root
    }

    /// Borrow the exact positive rational frequency in ticks per SI second.
    pub const fn ticks_per_second(&self) -> &Rational {
        &self.ticks_per_second
    }
}

/// One admitted exact repeating Stream-rate transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphRateTransition {
    node: GraphNodeId,
    input: GraphPortId,
    output: GraphPortId,
    kind: RateTransitionKind,
    source_clock: GraphClockId,
    target_clock: GraphClockId,
    root_clock: GraphClockId,
    source_ticks_per_pattern: u64,
    target_ticks_per_pattern: u64,
    required_input_capacity: u64,
    retained_sample_bytes: u64,
}

impl GraphRateTransition {
    /// Return the admitted node instance.
    pub const fn node(self) -> GraphNodeId {
        self.node
    }

    /// Return the source Stream input port.
    pub const fn input(self) -> GraphPortId {
        self.input
    }

    /// Return the target Stream output port.
    pub const fn output(self) -> GraphPortId {
        self.output
    }

    /// Return exact sample-selection and coincident-tick ordering semantics.
    pub const fn kind(self) -> RateTransitionKind {
        self.kind
    }

    /// Return the source Stream clock.
    pub const fn source_clock(self) -> GraphClockId {
        self.source_clock
    }

    /// Return the target Stream clock.
    pub const fn target_clock(self) -> GraphClockId {
        self.target_clock
    }

    /// Return the shared tick-zero clock root.
    pub const fn root_clock(self) -> GraphClockId {
        self.root_clock
    }

    /// Return exact source ticks in one smallest repeating schedule pattern.
    pub const fn source_ticks_per_pattern(self) -> u64 {
        self.source_ticks_per_pattern
    }

    /// Return exact target ticks in one smallest repeating schedule pattern.
    pub const fn target_ticks_per_pattern(self) -> u64 {
        self.target_ticks_per_pattern
    }

    /// Return the minimum input queue items needed between target evaluations.
    pub const fn required_input_capacity(self) -> u64 {
        self.required_input_capacity
    }

    /// Return one complete canonical latest-sample retention ceiling.
    pub const fn retained_sample_bytes(self) -> u64 {
        self.retained_sample_bytes
    }
}

/// Successful semantic shape/domain/state/channel/rate/cycle analysis report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphAnalysis {
    admitted_nodes: usize,
    dependency_links: usize,
    total_declared_state_bytes: usize,
    total_required_state_bytes: u64,
    state_allocations: Vec<NodeStateAllocation>,
    type_storage_bounds: Vec<GraphTypeStorageBound>,
    total_channel_bytes: u64,
    channel_allocations: Vec<GraphChannelAllocation>,
    clock_rates: Vec<GraphClockRate>,
    rate_transitions: Vec<GraphRateTransition>,
    total_rate_transition_state_bytes: u64,
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

    /// Return summed proven canonical bytes for all explicit state values.
    pub const fn total_required_state_bytes(&self) -> u64 {
        self.total_required_state_bytes
    }

    /// Borrow state declarations in canonical node-ID order.
    pub fn state_allocations(&self) -> &[NodeStateAllocation] {
        &self.state_allocations
    }

    /// Borrow checked bounds for every registered type in canonical ID order.
    pub fn type_storage_bounds(&self) -> &[GraphTypeStorageBound] {
        &self.type_storage_bounds
    }

    /// Return summed canonical slots/queues for all connected inputs.
    pub const fn total_channel_bytes(&self) -> u64 {
        self.total_channel_bytes
    }

    /// Borrow connected input allocations in canonical node/port order.
    pub fn channel_allocations(&self) -> &[GraphChannelAllocation] {
        &self.channel_allocations
    }

    /// Borrow exact clock frequencies in canonical clock-ID order.
    pub fn clock_rates(&self) -> &[GraphClockRate] {
        &self.clock_rates
    }

    /// Resolve one exact clock-rate report.
    pub fn clock_rate(&self, clock: GraphClockId) -> Option<&GraphClockRate> {
        self.clock_rates
            .binary_search_by_key(&clock, |rate| rate.clock)
            .ok()
            .map(|index| &self.clock_rates[index])
    }

    /// Borrow admitted rate transitions in canonical node/output/input order.
    pub fn rate_transitions(&self) -> &[GraphRateTransition] {
        &self.rate_transitions
    }

    /// Return summed latest-sample retention for all rate transitions.
    pub const fn total_rate_transition_state_bytes(&self) -> u64 {
        self.total_rate_transition_state_bytes
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
    /// Canonical type-storage analysis rejected the registered type graph.
    Storage(GraphStorageError),
    /// Declared state storage could not hold every value of its exact type.
    StateStorageTooSmall {
        /// Stateful node instance.
        node: GraphNodeId,
        /// Caller-declared byte ceiling.
        declared: u32,
        /// Proven maximum canonical typed-value bytes.
        required: u64,
    },
    /// An audited required input had no structural wire.
    RequiredInputUnconnected(WireEndpoint),
    /// A synchronous value attempted to cross concrete execution ownership.
    CrossDomainSynchronous {
        /// Exact structural wire.
        wire: GraphWireId,
        /// Output owner.
        source: ExecutionDomain,
        /// Input owner.
        target: ExecutionDomain,
    },
    /// Type-storage class contradicted an already validated input channel kind.
    ChannelStorageMismatch(WireEndpoint),
    /// A rate transition referenced clocks without one shared tick-zero root.
    RateTransitionRootMismatch {
        /// Exact node instance.
        node: GraphNodeId,
        /// Source Stream input.
        input: GraphPortId,
        /// Target Stream output.
        output: GraphPortId,
        /// Independent source root.
        source_root: GraphClockId,
        /// Independent target root.
        target_root: GraphClockId,
    },
    /// A rate transition's smallest exact schedule exceeded admission policy.
    RateTransitionPatternTooLarge {
        /// Exact node instance.
        node: GraphNodeId,
        /// Source Stream input.
        input: GraphPortId,
        /// Target Stream output.
        output: GraphPortId,
    },
    /// The declared Stream input queue cannot retain one worst-case interval.
    RateTransitionQueueTooSmall {
        /// Exact node instance.
        node: GraphNodeId,
        /// Source Stream input.
        input: GraphPortId,
        /// Declared queue item capacity.
        declared: u32,
        /// Proven minimum item capacity.
        required: u64,
    },
    /// A validated clock reference could not be resolved during analysis.
    ClockResolution(GraphClockId),
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
            Self::Storage(error) => write!(formatter, "graph storage analysis rejected: {error}"),
            Self::StateStorageTooSmall {
                node,
                declared,
                required,
            } => write!(
                formatter,
                "graph node {node:?} declares {declared} state bytes but requires {required}"
            ),
            Self::RequiredInputUnconnected(endpoint) => {
                write!(
                    formatter,
                    "required graph input {endpoint:?} is unconnected"
                )
            }
            Self::CrossDomainSynchronous {
                wire,
                source,
                target,
            } => write!(
                formatter,
                "synchronous graph wire {wire:?} crosses {source:?} to {target:?}"
            ),
            Self::ChannelStorageMismatch(endpoint) => write!(
                formatter,
                "graph input {endpoint:?} channel contradicts its storage class"
            ),
            Self::RateTransitionRootMismatch {
                node,
                input,
                output,
                source_root,
                target_root,
            } => write!(
                formatter,
                "graph node {node:?} rate transition {input:?}->{output:?} crosses independent clock roots {source_root:?} and {target_root:?}"
            ),
            Self::RateTransitionPatternTooLarge {
                node,
                input,
                output,
            } => write!(
                formatter,
                "graph node {node:?} rate transition {input:?}->{output:?} exceeds exact pattern policy"
            ),
            Self::RateTransitionQueueTooSmall {
                node,
                input,
                declared,
                required,
            } => write!(
                formatter,
                "graph node {node:?} rate input {input:?} declares {declared} queue items but requires {required}"
            ),
            Self::ClockResolution(clock) => {
                write!(formatter, "graph clock {clock:?} could not be resolved")
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

impl From<GraphStorageError> for GraphAnalysisError {
    fn from(value: GraphStorageError) -> Self {
        Self::Storage(value)
    }
}

/// Resolve every opaque node through an audited schema, verify exact instance
/// shape/domain/state/channel/rate facts, and reject current-tick cycles.
pub fn analyze_graph(
    document: &GraphDocument,
    registry: &GraphNodeRegistry,
) -> Result<GraphAnalysis, GraphAnalysisError> {
    if document.schema() != registry.context_schema()
        || document.clocks() != registry.context_clocks()
    {
        return Err(GraphAnalysisError::SemanticContextMismatch);
    }
    let type_storage_bounds = analyze_type_storage(document.schema())?;
    let clock_rates = resolve_clock_rates(document)?;
    let connections: BTreeMap<_, _> = document
        .wires()
        .iter()
        .map(|wire| (wire.target(), (wire.source(), wire.id())))
        .collect();
    let mut schemas = Vec::with_capacity(document.nodes().len());
    let mut state_allocations = Vec::new();
    let mut total_declared_state_bytes = 0_usize;
    let mut total_required_state_bytes = 0_u64;
    let mut channel_allocations = Vec::new();
    let mut total_channel_bytes = 0_u64;
    let mut rate_transitions = Vec::new();
    let mut total_rate_transition_state_bytes = 0_u64;
    for node in document.nodes() {
        let schema =
            registry
                .schema(node.kind())
                .ok_or_else(|| GraphAnalysisError::UnresolvedNode {
                    node: node.id(),
                    kind: node.kind().clone(),
                })?;
        validate_instance(node, schema)?;
        for (input, channel) in node.inputs().iter().zip(&schema.input_channels) {
            let target = WireEndpoint {
                node: node.id(),
                port: input.id(),
            };
            let Some((source, wire)) = connections.get(&target).copied() else {
                if channel.requirement == InputConnectionRequirement::Required {
                    return Err(GraphAnalysisError::RequiredInputUnconnected(target));
                }
                continue;
            };
            let source_domain = document
                .node(source.node)
                .map(NodeDefinition::domain)
                .ok_or(GraphAnalysisError::ChannelStorageMismatch(target))?;
            if channel.kind == NodeInputChannelKind::Synchronous && source_domain != node.domain() {
                return Err(GraphAnalysisError::CrossDomainSynchronous {
                    wire,
                    source: source_domain,
                    target: node.domain(),
                });
            }
            let (maximum_item_bytes, maximum_total_bytes) = channel_storage(
                &type_storage_bounds,
                input.value_type(),
                channel.kind,
                target,
            )?;
            if maximum_total_bytes > registry.limits.maximum_channel_bytes_per_input {
                return Err(GraphAnalysisError::LimitExceeded("per-input channel bytes"));
            }
            total_channel_bytes = total_channel_bytes
                .checked_add(maximum_total_bytes)
                .ok_or(GraphAnalysisError::LimitExceeded("total channel bytes"))?;
            if total_channel_bytes > registry.limits.maximum_total_channel_bytes {
                return Err(GraphAnalysisError::LimitExceeded("total channel bytes"));
            }
            channel_allocations.push(GraphChannelAllocation {
                source,
                target,
                kind: channel.kind,
                maximum_item_bytes,
                maximum_total_bytes,
            });
        }
        for transition in &schema.rate_transitions {
            if rate_transitions.len() >= registry.limits.maximum_rate_transitions {
                return Err(GraphAnalysisError::LimitExceeded("rate transition count"));
            }
            let analyzed = analyze_rate_transition(
                document,
                node,
                schema,
                *transition,
                &clock_rates,
                &type_storage_bounds,
                registry.limits.maximum_rate_pattern_ticks,
            )?;
            total_rate_transition_state_bytes = total_rate_transition_state_bytes
                .checked_add(analyzed.retained_sample_bytes)
                .ok_or(GraphAnalysisError::LimitExceeded(
                    "rate transition state bytes",
                ))?;
            if total_rate_transition_state_bytes
                > registry.limits.maximum_total_rate_transition_state_bytes
            {
                return Err(GraphAnalysisError::LimitExceeded(
                    "rate transition state bytes",
                ));
            }
            rate_transitions.push(analyzed);
        }
        if let Some(state) = schema.state {
            let bytes = usize::try_from(state.declared_storage_bytes)
                .map_err(|_| GraphAnalysisError::LimitExceeded("state byte count"))?;
            total_declared_state_bytes = total_declared_state_bytes
                .checked_add(bytes)
                .ok_or(GraphAnalysisError::LimitExceeded("total state bytes"))?;
            if total_declared_state_bytes > registry.limits.maximum_total_state_bytes {
                return Err(GraphAnalysisError::LimitExceeded("total state bytes"));
            }
            let required = literal_storage_bytes(&type_storage_bounds, state.value_type)?;
            if u64::from(state.declared_storage_bytes) < required {
                return Err(GraphAnalysisError::StateStorageTooSmall {
                    node: node.id(),
                    declared: state.declared_storage_bytes,
                    required,
                });
            }
            total_required_state_bytes = total_required_state_bytes.checked_add(required).ok_or(
                GraphAnalysisError::LimitExceeded("required state byte count"),
            )?;
            state_allocations.push(NodeStateAllocation {
                node: node.id(),
                domain: node.domain(),
                clock: state.clock,
                value_type: state.value_type,
                declared_storage_bytes: state.declared_storage_bytes,
                required_canonical_bytes: required,
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
        total_required_state_bytes,
        state_allocations,
        type_storage_bounds,
        total_channel_bytes,
        channel_allocations,
        clock_rates,
        rate_transitions,
        total_rate_transition_state_bytes,
    })
}

fn resolve_clock_rates(
    document: &GraphDocument,
) -> Result<Vec<GraphClockRate>, GraphAnalysisError> {
    let mut rates = Vec::with_capacity(document.clocks().len());
    for clock in document.clocks() {
        let mut current = clock.id();
        let mut multiplier = Rational::one();
        let mut remaining = document.clocks().len().saturating_add(1);
        let (root, ticks_per_second) = loop {
            if remaining == 0 {
                return Err(GraphAnalysisError::ClockResolution(clock.id()));
            }
            remaining -= 1;
            let definition = document
                .clock(current)
                .ok_or(GraphAnalysisError::ClockResolution(current))?;
            match definition.kind() {
                super::ClockKind::HostMonotonic { ticks_per_second }
                | super::ClockKind::DeviceCycle {
                    ticks_per_second, ..
                } => {
                    break (current, Rational::from(ticks_per_second) * multiplier);
                }
                super::ClockKind::Derived {
                    source,
                    numerator,
                    denominator,
                } => {
                    multiplier =
                        multiplier * Rational::from(numerator) / Rational::from(denominator);
                    current = source;
                }
            }
        };
        rates.push(GraphClockRate {
            clock: clock.id(),
            root,
            ticks_per_second,
        });
    }
    Ok(rates)
}

fn analyze_rate_transition(
    document: &GraphDocument,
    node: &NodeDefinition,
    schema: &NodeSchema,
    transition: NodeRateTransitionContract,
    clock_rates: &[GraphClockRate],
    type_storage_bounds: &[GraphTypeStorageBound],
    maximum_pattern_ticks: u64,
) -> Result<GraphRateTransition, GraphAnalysisError> {
    let input = schema
        .inputs
        .iter()
        .find(|port| port.id() == transition.input)
        .ok_or(GraphAnalysisError::NodeShape {
            node: node.id(),
            aspect: "rate transition input",
        })?;
    let output = schema
        .outputs
        .iter()
        .find(|port| port.id() == transition.output)
        .ok_or(GraphAnalysisError::NodeShape {
            node: node.id(),
            aspect: "rate transition output",
        })?;
    let source = runtime_port(document.schema(), input).ok_or(GraphAnalysisError::NodeShape {
        node: node.id(),
        aspect: "rate transition input type",
    })?;
    let target = runtime_port(document.schema(), output).ok_or(GraphAnalysisError::NodeShape {
        node: node.id(),
        aspect: "rate transition output type",
    })?;
    let source_rate = clock_rate(clock_rates, source.clock)?;
    let target_rate = clock_rate(clock_rates, target.clock)?;
    if source_rate.root != target_rate.root {
        return Err(GraphAnalysisError::RateTransitionRootMismatch {
            node: node.id(),
            input: transition.input,
            output: transition.output,
            source_root: source_rate.root,
            target_root: target_rate.root,
        });
    }
    let ratio = source_rate.ticks_per_second.clone() / target_rate.ticks_per_second.clone();
    let source_ticks = u64::try_from(ratio.numerator().clone()).map_err(|_| {
        GraphAnalysisError::RateTransitionPatternTooLarge {
            node: node.id(),
            input: transition.input,
            output: transition.output,
        }
    })?;
    let target_ticks = u64::try_from(ratio.denominator().clone()).map_err(|_| {
        GraphAnalysisError::RateTransitionPatternTooLarge {
            node: node.id(),
            input: transition.input,
            output: transition.output,
        }
    })?;
    if source_ticks > maximum_pattern_ticks || target_ticks > maximum_pattern_ticks {
        return Err(GraphAnalysisError::RateTransitionPatternTooLarge {
            node: node.id(),
            input: transition.input,
            output: transition.output,
        });
    }
    let required_input_capacity =
        source_ticks / target_ticks + u64::from(source_ticks % target_ticks != 0);
    let declared = schema
        .input_channels
        .iter()
        .find(|channel| channel.port == transition.input)
        .and_then(|channel| match channel.kind {
            NodeInputChannelKind::StreamQueue { capacity, .. } => Some(capacity),
            NodeInputChannelKind::Synchronous | NodeInputChannelKind::EventQueue { .. } => None,
        })
        .ok_or(GraphAnalysisError::ChannelStorageMismatch(WireEndpoint {
            node: node.id(),
            port: transition.input,
        }))?;
    if u64::from(declared) < required_input_capacity {
        return Err(GraphAnalysisError::RateTransitionQueueTooSmall {
            node: node.id(),
            input: transition.input,
            declared,
            required: required_input_capacity,
        });
    }
    let retained_sample_bytes = type_storage_bounds
        .binary_search_by_key(&input.value_type(), |bound| bound.value_type())
        .ok()
        .map(|index| type_storage_bounds[index].kind())
        .and_then(|kind| match kind {
            GraphTypeStorageKind::StreamSample {
                maximum_sample_bytes,
                ..
            } => Some(maximum_sample_bytes),
            GraphTypeStorageKind::Literal { .. } | GraphTypeStorageKind::EventPayload { .. } => {
                None
            }
        })
        .ok_or(GraphAnalysisError::ChannelStorageMismatch(WireEndpoint {
            node: node.id(),
            port: transition.input,
        }))?;
    Ok(GraphRateTransition {
        node: node.id(),
        input: transition.input,
        output: transition.output,
        kind: transition.kind,
        source_clock: source.clock,
        target_clock: target.clock,
        root_clock: source_rate.root,
        source_ticks_per_pattern: source_ticks,
        target_ticks_per_pattern: target_ticks,
        required_input_capacity,
        retained_sample_bytes,
    })
}

fn clock_rate(
    rates: &[GraphClockRate],
    clock: GraphClockId,
) -> Result<&GraphClockRate, GraphAnalysisError> {
    rates
        .binary_search_by_key(&clock, |rate| rate.clock)
        .ok()
        .map(|index| &rates[index])
        .ok_or(GraphAnalysisError::ClockResolution(clock))
}

fn channel_storage(
    bounds: &[GraphTypeStorageBound],
    value_type: GraphTypeId,
    channel: NodeInputChannelKind,
    target: WireEndpoint,
) -> Result<(u64, u64), GraphAnalysisError> {
    let bound = bounds
        .binary_search_by_key(&value_type, |bound| bound.value_type())
        .ok()
        .map(|index| bounds[index])
        .ok_or(GraphStorageError::UnknownType(value_type))?;
    let (payload, capacity, queued) = match (bound.kind(), channel) {
        (
            GraphTypeStorageKind::Literal {
                maximum_canonical_bytes,
            },
            NodeInputChannelKind::Synchronous,
        ) => (maximum_canonical_bytes, 1_u64, false),
        (
            GraphTypeStorageKind::EventPayload {
                maximum_payload_bytes,
                ..
            },
            NodeInputChannelKind::EventQueue { capacity, .. },
        ) => (maximum_payload_bytes, u64::from(capacity), true),
        (
            GraphTypeStorageKind::StreamSample {
                maximum_sample_bytes,
                ..
            },
            NodeInputChannelKind::StreamQueue { capacity, .. },
        ) => (maximum_sample_bytes, u64::from(capacity), true),
        _ => return Err(GraphAnalysisError::ChannelStorageMismatch(target)),
    };
    let item = if queued {
        payload
            .checked_add(GRAPH_CHANNEL_ENVELOPE_BYTES)
            .ok_or(GraphAnalysisError::LimitExceeded("channel item bytes"))?
    } else {
        payload
    };
    let total = item
        .checked_mul(capacity)
        .ok_or(GraphAnalysisError::LimitExceeded("channel byte count"))?;
    Ok((item, total))
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
        let mut limits = GraphLimits::interactive();
        limits.maximum_rational_digits = 16;
        GraphSchema::try_new(
            limits,
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

    fn required_sync(port: u32) -> NodeInputChannelContract {
        NodeInputChannelContract::new(
            GraphPortId::new(port),
            InputConnectionRequirement::Required,
            NodeInputChannelKind::Synchronous,
        )
    }

    fn source_schema() -> NodeSchema {
        NodeSchema::new(
            NodeKind::new("test.source", 1),
            ExecutionDomainSet::HOST_EXACT,
            Vec::new(),
            Vec::new(),
            vec![port(1, "value")],
            Vec::new(),
            vec![dependency(1, &[])],
            Vec::new(),
            None,
        )
    }

    fn add_schema() -> NodeSchema {
        NodeSchema::new(
            NodeKind::new("test.add", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "left"), port(2, "right")],
            vec![required_sync(1), required_sync(2)],
            vec![port(3, "sum")],
            Vec::new(),
            vec![dependency(3, &[2, 1])],
            Vec::new(),
            None,
        )
    }

    fn delay_schema() -> NodeSchema {
        NodeSchema::new(
            NodeKind::new("test.delay", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "next")],
            vec![required_sync(1)],
            vec![port(2, "current")],
            vec![NodeParameterContract::new(1, "initial", EXACT)],
            vec![dependency(2, &[])],
            Vec::new(),
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
            vec![required_sync(1)],
            Vec::new(),
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
        assert_eq!(report.total_required_state_bytes(), 45);
        assert_eq!(report.total_channel_bytes(), 180);
        assert_eq!(report.clock_rates().len(), 1);
        assert_eq!(report.clock_rates()[0].clock(), CLOCK);
        assert_eq!(report.clock_rates()[0].root(), CLOCK);
        assert_eq!(
            report.clock_rates()[0].ticks_per_second(),
            &Rational::from(1_000_u64)
        );
        assert!(report.rate_transitions().is_empty());
        assert_eq!(report.total_rate_transition_state_bytes(), 0);
        assert_eq!(report.channel_allocations().len(), 4);
        assert!(report.channel_allocations().iter().all(|allocation| {
            allocation.kind() == NodeInputChannelKind::Synchronous
                && allocation.maximum_item_bytes() == 45
                && allocation.maximum_total_bytes() == 45
        }));
        assert_eq!(report.state_allocations().len(), 1);
        assert_eq!(report.state_allocations()[0].node(), GraphNodeId::new(3));
        assert_eq!(report.state_allocations()[0].clock(), CLOCK);
        assert_eq!(report.state_allocations()[0].value_type(), EXACT);
        assert_eq!(report.state_allocations()[0].required_canonical_bytes(), 45);
        assert_eq!(
            report.type_storage_bounds()[0].maximum_literal_bytes(),
            Some(45)
        );
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
            vec![required_sync(1)],
            vec![port(2, "current")],
            vec![NodeParameterContract::new(1, "initial", EXACT)],
            vec![dependency(2, &[1])],
            Vec::new(),
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
            vec![
                node(
                    1,
                    "test.source",
                    Vec::new(),
                    vec![port(1, "value")],
                    Vec::new(),
                ),
                node(
                    3,
                    "test.delay",
                    vec![port(1, "next")],
                    vec![port(2, "current")],
                    vec![initial_parameter(&schema)],
                ),
            ],
            vec![wire(1, (1, 1), (3, 1))],
        )
        .unwrap();
        let mut state_limits = GraphAnalysisLimits::interactive();
        state_limits.maximum_total_state_bytes = 32;
        let state_registry = GraphNodeRegistry::try_new(
            state_limits,
            &state_document,
            vec![delay_schema(), source_schema()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&state_document, &state_registry),
            Err(GraphAnalysisError::LimitExceeded("total state bytes"))
        );

        let small_delay = NodeSchema::new(
            NodeKind::new("test.delay", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "next")],
            vec![required_sync(1)],
            vec![port(2, "current")],
            vec![NodeParameterContract::new(1, "initial", EXACT)],
            vec![dependency(2, &[])],
            Vec::new(),
            Some(NodeStateContract::new(
                CLOCK,
                EXACT,
                1,
                GraphPortId::new(1),
                GraphPortId::new(2),
                44,
            )),
        );
        let small_registry = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &state_document,
            vec![small_delay, source_schema()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&state_document, &small_registry),
            Err(GraphAnalysisError::StateStorageTooSmall {
                node: GraphNodeId::new(3),
                declared: 44,
                required: 45,
            })
        );
    }

    #[test]
    fn required_optional_and_cross_domain_synchronous_inputs_are_explicit() {
        let required_document = GraphDocument::try_new(
            0,
            graph_schema(),
            vec![clock()],
            vec![node(
                1,
                "test.sink",
                vec![port(1, "value")],
                Vec::new(),
                Vec::new(),
            )],
            Vec::new(),
        )
        .unwrap();
        let required_registry = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &required_document,
            vec![sink_schema()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&required_document, &required_registry),
            Err(GraphAnalysisError::RequiredInputUnconnected(endpoint(1, 1)))
        );

        let optional_sink = NodeSchema::new(
            NodeKind::new("test.sink", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "value")],
            vec![NodeInputChannelContract::new(
                GraphPortId::new(1),
                InputConnectionRequirement::Optional,
                NodeInputChannelKind::Synchronous,
            )],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        let optional_registry = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &required_document,
            vec![optional_sink],
        )
        .unwrap();
        let optional_report = analyze_graph(&required_document, &optional_registry).unwrap();
        assert_eq!(optional_report.total_channel_bytes(), 0);
        assert!(optional_report.channel_allocations().is_empty());

        let service_sink_node = NodeDefinition::new(
            GraphNodeId::new(2),
            NodeKind::new("test.service-sink", 1),
            "service sink",
            ExecutionDomain::Service {
                device_id: DeviceId([1; 16]),
            },
            vec![port(1, "value")],
            Vec::new(),
            Vec::new(),
        );
        let cross_document = GraphDocument::try_new(
            0,
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
                service_sink_node,
            ],
            vec![wire(1, (1, 1), (2, 1))],
        )
        .unwrap();
        let service_sink_schema = NodeSchema::new(
            NodeKind::new("test.service-sink", 1),
            ExecutionDomainSet::SERVICE,
            vec![port(1, "value")],
            vec![required_sync(1)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        let cross_registry = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &cross_document,
            vec![source_schema(), service_sink_schema],
        )
        .unwrap();
        assert!(matches!(
            analyze_graph(&cross_document, &cross_registry),
            Err(GraphAnalysisError::CrossDomainSynchronous {
                wire,
                ..
            }) if wire == GraphWireId::new(1)
        ));
    }

    #[test]
    fn event_and_stream_queues_retain_full_policy_and_exact_byte_bounds() {
        const BOOL: GraphTypeId = GraphTypeId::new(1);
        const EVENT: GraphTypeId = GraphTypeId::new(2);
        const STREAM: GraphTypeId = GraphTypeId::new(3);
        let schema = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![
                TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    EVENT,
                    "event.bool",
                    TypeKind::Event {
                        payload: BOOL,
                        clock: CLOCK,
                    },
                ),
                TypeDefinition::new(
                    STREAM,
                    "stream.bool",
                    TypeKind::Stream {
                        sample: BOOL,
                        clock: CLOCK,
                        capacity: 4,
                    },
                ),
            ],
        )
        .unwrap();
        let event_port = |id, name| PortDefinition::new(GraphPortId::new(id), name, EVENT);
        let stream_port = |id, name| PortDefinition::new(GraphPortId::new(id), name, STREAM);
        let source = NodeDefinition::new(
            GraphNodeId::new(1),
            NodeKind::new("test.queue-source", 1),
            "queue source",
            ExecutionDomain::HostExact,
            Vec::new(),
            vec![event_port(1, "events"), stream_port(2, "samples")],
            Vec::new(),
        );
        let sink = NodeDefinition::new(
            GraphNodeId::new(2),
            NodeKind::new("test.queue-sink", 1),
            "queue sink",
            ExecutionDomain::Service {
                device_id: DeviceId([7; 16]),
            },
            vec![event_port(1, "events"), stream_port(2, "samples")],
            Vec::new(),
            Vec::new(),
        );
        let document = GraphDocument::try_new(
            0,
            schema,
            vec![clock()],
            vec![source, sink],
            vec![wire(1, (1, 1), (2, 1)), wire(2, (1, 2), (2, 2))],
        )
        .unwrap();
        let source_schema = NodeSchema::new(
            NodeKind::new("test.queue-source", 1),
            ExecutionDomainSet::HOST_EXACT,
            Vec::new(),
            Vec::new(),
            vec![event_port(1, "events"), stream_port(2, "samples")],
            Vec::new(),
            vec![dependency(1, &[]), dependency(2, &[])],
            Vec::new(),
            None,
        );
        let sink_schema = NodeSchema::new(
            NodeKind::new("test.queue-sink", 1),
            ExecutionDomainSet::SERVICE,
            vec![event_port(1, "events"), stream_port(2, "samples")],
            vec![
                NodeInputChannelContract::new(
                    GraphPortId::new(1),
                    InputConnectionRequirement::Required,
                    NodeInputChannelKind::EventQueue {
                        capacity: 3,
                        full_policy: ChannelFullPolicy::Fault,
                    },
                ),
                NodeInputChannelContract::new(
                    GraphPortId::new(2),
                    InputConnectionRequirement::Required,
                    NodeInputChannelKind::StreamQueue {
                        capacity: 4,
                        full_policy: ChannelFullPolicy::DropOldest,
                    },
                ),
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        let registry = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &document,
            vec![sink_schema.clone(), source_schema.clone()],
        )
        .unwrap();
        let report = analyze_graph(&document, &registry).unwrap();
        assert_eq!(report.channel_allocations().len(), 2);
        assert_eq!(report.channel_allocations()[0].maximum_item_bytes(), 21);
        assert_eq!(report.channel_allocations()[0].maximum_total_bytes(), 63);
        assert_eq!(report.channel_allocations()[1].maximum_item_bytes(), 21);
        assert_eq!(report.channel_allocations()[1].maximum_total_bytes(), 84);
        assert_eq!(report.total_channel_bytes(), 147);
        assert!(matches!(
            report.channel_allocations()[0].kind(),
            NodeInputChannelKind::EventQueue {
                capacity: 3,
                full_policy: ChannelFullPolicy::Fault,
            }
        ));

        let mut per_input_limits = GraphAnalysisLimits::interactive();
        per_input_limits.maximum_channel_bytes_per_input = 83;
        let per_input_registry = GraphNodeRegistry::try_new(
            per_input_limits,
            &document,
            vec![sink_schema.clone(), source_schema.clone()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&document, &per_input_registry),
            Err(GraphAnalysisError::LimitExceeded("per-input channel bytes"))
        );

        let mut total_limits = GraphAnalysisLimits::interactive();
        total_limits.maximum_total_channel_bytes = 146;
        let total_registry = GraphNodeRegistry::try_new(
            total_limits,
            &document,
            vec![sink_schema.clone(), source_schema.clone()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&document, &total_registry),
            Err(GraphAnalysisError::LimitExceeded("total channel bytes"))
        );

        let mut capacity_limits = GraphAnalysisLimits::interactive();
        capacity_limits.maximum_queue_items_per_input = 3;
        assert!(matches!(
            GraphNodeRegistry::try_new(
                capacity_limits,
                &document,
                vec![sink_schema.clone(), source_schema]
            ),
            Err(NodeRegistryError::InvalidSchema {
                aspect: "queue capacity",
                ..
            })
        ));

        let mut invalid_sink = sink_schema;
        invalid_sink.input_channels[1] = NodeInputChannelContract::new(
            GraphPortId::new(2),
            InputConnectionRequirement::Required,
            NodeInputChannelKind::StreamQueue {
                capacity: 5,
                full_policy: ChannelFullPolicy::Backpressure,
            },
        );
        assert!(matches!(
            GraphNodeRegistry::try_new(
                GraphAnalysisLimits::interactive(),
                &document,
                vec![invalid_sink]
            ),
            Err(NodeRegistryError::InvalidSchema {
                aspect: "stream queue capacity",
                ..
            })
        ));
    }

    fn rate_transition_fixture(
        shared_root: bool,
        queue_capacity: u32,
    ) -> (GraphDocument, NodeSchema, NodeSchema) {
        const BOOL: GraphTypeId = GraphTypeId::new(1);
        const SOURCE_STREAM: GraphTypeId = GraphTypeId::new(2);
        const TARGET_STREAM: GraphTypeId = GraphTypeId::new(3);
        const ROOT: GraphClockId = GraphClockId::new(1);
        const SOURCE: GraphClockId = GraphClockId::new(2);
        const TARGET: GraphClockId = GraphClockId::new(3);

        let schema = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![
                TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    SOURCE_STREAM,
                    "stream.source-bool",
                    TypeKind::Stream {
                        sample: BOOL,
                        clock: SOURCE,
                        capacity: 8,
                    },
                ),
                TypeDefinition::new(
                    TARGET_STREAM,
                    "stream.target-bool",
                    TypeKind::Stream {
                        sample: BOOL,
                        clock: TARGET,
                        capacity: 8,
                    },
                ),
            ],
        )
        .unwrap();
        let target_clock = if shared_root {
            ClockDefinition::new(
                TARGET,
                "host.target-600hz",
                ClockKind::Derived {
                    source: ROOT,
                    numerator: 1,
                    denominator: 2,
                },
            )
        } else {
            ClockDefinition::new(
                TARGET,
                "host.independent-600hz",
                ClockKind::HostMonotonic {
                    ticks_per_second: 600,
                },
            )
        };
        let clocks = vec![
            ClockDefinition::new(
                ROOT,
                "host.root-1200hz",
                ClockKind::HostMonotonic {
                    ticks_per_second: 1_200,
                },
            ),
            ClockDefinition::new(
                SOURCE,
                "host.source-1000hz",
                ClockKind::Derived {
                    source: ROOT,
                    numerator: 5,
                    denominator: 6,
                },
            ),
            target_clock,
        ];
        let source_port = |id, name| PortDefinition::new(GraphPortId::new(id), name, SOURCE_STREAM);
        let target_port = |id, name| PortDefinition::new(GraphPortId::new(id), name, TARGET_STREAM);
        let source_node = NodeDefinition::new(
            GraphNodeId::new(1),
            NodeKind::new("test.rate-source", 1),
            "rate source",
            ExecutionDomain::HostExact,
            Vec::new(),
            vec![source_port(1, "samples")],
            Vec::new(),
        );
        let transition_node = NodeDefinition::new(
            GraphNodeId::new(2),
            NodeKind::new("test.rate-transition", 1),
            "rate transition",
            ExecutionDomain::HostExact,
            vec![source_port(1, "source")],
            vec![target_port(2, "target")],
            Vec::new(),
        );
        let document = GraphDocument::try_new(
            0,
            schema,
            clocks,
            vec![source_node, transition_node],
            vec![wire(1, (1, 1), (2, 1))],
        )
        .unwrap();
        let source_schema = NodeSchema::new(
            NodeKind::new("test.rate-source", 1),
            ExecutionDomainSet::HOST_EXACT,
            Vec::new(),
            Vec::new(),
            vec![source_port(1, "samples")],
            Vec::new(),
            vec![dependency(1, &[])],
            Vec::new(),
            None,
        );
        let transition_schema = NodeSchema::new(
            NodeKind::new("test.rate-transition", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![source_port(1, "source")],
            vec![NodeInputChannelContract::new(
                GraphPortId::new(1),
                InputConnectionRequirement::Required,
                NodeInputChannelKind::StreamQueue {
                    capacity: queue_capacity,
                    full_policy: ChannelFullPolicy::Fault,
                },
            )],
            vec![target_port(2, "target")],
            Vec::new(),
            vec![dependency(2, &[1])],
            vec![NodeRateTransitionContract::new(
                GraphPortId::new(1),
                GraphPortId::new(2),
                RateTransitionKind::LatestAtOrBeforeSourceFirst,
            )],
            None,
        );
        (document, source_schema, transition_schema)
    }

    #[test]
    fn exact_rate_transition_reports_smallest_pattern_and_queue_requirement() {
        let (document, source_schema, transition_schema) = rate_transition_fixture(true, 2);
        let registry = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &document,
            vec![transition_schema, source_schema],
        )
        .unwrap();
        let report = analyze_graph(&document, &registry).unwrap();
        assert_eq!(report.clock_rates().len(), 3);
        assert_eq!(
            report.clock_rates()[0].ticks_per_second(),
            &Rational::from(1_200_u64)
        );
        assert_eq!(
            report.clock_rates()[1].ticks_per_second(),
            &Rational::from(1_000_u64)
        );
        assert_eq!(
            report.clock_rates()[2].ticks_per_second(),
            &Rational::from(600_u64)
        );
        assert!(
            report
                .clock_rates()
                .iter()
                .all(|rate| rate.root() == GraphClockId::new(1))
        );
        assert_eq!(report.rate_transitions().len(), 1);
        let transition = report.rate_transitions()[0];
        assert_eq!(transition.node(), GraphNodeId::new(2));
        assert_eq!(transition.input(), GraphPortId::new(1));
        assert_eq!(transition.output(), GraphPortId::new(2));
        assert_eq!(transition.source_clock(), GraphClockId::new(2));
        assert_eq!(transition.target_clock(), GraphClockId::new(3));
        assert_eq!(transition.root_clock(), GraphClockId::new(1));
        assert_eq!(transition.source_ticks_per_pattern(), 5);
        assert_eq!(transition.target_ticks_per_pattern(), 3);
        assert_eq!(transition.required_input_capacity(), 2);
        assert_eq!(transition.retained_sample_bytes(), 5);
        assert_eq!(report.total_rate_transition_state_bytes(), 5);
        assert_eq!(
            transition.kind(),
            RateTransitionKind::LatestAtOrBeforeSourceFirst
        );
    }

    #[test]
    fn rate_transitions_reject_implicit_clocks_small_queues_and_large_patterns() {
        let (document, source_schema, transition_schema) = rate_transition_fixture(true, 2);
        let mut implicit = transition_schema.clone();
        implicit.rate_transitions.clear();
        assert!(matches!(
            GraphNodeRegistry::try_new(
                GraphAnalysisLimits::interactive(),
                &document,
                vec![implicit, source_schema.clone()]
            ),
            Err(NodeRegistryError::InvalidSchema {
                aspect: "cross-clock dependency",
                ..
            })
        ));

        let (small_document, small_source, small_transition) = rate_transition_fixture(true, 1);
        let small_registry = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &small_document,
            vec![small_transition, small_source],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&small_document, &small_registry),
            Err(GraphAnalysisError::RateTransitionQueueTooSmall {
                node: GraphNodeId::new(2),
                input: GraphPortId::new(1),
                declared: 1,
                required: 2,
            })
        );

        let mut pattern_limits = GraphAnalysisLimits::interactive();
        pattern_limits.maximum_rate_pattern_ticks = 4;
        let pattern_registry = GraphNodeRegistry::try_new(
            pattern_limits,
            &document,
            vec![transition_schema.clone(), source_schema.clone()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&document, &pattern_registry),
            Err(GraphAnalysisError::RateTransitionPatternTooLarge {
                node: GraphNodeId::new(2),
                input: GraphPortId::new(1),
                output: GraphPortId::new(2),
            })
        );

        let mut doubled_nodes = document.nodes().to_vec();
        doubled_nodes.push(NodeDefinition::new(
            GraphNodeId::new(3),
            NodeKind::new("test.rate-transition", 1),
            "second rate transition",
            ExecutionDomain::HostExact,
            vec![PortDefinition::new(
                GraphPortId::new(1),
                "source",
                GraphTypeId::new(2),
            )],
            vec![PortDefinition::new(
                GraphPortId::new(2),
                "target",
                GraphTypeId::new(3),
            )],
            Vec::new(),
        ));
        let mut doubled_wires = document.wires().to_vec();
        doubled_wires.push(wire(2, (1, 1), (3, 1)));
        let doubled_document = GraphDocument::try_new(
            document.revision(),
            document.schema().clone(),
            document.clocks().to_vec(),
            doubled_nodes,
            doubled_wires,
        )
        .unwrap();
        let mut count_limits = GraphAnalysisLimits::interactive();
        count_limits.maximum_rate_transitions = 1;
        let count_registry = GraphNodeRegistry::try_new(
            count_limits,
            &doubled_document,
            vec![transition_schema.clone(), source_schema.clone()],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&doubled_document, &count_registry),
            Err(GraphAnalysisError::LimitExceeded("rate transition count"))
        );

        let mut state_limits = GraphAnalysisLimits::interactive();
        state_limits.maximum_total_rate_transition_state_bytes = 4;
        let state_registry = GraphNodeRegistry::try_new(
            state_limits,
            &document,
            vec![transition_schema, source_schema],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&document, &state_registry),
            Err(GraphAnalysisError::LimitExceeded(
                "rate transition state bytes"
            ))
        );
    }

    #[test]
    fn rate_transition_rejects_independent_clock_roots() {
        let (document, source_schema, transition_schema) = rate_transition_fixture(false, 2);
        let registry = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &document,
            vec![transition_schema, source_schema],
        )
        .unwrap();
        assert_eq!(
            analyze_graph(&document, &registry),
            Err(GraphAnalysisError::RateTransitionRootMismatch {
                node: GraphNodeId::new(2),
                input: GraphPortId::new(1),
                output: GraphPortId::new(2),
                source_root: GraphClockId::new(1),
                target_root: GraphClockId::new(3),
            })
        );
    }
}
