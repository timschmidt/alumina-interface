//! Deterministic host simulation for a deliberately fixed Stream/rate subset.
//!
//! The structural document and audited semantic registry still do not grant an
//! arbitrary implementation. This module adds a second, explicit registry for
//! three host-simulation behaviors: externally supplied Stream sources,
//! latest-at-or-before rate transitions, and Stream sinks. It evaluates no
//! firmware resource and grants no deployment authority.

use core::cmp::Ordering;
use core::fmt;
use std::collections::BTreeMap;

use alumina_protocol::Digest;
use alumina_storage::sha256;
use hyperreal::Rational;

use super::{
    ChannelFullPolicy, ExecutionDomain, GraphAnalysis, GraphAnalysisError, GraphAnalysisLimits,
    GraphClockId, GraphDocument, GraphNodeId, GraphNodeRegistry, GraphPortId, GraphRateTransition,
    GraphSchema, GraphTypeId, GraphWireError, InputConnectionRequirement, NodeInputChannelKind,
    NodeKind, NodeRateTransitionContract, NodeSchema, RateTransitionKind, TypeKind,
    TypedGraphValue, WireEndpoint, analyze_graph, encode_graph_document,
};

/// Fixed host behavior admitted for one audited node kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphSimulationNodeKind {
    /// One Stream output is supplied by caller-owned exact samples.
    ExternalStreamSource {
        /// External output port.
        output: GraphPortId,
    },
    /// Execute one audited latest-at-or-before Stream rate transition.
    LatestRateTransition {
        /// Source Stream input.
        input: GraphPortId,
        /// Target Stream output.
        output: GraphPortId,
    },
    /// Require one Stream input and consume it without a modeled side effect.
    StreamSink {
        /// Consumed Stream input.
        input: GraphPortId,
    },
}

/// Exact node-kind to fixed host-simulation behavior binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphSimulationImplementation {
    kind: NodeKind,
    behavior: GraphSimulationNodeKind,
}

impl GraphSimulationImplementation {
    /// Construct one implementation binding.
    pub fn new(kind: NodeKind, behavior: GraphSimulationNodeKind) -> Self {
        Self { kind, behavior }
    }

    /// Return the exact opaque node kind/version being implemented.
    pub const fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Return the fixed host behavior.
    pub const fn behavior(&self) -> GraphSimulationNodeKind {
        self.behavior
    }
}

/// Canonical fixed implementation registry above one audited semantic registry.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphSimulationRegistry {
    semantic: GraphNodeRegistry,
    implementations: Vec<GraphSimulationImplementation>,
    digest: Digest,
}

impl GraphSimulationRegistry {
    /// Validate, canonicalize, and identify fixed simulation bindings.
    pub fn try_new(
        semantic: GraphNodeRegistry,
        mut implementations: Vec<GraphSimulationImplementation>,
    ) -> Result<Self, GraphSimulationError> {
        implementations.sort_unstable_by(|left, right| compare_kind(&left.kind, &right.kind));
        let mut previous: Option<&NodeKind> = None;
        for implementation in &implementations {
            if previous.is_some_and(|kind| compare_kind(kind, &implementation.kind).is_eq()) {
                return Err(GraphSimulationError::DuplicateImplementation(
                    implementation.kind.clone(),
                ));
            }
            previous = Some(&implementation.kind);
            let schema = semantic.schema(&implementation.kind).ok_or_else(|| {
                GraphSimulationError::UnknownImplementationKind(implementation.kind.clone())
            })?;
            validate_implementation(schema, implementation.behavior, semantic.context_schema())?;
        }
        let digest = simulation_registry_digest(&semantic, &implementations)?;
        Ok(Self {
            semantic,
            implementations,
            digest,
        })
    }

    /// Borrow the audited semantic authority below this implementation set.
    pub const fn semantic_registry(&self) -> &GraphNodeRegistry {
        &self.semantic
    }

    /// Borrow bindings in canonical kind/version order.
    pub fn implementations(&self) -> &[GraphSimulationImplementation] {
        &self.implementations
    }

    /// Return the canonical SHA-256 identity of semantic and implementation facts.
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// Resolve one exact kind/version.
    pub fn implementation(&self, kind: &NodeKind) -> Option<&GraphSimulationImplementation> {
        self.implementations
            .binary_search_by(|implementation| compare_kind(&implementation.kind, kind))
            .ok()
            .map(|index| &self.implementations[index])
    }
}

/// Bounded deterministic simulation policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphSimulationLimits {
    /// Maximum caller-supplied Stream samples.
    pub maximum_external_samples: usize,
    /// Maximum complete input/output trace entries.
    pub maximum_trace_entries: usize,
    /// Maximum generated target ticks for any one transition.
    pub maximum_ticks_per_transition: u64,
    /// Maximum inclusive horizon in root-clock ticks.
    pub maximum_root_ticks: u64,
    /// Maximum canonical trace bytes accepted by trace replay.
    pub maximum_trace_bytes: usize,
}

impl GraphSimulationLimits {
    /// First-release bounded host policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_external_samples: 65_536,
            maximum_trace_entries: 131_072,
            maximum_ticks_per_transition: 1_000_000,
            maximum_root_ticks: 1_000_000,
            maximum_trace_bytes: 16 * 1024 * 1024,
        }
    }

    pub(super) fn validate(self) -> Result<(), GraphSimulationError> {
        if self.maximum_external_samples == 0
            || self.maximum_trace_entries == 0
            || self.maximum_ticks_per_transition == 0
            || self.maximum_root_ticks == 0
            || self.maximum_trace_bytes == 0
        {
            Err(GraphSimulationError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphSimulationLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Inclusive exact simulation horizon expressed in one independent root clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphSimulationHorizon {
    root_clock: GraphClockId,
    inclusive_root_tick: u64,
}

impl GraphSimulationHorizon {
    /// Construct an inclusive root-clock horizon.
    pub const fn new(root_clock: GraphClockId, inclusive_root_tick: u64) -> Self {
        Self {
            root_clock,
            inclusive_root_tick,
        }
    }

    /// Return the independent root clock.
    pub const fn root_clock(self) -> GraphClockId {
        self.root_clock
    }

    /// Return the last included root tick.
    pub const fn inclusive_root_tick(self) -> u64 {
        self.inclusive_root_tick
    }
}

/// One caller-owned exact sample emitted by an external Stream source.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalStreamSample {
    source: WireEndpoint,
    clock_tick: u64,
    sequence: u64,
    value: TypedGraphValue,
}

impl ExternalStreamSample {
    /// Construct one timestamped external source sample.
    pub fn new(
        source: WireEndpoint,
        clock_tick: u64,
        sequence: u64,
        value: TypedGraphValue,
    ) -> Self {
        Self {
            source,
            clock_tick,
            sequence,
            value,
        }
    }

    /// Return the implemented source output.
    pub const fn source(&self) -> WireEndpoint {
        self.source
    }

    /// Return the source-clock tick.
    pub const fn clock_tick(&self) -> u64 {
        self.clock_tick
    }

    /// Return the caller-supplied monotonic sequence.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Borrow the exact typed sample payload.
    pub const fn value(&self) -> &TypedGraphValue {
        &self.value
    }
}

/// Origin of one deterministic trace sample.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GraphTraceEntryKind {
    /// Caller-owned external Stream sample.
    ExternalInput,
    /// Sample emitted by an admitted fixed node implementation.
    NodeOutput,
}

/// One exact timestamped Stream sample in a deterministic host trace.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphTraceEntry {
    kind: GraphTraceEntryKind,
    endpoint: WireEndpoint,
    clock: GraphClockId,
    clock_tick: u64,
    sequence: u64,
    value: TypedGraphValue,
}

impl GraphTraceEntry {
    /// Return whether this is external input or modeled node output.
    pub const fn kind(&self) -> GraphTraceEntryKind {
        self.kind
    }

    /// Return the observed output endpoint.
    pub const fn endpoint(&self) -> WireEndpoint {
        self.endpoint
    }

    /// Return the timestamp clock.
    pub const fn clock(&self) -> GraphClockId {
        self.clock
    }

    /// Return the tick on [`Self::clock`].
    pub const fn clock_tick(&self) -> u64 {
        self.clock_tick
    }

    /// Return the monotonic stream sequence.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Borrow the exact typed sample payload.
    pub const fn value(&self) -> &TypedGraphValue {
        &self.value
    }
}

/// Deterministic in-memory host result ready for canonical trace encoding.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphSimulation {
    graph_digest: Digest,
    registry_digest: Digest,
    horizon: GraphSimulationHorizon,
    entries: Vec<GraphTraceEntry>,
}

impl GraphSimulation {
    /// Return the exact canonical graph-document identity.
    pub const fn graph_digest(&self) -> Digest {
        self.graph_digest
    }

    /// Return the exact semantic/implementation registry identity.
    pub const fn registry_digest(&self) -> Digest {
        self.registry_digest
    }

    /// Return the inclusive simulation horizon.
    pub const fn horizon(&self) -> GraphSimulationHorizon {
        self.horizon
    }

    /// Borrow entries in canonical exact-time order.
    pub fn entries(&self) -> &[GraphTraceEntry] {
        &self.entries
    }
}

/// Failure at fixed host implementation admission or deterministic evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphSimulationError {
    /// One simulation limit was zero.
    ZeroLimit,
    /// A bounded collection or horizon exceeded policy.
    LimitExceeded(&'static str),
    /// Canonical graph encoding failed.
    Graph(GraphWireError),
    /// Audited semantic analysis failed.
    Analysis(GraphAnalysisError),
    /// Two implementations claimed one exact node kind/version.
    DuplicateImplementation(NodeKind),
    /// An implementation named no audited schema.
    UnknownImplementationKind(NodeKind),
    /// One fixed implementation contradicted its audited schema.
    InvalidImplementation {
        /// Exact kind/version.
        kind: NodeKind,
        /// Rejected implementation aspect.
        aspect: &'static str,
    },
    /// A document node had no fixed host implementation.
    UnimplementedNode {
        /// Exact node instance.
        node: GraphNodeId,
        /// Opaque kind/version.
        kind: NodeKind,
    },
    /// The fixed host simulator will not model a device execution placement.
    NonHostDomain(GraphNodeId),
    /// The requested horizon clock was absent or was not an independent root.
    InvalidHorizonRoot(GraphClockId),
    /// A modeled Stream clock belonged to another independent root.
    ClockOutsideHorizonRoot {
        /// Modeled Stream clock.
        clock: GraphClockId,
        /// Independent root resolved for `clock`.
        clock_root: GraphClockId,
        /// Root selected by the simulation horizon.
        horizon_root: GraphClockId,
    },
    /// An external sample did not name a declared external source output.
    UnknownExternalSource(WireEndpoint),
    /// An external sample's literal type did not match its Stream sample type.
    ExternalSampleType {
        /// Source endpoint.
        source: WireEndpoint,
        /// Required literal type.
        expected: GraphTypeId,
        /// Received literal type.
        received: GraphTypeId,
    },
    /// External ticks or sequences were not strictly monotonic per source.
    ExternalSampleOrder(WireEndpoint),
    /// A sample occurred after the inclusive simulation horizon.
    ExternalSampleAfterHorizon(WireEndpoint),
    /// A transition or sink input had no simulated output source.
    UnavailableStreamSource(WireEndpoint),
    /// Latest-at-or-before had no source sample at its first target tick.
    MissingInitialSample(WireEndpoint),
    /// Due samples exceeded the declared input queue capacity.
    QueueCapacityExceeded {
        /// Transition input endpoint.
        input: WireEndpoint,
        /// Declared queue capacity.
        capacity: u32,
        /// Simultaneously pending due samples.
        pending: usize,
    },
    /// Canonical registry identity could not represent a host integer.
    RegistryEncoding,
}

impl From<GraphWireError> for GraphSimulationError {
    fn from(value: GraphWireError) -> Self {
        Self::Graph(value)
    }
}

impl From<GraphAnalysisError> for GraphSimulationError {
    fn from(value: GraphAnalysisError) -> Self {
        Self::Analysis(value)
    }
}

impl fmt::Display for GraphSimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph simulation limit is zero"),
            Self::LimitExceeded(name) => {
                write!(formatter, "graph simulation {name} exceeds policy")
            }
            Self::Graph(error) => write!(formatter, "graph simulation encoding rejected: {error}"),
            Self::Analysis(error) => {
                write!(formatter, "graph simulation analysis rejected: {error}")
            }
            Self::DuplicateImplementation(kind) => write!(
                formatter,
                "graph simulation implementation {}@{} is duplicated",
                kind.name(),
                kind.version()
            ),
            Self::UnknownImplementationKind(kind) => write!(
                formatter,
                "graph simulation implementation {}@{} has no audited schema",
                kind.name(),
                kind.version()
            ),
            Self::InvalidImplementation { kind, aspect } => write!(
                formatter,
                "graph simulation implementation {}@{} contradicts {aspect}",
                kind.name(),
                kind.version()
            ),
            Self::UnimplementedNode { node, kind } => write!(
                formatter,
                "graph node {node:?} kind {}@{} has no host simulation implementation",
                kind.name(),
                kind.version()
            ),
            Self::NonHostDomain(node) => {
                write!(formatter, "graph node {node:?} is not in HostExact")
            }
            Self::InvalidHorizonRoot(clock) => {
                write!(
                    formatter,
                    "graph simulation horizon clock {clock:?} is not a root"
                )
            }
            Self::ClockOutsideHorizonRoot {
                clock,
                clock_root,
                horizon_root,
            } => write!(
                formatter,
                "graph simulation clock {clock:?} has root {clock_root:?}, not horizon root {horizon_root:?}"
            ),
            Self::UnknownExternalSource(source) => {
                write!(
                    formatter,
                    "external sample source {source:?} is not implemented"
                )
            }
            Self::ExternalSampleType {
                source,
                expected,
                received,
            } => write!(
                formatter,
                "external sample {source:?} has type {received:?}, expected {expected:?}"
            ),
            Self::ExternalSampleOrder(source) => {
                write!(
                    formatter,
                    "external samples for {source:?} are not monotonic"
                )
            }
            Self::ExternalSampleAfterHorizon(source) => {
                write!(
                    formatter,
                    "external sample for {source:?} is after the horizon"
                )
            }
            Self::UnavailableStreamSource(input) => {
                write!(
                    formatter,
                    "graph simulation input {input:?} has no available stream"
                )
            }
            Self::MissingInitialSample(input) => {
                write!(
                    formatter,
                    "graph simulation input {input:?} has no run-start sample"
                )
            }
            Self::QueueCapacityExceeded {
                input,
                capacity,
                pending,
            } => write!(
                formatter,
                "graph simulation input {input:?} has {pending} due samples for capacity {capacity}"
            ),
            Self::RegistryEncoding => {
                formatter.write_str("graph simulation registry encoding failed")
            }
        }
    }
}

impl std::error::Error for GraphSimulationError {}

#[derive(Clone)]
struct RuntimeSample {
    clock_tick: u64,
    sequence: u64,
    value: TypedGraphValue,
}

#[derive(Clone, Copy)]
struct StreamPort {
    sample_type: GraphTypeId,
    clock: GraphClockId,
}

/// Evaluate the fixed external-source/rate-transition/sink subset exactly.
pub fn simulate_graph(
    document: &GraphDocument,
    registry: &GraphSimulationRegistry,
    horizon: GraphSimulationHorizon,
    external_samples: &[ExternalStreamSample],
    limits: GraphSimulationLimits,
) -> Result<GraphSimulation, GraphSimulationError> {
    limits.validate()?;
    if external_samples.len() > limits.maximum_external_samples {
        return Err(GraphSimulationError::LimitExceeded("external sample count"));
    }
    if horizon.inclusive_root_tick > limits.maximum_root_ticks {
        return Err(GraphSimulationError::LimitExceeded("root tick horizon"));
    }
    let analysis = analyze_graph(document, &registry.semantic)?;
    let root_rate = analysis
        .clock_rate(horizon.root_clock)
        .filter(|rate| rate.root() == horizon.root_clock)
        .ok_or(GraphSimulationError::InvalidHorizonRoot(horizon.root_clock))?;
    let graph_digest = encode_graph_document(document)?.digest();

    let mut source_ports = BTreeMap::new();
    let mut transition_nodes = Vec::new();
    let mut sink_nodes = Vec::new();
    for node in document.nodes() {
        if node.domain() != ExecutionDomain::HostExact {
            return Err(GraphSimulationError::NonHostDomain(node.id()));
        }
        let implementation = registry.implementation(node.kind()).ok_or_else(|| {
            GraphSimulationError::UnimplementedNode {
                node: node.id(),
                kind: node.kind().clone(),
            }
        })?;
        match implementation.behavior {
            GraphSimulationNodeKind::ExternalStreamSource { output } => {
                let endpoint = WireEndpoint {
                    node: node.id(),
                    port: output,
                };
                let port = stream_output(document, endpoint)?;
                clock_rate_in_horizon(&analysis, root_rate, port.clock)?;
                source_ports.insert(endpoint, port);
            }
            GraphSimulationNodeKind::LatestRateTransition { input, output } => {
                transition_nodes.push((node.id(), input, output));
            }
            GraphSimulationNodeKind::StreamSink { input } => {
                sink_nodes.push(WireEndpoint {
                    node: node.id(),
                    port: input,
                });
            }
        }
    }

    let mut sorted_external = external_samples.to_vec();
    sorted_external.sort_by_key(|sample| (sample.source, sample.clock_tick, sample.sequence));
    let mut streams: BTreeMap<WireEndpoint, Vec<RuntimeSample>> = source_ports
        .keys()
        .copied()
        .map(|endpoint| (endpoint, Vec::new()))
        .collect();
    let mut entries = Vec::new();
    let mut prior_by_source: BTreeMap<WireEndpoint, (u64, u64)> = BTreeMap::new();
    for sample in sorted_external {
        let port = source_ports
            .get(&sample.source)
            .copied()
            .ok_or(GraphSimulationError::UnknownExternalSource(sample.source))?;
        if sample.value.value_type() != port.sample_type {
            return Err(GraphSimulationError::ExternalSampleType {
                source: sample.source,
                expected: port.sample_type,
                received: sample.value.value_type(),
            });
        }
        if let Some((prior_tick, prior_sequence)) =
            prior_by_source.insert(sample.source, (sample.clock_tick, sample.sequence))
            && (sample.clock_tick <= prior_tick || sample.sequence <= prior_sequence)
        {
            return Err(GraphSimulationError::ExternalSampleOrder(sample.source));
        }
        let time = root_time(&analysis, root_rate, port.clock, sample.clock_tick)?;
        if time > Rational::from(horizon.inclusive_root_tick) {
            return Err(GraphSimulationError::ExternalSampleAfterHorizon(
                sample.source,
            ));
        }
        push_trace(
            &mut entries,
            GraphTraceEntry {
                kind: GraphTraceEntryKind::ExternalInput,
                endpoint: sample.source,
                clock: port.clock,
                clock_tick: sample.clock_tick,
                sequence: sample.sequence,
                value: sample.value.clone(),
            },
            limits,
        )?;
        streams
            .get_mut(&sample.source)
            .ok_or(GraphSimulationError::UnknownExternalSource(sample.source))?
            .push(RuntimeSample {
                clock_tick: sample.clock_tick,
                sequence: sample.sequence,
                value: sample.value,
            });
    }

    let mut pending = transition_nodes;
    while !pending.is_empty() {
        let mut deferred = Vec::new();
        let mut progressed = false;
        for (node, input, output) in pending {
            let input_endpoint = WireEndpoint { node, port: input };
            let source = source_for_input(document, input_endpoint)?;
            let Some(source_samples) = streams.get(&source) else {
                deferred.push((node, input, output));
                continue;
            };
            let report = analysis
                .rate_transitions()
                .iter()
                .find(|transition| {
                    transition.node() == node
                        && transition.input() == input
                        && transition.output() == output
                })
                .copied()
                .ok_or(GraphSimulationError::UnavailableStreamSource(
                    input_endpoint,
                ))?;
            let schema = registry
                .semantic
                .schema(
                    document
                        .node(node)
                        .ok_or(GraphSimulationError::UnavailableStreamSource(
                            input_endpoint,
                        ))?
                        .kind(),
                )
                .ok_or(GraphSimulationError::UnavailableStreamSource(
                    input_endpoint,
                ))?;
            let capacity = stream_queue_capacity(schema, input).ok_or(
                GraphSimulationError::UnavailableStreamSource(input_endpoint),
            )?;
            let target_endpoint = WireEndpoint { node, port: output };
            let generated = simulate_transition(
                &analysis,
                root_rate,
                horizon,
                input_endpoint,
                target_endpoint,
                report,
                capacity,
                source_samples,
                limits,
            )?;
            for sample in &generated {
                push_trace(
                    &mut entries,
                    GraphTraceEntry {
                        kind: GraphTraceEntryKind::NodeOutput,
                        endpoint: target_endpoint,
                        clock: report.target_clock(),
                        clock_tick: sample.clock_tick,
                        sequence: sample.sequence,
                        value: sample.value.clone(),
                    },
                    limits,
                )?;
            }
            streams.insert(target_endpoint, generated);
            progressed = true;
        }
        if !progressed {
            let (node, input, _) = deferred[0];
            return Err(GraphSimulationError::UnavailableStreamSource(
                WireEndpoint { node, port: input },
            ));
        }
        pending = deferred;
    }

    for sink in sink_nodes {
        let source = source_for_input(document, sink)?;
        if !streams.contains_key(&source) {
            return Err(GraphSimulationError::UnavailableStreamSource(sink));
        }
    }

    let mut timed_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        timed_entries.push((
            root_time(&analysis, root_rate, entry.clock, entry.clock_tick)?,
            entry,
        ));
    }
    timed_entries.sort_by(|left, right| {
        left.0
            .partial_cmp(&right.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.1.kind.cmp(&right.1.kind))
            .then_with(|| left.1.endpoint.cmp(&right.1.endpoint))
            .then_with(|| left.1.sequence.cmp(&right.1.sequence))
    });
    let entries = timed_entries.into_iter().map(|(_, entry)| entry).collect();
    Ok(GraphSimulation {
        graph_digest,
        registry_digest: registry.digest,
        horizon,
        entries,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the exact transition boundary keeps all scheduling authorities explicit"
)]
fn simulate_transition(
    analysis: &GraphAnalysis,
    root_rate: &super::GraphClockRate,
    horizon: GraphSimulationHorizon,
    input: WireEndpoint,
    _output: WireEndpoint,
    transition: GraphRateTransition,
    capacity: u32,
    source: &[RuntimeSample],
    limits: GraphSimulationLimits,
) -> Result<Vec<RuntimeSample>, GraphSimulationError> {
    let horizon_time = Rational::from(horizon.inclusive_root_tick);
    let mut generated = Vec::new();
    let mut source_index = 0_usize;
    let mut held: Option<TypedGraphValue> = None;
    let mut target_tick = 0_u64;
    loop {
        let target_time = root_time(analysis, root_rate, transition.target_clock(), target_tick)?;
        if target_time > horizon_time {
            break;
        }
        if generated.len()
            >= usize::try_from(limits.maximum_ticks_per_transition).unwrap_or(usize::MAX)
        {
            return Err(GraphSimulationError::LimitExceeded("ticks per transition"));
        }
        let interval_start = source_index;
        while let Some(sample) = source.get(source_index) {
            let source_time = root_time(
                analysis,
                root_rate,
                transition.source_clock(),
                sample.clock_tick,
            )?;
            if source_time > target_time {
                break;
            }
            held = Some(sample.value.clone());
            source_index += 1;
        }
        let pending = source_index - interval_start;
        if pending > capacity as usize {
            return Err(GraphSimulationError::QueueCapacityExceeded {
                input,
                capacity,
                pending,
            });
        }
        let value = held
            .clone()
            .ok_or(GraphSimulationError::MissingInitialSample(input))?;
        generated.push(RuntimeSample {
            clock_tick: target_tick,
            sequence: target_tick,
            value,
        });
        target_tick = target_tick
            .checked_add(1)
            .ok_or(GraphSimulationError::LimitExceeded("target clock tick"))?;
    }
    let pending = source.len().saturating_sub(source_index);
    if pending > capacity as usize {
        return Err(GraphSimulationError::QueueCapacityExceeded {
            input,
            capacity,
            pending,
        });
    }
    Ok(generated)
}

fn push_trace(
    entries: &mut Vec<GraphTraceEntry>,
    entry: GraphTraceEntry,
    limits: GraphSimulationLimits,
) -> Result<(), GraphSimulationError> {
    if entries.len() >= limits.maximum_trace_entries {
        return Err(GraphSimulationError::LimitExceeded("trace entry count"));
    }
    entries.push(entry);
    Ok(())
}

fn root_time(
    analysis: &GraphAnalysis,
    root_rate: &super::GraphClockRate,
    clock: GraphClockId,
    tick: u64,
) -> Result<Rational, GraphSimulationError> {
    let rate = clock_rate_in_horizon(analysis, root_rate, clock)?;
    Ok(Rational::from(tick) * root_rate.ticks_per_second().clone()
        / rate.ticks_per_second().clone())
}

fn clock_rate_in_horizon<'a>(
    analysis: &'a GraphAnalysis,
    root_rate: &super::GraphClockRate,
    clock: GraphClockId,
) -> Result<&'a super::GraphClockRate, GraphSimulationError> {
    let rate = analysis
        .clock_rate(clock)
        .ok_or(GraphSimulationError::InvalidHorizonRoot(clock))?;
    if rate.root() != root_rate.clock() {
        return Err(GraphSimulationError::ClockOutsideHorizonRoot {
            clock,
            clock_root: rate.root(),
            horizon_root: root_rate.clock(),
        });
    }
    Ok(rate)
}

fn source_for_input(
    document: &GraphDocument,
    input: WireEndpoint,
) -> Result<WireEndpoint, GraphSimulationError> {
    document
        .wires()
        .iter()
        .find(|wire| wire.target() == input)
        .map(|wire| wire.source())
        .ok_or(GraphSimulationError::UnavailableStreamSource(input))
}

fn stream_output(
    document: &GraphDocument,
    endpoint: WireEndpoint,
) -> Result<StreamPort, GraphSimulationError> {
    let node = document
        .node(endpoint.node)
        .ok_or(GraphSimulationError::UnknownExternalSource(endpoint))?;
    let port = node
        .outputs()
        .iter()
        .find(|port| port.id() == endpoint.port)
        .ok_or(GraphSimulationError::UnknownExternalSource(endpoint))?;
    stream_port(document.schema(), port.value_type())
        .ok_or(GraphSimulationError::UnknownExternalSource(endpoint))
}

fn stream_port(schema: &GraphSchema, value_type: GraphTypeId) -> Option<StreamPort> {
    match schema.value_type(value_type)?.kind() {
        TypeKind::Stream { sample, clock, .. } => Some(StreamPort {
            sample_type: *sample,
            clock: *clock,
        }),
        _ => None,
    }
}

fn stream_queue_capacity(schema: &NodeSchema, input: GraphPortId) -> Option<u32> {
    schema
        .input_channels()
        .iter()
        .find(|channel| channel.port() == input)
        .and_then(|channel| match channel.kind() {
            NodeInputChannelKind::StreamQueue { capacity, .. } => Some(capacity),
            NodeInputChannelKind::Synchronous | NodeInputChannelKind::EventQueue { .. } => None,
        })
}

fn validate_implementation(
    schema: &NodeSchema,
    behavior: GraphSimulationNodeKind,
    values: &GraphSchema,
) -> Result<(), GraphSimulationError> {
    let invalid = |aspect| GraphSimulationError::InvalidImplementation {
        kind: schema.kind().clone(),
        aspect,
    };
    if !schema.parameters().is_empty() || schema.state().is_some() {
        return Err(invalid("parameter/state-free fixed subset"));
    }
    match behavior {
        GraphSimulationNodeKind::ExternalStreamSource { output } => {
            if !schema.inputs().is_empty()
                || schema.outputs().len() != 1
                || schema.outputs()[0].id() != output
                || stream_port(values, schema.outputs()[0].value_type()).is_none()
                || schema.output_dependencies().len() != 1
                || !schema.output_dependencies()[0].inputs().is_empty()
                || !schema.rate_transitions().is_empty()
            {
                return Err(invalid("external Stream source shape"));
            }
        }
        GraphSimulationNodeKind::LatestRateTransition { input, output } => {
            if schema.inputs().len() != 1
                || schema.outputs().len() != 1
                || schema.inputs()[0].id() != input
                || schema.outputs()[0].id() != output
                || schema.rate_transitions()
                    != [NodeRateTransitionContract::new(
                        input,
                        output,
                        RateTransitionKind::LatestAtOrBeforeSourceFirst,
                    )]
            {
                return Err(invalid("latest rate-transition shape"));
            }
        }
        GraphSimulationNodeKind::StreamSink { input } => {
            if schema.inputs().len() != 1
                || schema.inputs()[0].id() != input
                || stream_port(values, schema.inputs()[0].value_type()).is_none()
                || !required_stream_queue(schema, input)
                || !schema.outputs().is_empty()
                || !schema.output_dependencies().is_empty()
                || !schema.rate_transitions().is_empty()
            {
                return Err(invalid("Stream sink shape"));
            }
        }
    }
    Ok(())
}

fn required_stream_queue(schema: &NodeSchema, input: GraphPortId) -> bool {
    schema.input_channels().iter().any(|channel| {
        channel.port() == input
            && channel.requirement() == InputConnectionRequirement::Required
            && matches!(channel.kind(), NodeInputChannelKind::StreamQueue { .. })
    })
}

fn compare_kind(left: &NodeKind, right: &NodeKind) -> Ordering {
    left.name()
        .cmp(right.name())
        .then_with(|| left.version().cmp(&right.version()))
}

fn simulation_registry_digest(
    semantic: &GraphNodeRegistry,
    implementations: &[GraphSimulationImplementation],
) -> Result<Digest, GraphSimulationError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ALSI");
    put_u16(&mut bytes, 1);
    put_u16(&mut bytes, 0);
    encode_analysis_limits(&mut bytes, semantic.limits())?;
    put_count(&mut bytes, semantic.schemas().len())?;
    for schema in semantic.schemas() {
        put_kind(&mut bytes, schema.kind())?;
        bytes.push(schema.allowed_domains().bits());
        put_ports(&mut bytes, schema.inputs())?;
        put_count(&mut bytes, schema.input_channels().len())?;
        for channel in schema.input_channels() {
            put_u32(&mut bytes, channel.port().get());
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
                    put_u32(&mut bytes, capacity);
                    bytes.push(full_policy_tag(full_policy));
                }
                NodeInputChannelKind::StreamQueue {
                    capacity,
                    full_policy,
                } => {
                    bytes.push(2);
                    put_u32(&mut bytes, capacity);
                    bytes.push(full_policy_tag(full_policy));
                }
            }
        }
        put_ports(&mut bytes, schema.outputs())?;
        put_count(&mut bytes, schema.parameters().len())?;
        for parameter in schema.parameters() {
            put_u32(&mut bytes, parameter.id());
            put_text(&mut bytes, parameter.name())?;
            put_u32(&mut bytes, parameter.value_type().get());
        }
        put_count(&mut bytes, schema.output_dependencies().len())?;
        for dependency in schema.output_dependencies() {
            put_u32(&mut bytes, dependency.output().get());
            put_count(&mut bytes, dependency.inputs().len())?;
            for input in dependency.inputs() {
                put_u32(&mut bytes, input.get());
            }
        }
        put_count(&mut bytes, schema.rate_transitions().len())?;
        for transition in schema.rate_transitions() {
            put_u32(&mut bytes, transition.input().get());
            put_u32(&mut bytes, transition.output().get());
            bytes.push(match transition.kind() {
                RateTransitionKind::LatestAtOrBeforeSourceFirst => 0,
            });
        }
        match schema.state() {
            None => bytes.push(0),
            Some(state) => {
                bytes.push(1);
                put_u32(&mut bytes, state.clock().get());
                put_u32(&mut bytes, state.value_type().get());
                put_u32(&mut bytes, state.initial_parameter());
                put_u32(&mut bytes, state.next_input().get());
                put_u32(&mut bytes, state.current_output().get());
                put_u32(&mut bytes, state.declared_storage_bytes());
            }
        }
    }
    put_count(&mut bytes, implementations.len())?;
    for implementation in implementations {
        put_kind(&mut bytes, &implementation.kind)?;
        match implementation.behavior {
            GraphSimulationNodeKind::ExternalStreamSource { output } => {
                bytes.push(0);
                put_u32(&mut bytes, output.get());
            }
            GraphSimulationNodeKind::LatestRateTransition { input, output } => {
                bytes.push(1);
                put_u32(&mut bytes, input.get());
                put_u32(&mut bytes, output.get());
            }
            GraphSimulationNodeKind::StreamSink { input } => {
                bytes.push(2);
                put_u32(&mut bytes, input.get());
            }
        }
    }
    Ok(sha256(&bytes).digest)
}

fn encode_analysis_limits(
    bytes: &mut Vec<u8>,
    limits: GraphAnalysisLimits,
) -> Result<(), GraphSimulationError> {
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
            u64::try_from(value).map_err(|_| GraphSimulationError::RegistryEncoding)?,
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

fn put_ports(
    bytes: &mut Vec<u8>,
    ports: &[super::PortDefinition],
) -> Result<(), GraphSimulationError> {
    put_count(bytes, ports.len())?;
    for port in ports {
        put_u32(bytes, port.id().get());
        put_text(bytes, port.name())?;
        put_u32(bytes, port.value_type().get());
    }
    Ok(())
}

fn put_kind(bytes: &mut Vec<u8>, kind: &NodeKind) -> Result<(), GraphSimulationError> {
    put_text(bytes, kind.name())?;
    put_u16(bytes, kind.version());
    Ok(())
}

fn put_text(bytes: &mut Vec<u8>, value: &str) -> Result<(), GraphSimulationError> {
    put_count(bytes, value.len())?;
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_count(bytes: &mut Vec<u8>, value: usize) -> Result<(), GraphSimulationError> {
    put_u32(
        bytes,
        u32::try_from(value).map_err(|_| GraphSimulationError::RegistryEncoding)?,
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        ClockDefinition, ClockKind, ExecutionDomainSet, GraphLimits, GraphNodeId, GraphPortId,
        GraphSchema, GraphTraceError, GraphValue, GraphWireId, NodeDefinition,
        NodeInputChannelContract, NodeOutputDependency, PortDefinition, TypeDefinition,
        WireDefinition, encode_graph_trace, replay_graph_trace,
    };

    const BOOL: GraphTypeId = GraphTypeId::new(1);
    const SOURCE_STREAM: GraphTypeId = GraphTypeId::new(2);
    const TARGET_STREAM: GraphTypeId = GraphTypeId::new(3);
    const ROOT: GraphClockId = GraphClockId::new(1);
    const SOURCE_CLOCK: GraphClockId = GraphClockId::new(2);
    const TARGET_CLOCK: GraphClockId = GraphClockId::new(3);

    fn port(id: u32, name: &str, value_type: GraphTypeId) -> PortDefinition {
        PortDefinition::new(GraphPortId::new(id), name, value_type)
    }

    fn endpoint(node: u32, port: u32) -> WireEndpoint {
        WireEndpoint {
            node: GraphNodeId::new(node),
            port: GraphPortId::new(port),
        }
    }

    fn fixture() -> (GraphDocument, GraphSimulationRegistry) {
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
        let clocks = vec![
            ClockDefinition::new(
                ROOT,
                "host.root",
                ClockKind::HostMonotonic {
                    ticks_per_second: 1_200,
                },
            ),
            ClockDefinition::new(
                SOURCE_CLOCK,
                "host.source",
                ClockKind::Derived {
                    source: ROOT,
                    numerator: 5,
                    denominator: 6,
                },
            ),
            ClockDefinition::new(
                TARGET_CLOCK,
                "host.target",
                ClockKind::Derived {
                    source: ROOT,
                    numerator: 1,
                    denominator: 2,
                },
            ),
        ];
        let source_node = NodeDefinition::new(
            GraphNodeId::new(1),
            NodeKind::new("sim.source", 1),
            "source",
            ExecutionDomain::HostExact,
            Vec::new(),
            vec![port(1, "samples", SOURCE_STREAM)],
            Vec::new(),
        );
        let transition_node = NodeDefinition::new(
            GraphNodeId::new(2),
            NodeKind::new("sim.rate", 1),
            "rate",
            ExecutionDomain::HostExact,
            vec![port(1, "source", SOURCE_STREAM)],
            vec![port(2, "target", TARGET_STREAM)],
            Vec::new(),
        );
        let sink_node = NodeDefinition::new(
            GraphNodeId::new(3),
            NodeKind::new("sim.sink", 1),
            "sink",
            ExecutionDomain::HostExact,
            vec![port(1, "samples", TARGET_STREAM)],
            Vec::new(),
            Vec::new(),
        );
        let document = GraphDocument::try_new(
            7,
            schema,
            clocks,
            vec![sink_node, transition_node, source_node],
            vec![
                WireDefinition::new(GraphWireId::new(2), endpoint(2, 2), endpoint(3, 1)),
                WireDefinition::new(GraphWireId::new(1), endpoint(1, 1), endpoint(2, 1)),
            ],
        )
        .unwrap();
        let dependency = |output, inputs: &[u32]| {
            NodeOutputDependency::new(
                GraphPortId::new(output),
                inputs.iter().copied().map(GraphPortId::new).collect(),
            )
        };
        let source_schema = NodeSchema::new(
            NodeKind::new("sim.source", 1),
            ExecutionDomainSet::HOST_EXACT,
            Vec::new(),
            Vec::new(),
            vec![port(1, "samples", SOURCE_STREAM)],
            Vec::new(),
            vec![dependency(1, &[])],
            Vec::new(),
            None,
        );
        let stream_channel = |port| {
            NodeInputChannelContract::new(
                GraphPortId::new(port),
                InputConnectionRequirement::Required,
                NodeInputChannelKind::StreamQueue {
                    capacity: 2,
                    full_policy: ChannelFullPolicy::Fault,
                },
            )
        };
        let transition_schema = NodeSchema::new(
            NodeKind::new("sim.rate", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "source", SOURCE_STREAM)],
            vec![stream_channel(1)],
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
            NodeKind::new("sim.sink", 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![port(1, "samples", TARGET_STREAM)],
            vec![stream_channel(1)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        let semantic = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &document,
            vec![sink_schema, transition_schema, source_schema],
        )
        .unwrap();
        let simulation = GraphSimulationRegistry::try_new(
            semantic,
            vec![
                GraphSimulationImplementation::new(
                    NodeKind::new("sim.sink", 1),
                    GraphSimulationNodeKind::StreamSink {
                        input: GraphPortId::new(1),
                    },
                ),
                GraphSimulationImplementation::new(
                    NodeKind::new("sim.source", 1),
                    GraphSimulationNodeKind::ExternalStreamSource {
                        output: GraphPortId::new(1),
                    },
                ),
                GraphSimulationImplementation::new(
                    NodeKind::new("sim.rate", 1),
                    GraphSimulationNodeKind::LatestRateTransition {
                        input: GraphPortId::new(1),
                        output: GraphPortId::new(2),
                    },
                ),
            ],
        )
        .unwrap();
        (document, simulation)
    }

    fn samples(document: &GraphDocument) -> Vec<ExternalStreamSample> {
        (0_u64..=8)
            .map(|tick| {
                ExternalStreamSample::new(
                    endpoint(1, 1),
                    tick,
                    100 + tick,
                    TypedGraphValue::try_new(
                        document.schema(),
                        BOOL,
                        GraphValue::Boolean(tick % 2 == 1),
                    )
                    .unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn exact_multirate_simulation_is_order_independent_and_source_first() {
        let (document, registry) = fixture();
        let input = samples(&document);
        let first = simulate_graph(
            &document,
            &registry,
            GraphSimulationHorizon::new(ROOT, 10),
            &input,
            GraphSimulationLimits::interactive(),
        )
        .unwrap();
        let mut reversed = input;
        reversed.reverse();
        let second = simulate_graph(
            &document,
            &registry,
            GraphSimulationHorizon::new(ROOT, 10),
            &reversed,
            GraphSimulationLimits::interactive(),
        )
        .unwrap();
        assert_eq!(first, second);
        assert_ne!(first.graph_digest(), Digest::ZERO);
        assert_ne!(first.registry_digest(), Digest::ZERO);
        assert_eq!(first.entries().len(), 15);
        let output: Vec<_> = first
            .entries()
            .iter()
            .filter(|entry| entry.endpoint() == endpoint(2, 2))
            .collect();
        assert_eq!(
            output
                .iter()
                .map(|entry| entry.clock_tick())
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5]
        );
        assert_eq!(
            output
                .iter()
                .map(|entry| entry.value().value())
                .collect::<Vec<_>>(),
            vec![
                &GraphValue::Boolean(false),
                &GraphValue::Boolean(true),
                &GraphValue::Boolean(true),
                &GraphValue::Boolean(true),
                &GraphValue::Boolean(false),
                &GraphValue::Boolean(false),
            ]
        );
    }

    #[test]
    fn simulation_registry_identity_is_canonical_across_binding_order() {
        let (_, registry) = fixture();
        let mut implementations = registry.implementations().to_vec();
        implementations.reverse();
        let rebuilt =
            GraphSimulationRegistry::try_new(registry.semantic_registry().clone(), implementations)
                .unwrap();
        assert_eq!(rebuilt, registry);
        assert_eq!(rebuilt.digest(), registry.digest());
    }

    #[test]
    fn simulation_rejects_missing_initial_late_and_nonmonotonic_external_samples() {
        let (document, registry) = fixture();
        assert_eq!(
            simulate_graph(
                &document,
                &registry,
                GraphSimulationHorizon::new(SOURCE_CLOCK, 10),
                &samples(&document),
                GraphSimulationLimits::interactive(),
            ),
            Err(GraphSimulationError::InvalidHorizonRoot(SOURCE_CLOCK))
        );

        let mut input = samples(&document);
        input.remove(0);
        assert_eq!(
            simulate_graph(
                &document,
                &registry,
                GraphSimulationHorizon::new(ROOT, 10),
                &input,
                GraphSimulationLimits::interactive(),
            ),
            Err(GraphSimulationError::MissingInitialSample(endpoint(2, 1)))
        );

        let mut late = samples(&document);
        late.push(ExternalStreamSample::new(
            endpoint(1, 1),
            9,
            109,
            TypedGraphValue::try_new(document.schema(), BOOL, GraphValue::Boolean(true)).unwrap(),
        ));
        assert_eq!(
            simulate_graph(
                &document,
                &registry,
                GraphSimulationHorizon::new(ROOT, 10),
                &late,
                GraphSimulationLimits::interactive(),
            ),
            Err(GraphSimulationError::ExternalSampleAfterHorizon(endpoint(
                1, 1
            )))
        );

        let mut unordered = samples(&document);
        unordered[4] = ExternalStreamSample::new(
            endpoint(1, 1),
            4,
            102,
            TypedGraphValue::try_new(document.schema(), BOOL, GraphValue::Boolean(false)).unwrap(),
        );
        assert_eq!(
            simulate_graph(
                &document,
                &registry,
                GraphSimulationHorizon::new(ROOT, 10),
                &unordered,
                GraphSimulationLimits::interactive(),
            ),
            Err(GraphSimulationError::ExternalSampleOrder(endpoint(1, 1)))
        );
    }

    #[test]
    fn canonical_trace_replays_by_independent_simulation_and_rejects_tamper() {
        let (document, registry) = fixture();
        let limits = GraphSimulationLimits::interactive();
        let simulation = simulate_graph(
            &document,
            &registry,
            GraphSimulationHorizon::new(ROOT, 10),
            &samples(&document),
            limits,
        )
        .unwrap();
        let trace = encode_graph_trace(&document, &simulation, limits).unwrap();
        assert_eq!(trace.bytes().len(), 658);
        assert_eq!(
            trace.digest(),
            Digest([
                0x4e, 0x0d, 0xba, 0xec, 0x54, 0x95, 0xe9, 0x6a, 0x1f, 0x47, 0x2c, 0x21, 0x8a, 0x83,
                0xe8, 0xf4, 0x41, 0x11, 0x8c, 0x8a, 0x68, 0xc4, 0xbd, 0x94, 0xb7, 0x75, 0x16, 0xb2,
                0xd6, 0x1b, 0x77, 0x3a,
            ])
        );
        assert_eq!(trace.digest(), sha256(trace.bytes()).digest);
        let replay = replay_graph_trace(trace.bytes(), &document, &registry, limits).unwrap();
        assert_eq!(replay.simulation(), &simulation);
        assert_eq!(replay.encoding(), &trace);

        for length in 0..trace.bytes().len() {
            assert!(
                replay_graph_trace(&trace.bytes()[..length], &document, &registry, limits).is_err(),
                "strict trace prefix {length} unexpectedly replayed"
            );
        }

        let mut trailing = trace.bytes().to_vec();
        trailing.push(0);
        assert_eq!(
            replay_graph_trace(&trailing, &document, &registry, limits),
            Err(GraphTraceError::ReplayDiverged)
        );

        const REGISTRY_DIGEST_OFFSET: usize = 4 + 2 + 2 + 32;
        let mut wrong_registry = trace.bytes().to_vec();
        wrong_registry[REGISTRY_DIGEST_OFFSET] ^= 1;
        assert_eq!(
            replay_graph_trace(&wrong_registry, &document, &registry, limits),
            Err(GraphTraceError::RegistryDigestMismatch)
        );

        // Header + first entry fields + its four-byte graph type identity.
        const FIRST_ENTRY_BOOLEAN_VALUE_OFFSET: usize = 88 + 33 + 4;
        let mut changed_value = trace.bytes().to_vec();
        changed_value[FIRST_ENTRY_BOOLEAN_VALUE_OFFSET] = 1;
        assert_eq!(
            replay_graph_trace(&changed_value, &document, &registry, limits),
            Err(GraphTraceError::ReplayDiverged)
        );

        let mut narrow = limits;
        narrow.maximum_trace_bytes = trace.bytes().len() - 1;
        assert_eq!(
            replay_graph_trace(trace.bytes(), &document, &registry, narrow),
            Err(GraphTraceError::LimitExceeded("byte length"))
        );
    }
}
