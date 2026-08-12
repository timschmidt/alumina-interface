//! Representative visible exact PID/interlock graph used by the application shell.
//!
//! This is a deterministic HostExact example, not a deployable controller or
//! firmware authority. Keeping construction here lets native, WASM, tests, and
//! future graph-editor import share exactly one saved graph and trace identity.

use core::fmt;

use super::CanonicalGraphTrace;
use super::{
    BaseDimensions, ChannelFullPolicy, ClockDefinition, ClockKind, ExecutionDomain,
    ExecutionDomainSet, ExternalStreamSample, GraphAnalysisLimits, GraphClockId, GraphDocument,
    GraphDocumentError, GraphLimits, GraphNodeId, GraphNodeRegistry, GraphPortId, GraphSchema,
    GraphSchemaError, GraphSimulation, GraphSimulationError, GraphSimulationHorizon,
    GraphSimulationImplementation, GraphSimulationLimits, GraphSimulationNodeKind,
    GraphSimulationRegistry, GraphTraceError, GraphTypeId, GraphValue, GraphWireId,
    InputConnectionRequirement, NodeDefinition, NodeInputChannelContract, NodeInputChannelKind,
    NodeKind, NodeOutputDependency, NodeParameter, NodeParameterContract,
    NodeRateTransitionContract, NodeRegistryError, NodeSchema, NodeStateContract, PortDefinition,
    RateTransitionKind, TypeDefinition, TypeKind, TypedGraphValue, UnitDefinition, UnitId,
    WireDefinition, WireEndpoint, encode_graph_trace, simulate_graph,
};
use hyperreal::{Problem, Rational};

/// One named trace exposed by the representative exact controller.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RepresentativeControlSignal {
    /// Setpoint minus measurement before controller terms.
    Error,
    /// Integral accumulator value exposed before the current update.
    IntegralPrior,
    /// Controller output after the exact inclusive clamp.
    ClampedController,
    /// Final output after the fail-safe permit gate.
    PermittedOutput,
}

impl RepresentativeControlSignal {
    /// Return the exact graph output endpoint carrying this signal.
    pub const fn endpoint(self) -> WireEndpoint {
        match self {
            Self::Error => endpoint(7, 3),
            Self::IntegralPrior => endpoint(9, 2),
            Self::ClampedController => endpoint(17, 2),
            Self::PermittedOutput => endpoint(18, 3),
        }
    }

    /// Return the stable user-facing signal label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::IntegralPrior => "integral prior state",
            Self::ClampedController => "clamped controller",
            Self::PermittedOutput => "permit-gated output",
        }
    }
}

/// The exact graph, fixed semantic/implementation authority, result, and trace.
#[derive(Clone, Debug, PartialEq)]
pub struct RepresentativeExactControlGraph {
    document: GraphDocument,
    registry: GraphSimulationRegistry,
    simulation: GraphSimulation,
    trace: CanonicalGraphTrace,
}

impl RepresentativeExactControlGraph {
    /// Borrow the canonical saved graph document.
    pub const fn document(&self) -> &GraphDocument {
        &self.document
    }

    /// Borrow the exact audited HostExact implementation authority.
    pub const fn registry(&self) -> &GraphSimulationRegistry {
        &self.registry
    }

    /// Borrow the deterministic exact simulation.
    pub const fn simulation(&self) -> &GraphSimulation {
        &self.simulation
    }

    /// Borrow the independently replayable canonical trace.
    pub const fn trace(&self) -> &CanonicalGraphTrace {
        &self.trace
    }
}

/// Failure while constructing or simulating the static representative graph.
#[derive(Clone, Debug, PartialEq)]
pub enum RepresentativeExactControlError {
    /// A static exact rational constant was rejected.
    Exact(Problem),
    /// Exact unit/type/value schema construction failed.
    Schema(GraphSchemaError),
    /// Structural graph construction failed.
    Document(GraphDocumentError),
    /// Audited node semantics rejected the fixture.
    Registry(NodeRegistryError),
    /// Fixed implementation admission or exact simulation failed.
    Simulation(GraphSimulationError),
    /// Canonical trace encoding failed.
    Trace(GraphTraceError),
}

impl fmt::Display for RepresentativeExactControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(error) => {
                write!(formatter, "representative exact constant failed: {error}")
            }
            Self::Schema(error) => {
                write!(formatter, "representative control schema failed: {error}")
            }
            Self::Document(error) => {
                write!(formatter, "representative control document failed: {error}")
            }
            Self::Registry(error) => {
                write!(formatter, "representative control registry failed: {error}")
            }
            Self::Simulation(error) => {
                write!(
                    formatter,
                    "representative control simulation failed: {error}"
                )
            }
            Self::Trace(error) => write!(formatter, "representative control trace failed: {error}"),
        }
    }
}

impl std::error::Error for RepresentativeExactControlError {}

impl From<Problem> for RepresentativeExactControlError {
    fn from(value: Problem) -> Self {
        Self::Exact(value)
    }
}

impl From<GraphSchemaError> for RepresentativeExactControlError {
    fn from(value: GraphSchemaError) -> Self {
        Self::Schema(value)
    }
}

impl From<GraphDocumentError> for RepresentativeExactControlError {
    fn from(value: GraphDocumentError) -> Self {
        Self::Document(value)
    }
}

impl From<NodeRegistryError> for RepresentativeExactControlError {
    fn from(value: NodeRegistryError) -> Self {
        Self::Registry(value)
    }
}

impl From<GraphSimulationError> for RepresentativeExactControlError {
    fn from(value: GraphSimulationError) -> Self {
        Self::Simulation(value)
    }
}

impl From<GraphTraceError> for RepresentativeExactControlError {
    fn from(value: GraphTraceError) -> Self {
        Self::Trace(value)
    }
}

const PERCENT: UnitId = UnitId::new(1);
const MILLIMETRE: UnitId = UnitId::new(2);

const BOOL: GraphTypeId = GraphTypeId::new(1);
const VALUE: GraphTypeId = GraphTypeId::new(2);
const FACTOR: GraphTypeId = GraphTypeId::new(3);
const SOURCE_VALUE_STREAM: GraphTypeId = GraphTypeId::new(4);
const CONTROL_VALUE_STREAM: GraphTypeId = GraphTypeId::new(5);
const SOURCE_BOOL_STREAM: GraphTypeId = GraphTypeId::new(6);
const CONTROL_BOOL_STREAM: GraphTypeId = GraphTypeId::new(7);

const ROOT: GraphClockId = GraphClockId::new(1);
const SOURCE_CLOCK: GraphClockId = GraphClockId::new(2);
const CONTROL_CLOCK: GraphClockId = GraphClockId::new(3);

const fn endpoint(node: u32, port: u32) -> WireEndpoint {
    WireEndpoint {
        node: GraphNodeId::new(node),
        port: GraphPortId::new(port),
    }
}

fn port(id: u32, name: &str, value_type: GraphTypeId) -> PortDefinition {
    PortDefinition::new(GraphPortId::new(id), name, value_type)
}

fn exact(
    schema: &GraphSchema,
    value_type: GraphTypeId,
    value: Rational,
) -> Result<TypedGraphValue, GraphSchemaError> {
    TypedGraphValue::try_new(schema, value_type, GraphValue::ExactRational(value))
}

fn boolean(schema: &GraphSchema, value: bool) -> Result<TypedGraphValue, GraphSchemaError> {
    TypedGraphValue::try_new(schema, BOOL, GraphValue::Boolean(value))
}

fn parameter(
    schema: &GraphSchema,
    id: u32,
    name: &str,
    value_type: GraphTypeId,
    value: Rational,
) -> Result<NodeParameter, GraphSchemaError> {
    Ok(NodeParameter::new(
        id,
        name,
        exact(schema, value_type, value)?,
    ))
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
        representative_node_label(id),
        ExecutionDomain::HostExact,
        inputs,
        outputs,
        parameters,
    )
}

const fn representative_node_label(id: u32) -> &'static str {
    match id {
        1 => "Setpoint source",
        2 => "Measurement source",
        3 => "Safety permit source",
        4 => "Setpoint 50 Hz to 10 Hz",
        5 => "Measurement 50 Hz to 10 Hz",
        6 => "Permit 50 Hz to 10 Hz",
        7 => "Control error",
        8 => "Proportional term",
        9 => "Integral prior state",
        10 => "Integral accumulator",
        11 => "Integral term",
        12 => "Previous error",
        13 => "Error delta",
        14 => "Derivative term",
        15 => "P + I",
        16 => "PID sum",
        17 => "Output clamp",
        18 => "Safety permit gate",
        19 => "Controller output",
        _ => "Representative control node",
    }
}

fn queue(port: u32, capacity: u32) -> NodeInputChannelContract {
    NodeInputChannelContract::new(
        GraphPortId::new(port),
        InputConnectionRequirement::Required,
        NodeInputChannelKind::StreamQueue {
            capacity,
            full_policy: ChannelFullPolicy::Fault,
        },
    )
}

fn dependency(output: u32, inputs: &[u32]) -> NodeOutputDependency {
    NodeOutputDependency::new(
        GraphPortId::new(output),
        inputs.iter().copied().map(GraphPortId::new).collect(),
    )
}

fn parameter_contract(id: u32, name: &str, value_type: GraphTypeId) -> NodeParameterContract {
    NodeParameterContract::new(id, name, value_type)
}

fn document_and_registry()
-> Result<(GraphDocument, GraphSimulationRegistry), RepresentativeExactControlError> {
    let mut limits = GraphLimits::interactive();
    limits.maximum_rational_digits = 32;
    let schema = GraphSchema::try_new(
        limits,
        vec![
            UnitDefinition::new(
                PERCENT,
                "%",
                BaseDimensions::DIMENSIONLESS,
                Rational::fraction(1, 100)?,
            ),
            UnitDefinition::new(
                MILLIMETRE,
                "mm",
                BaseDimensions::LENGTH,
                Rational::fraction(1, 1_000)?,
            ),
        ],
        vec![
            TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
            TypeDefinition::new(
                VALUE,
                "exact.mm",
                TypeKind::ExactRational { unit: MILLIMETRE },
            ),
            TypeDefinition::new(
                FACTOR,
                "exact.percent",
                TypeKind::ExactRational { unit: PERCENT },
            ),
            TypeDefinition::new(
                SOURCE_VALUE_STREAM,
                "stream.source.value",
                TypeKind::Stream {
                    sample: VALUE,
                    clock: SOURCE_CLOCK,
                    capacity: 32,
                },
            ),
            TypeDefinition::new(
                CONTROL_VALUE_STREAM,
                "stream.control.value",
                TypeKind::Stream {
                    sample: VALUE,
                    clock: CONTROL_CLOCK,
                    capacity: 16,
                },
            ),
            TypeDefinition::new(
                SOURCE_BOOL_STREAM,
                "stream.source.bool",
                TypeKind::Stream {
                    sample: BOOL,
                    clock: SOURCE_CLOCK,
                    capacity: 32,
                },
            ),
            TypeDefinition::new(
                CONTROL_BOOL_STREAM,
                "stream.control.bool",
                TypeKind::Stream {
                    sample: BOOL,
                    clock: CONTROL_CLOCK,
                    capacity: 16,
                },
            ),
        ],
    )?;
    let clocks = vec![
        ClockDefinition::new(
            ROOT,
            "host.root",
            ClockKind::HostMonotonic {
                ticks_per_second: 100,
            },
        ),
        ClockDefinition::new(
            SOURCE_CLOCK,
            "host.source",
            ClockKind::Derived {
                source: ROOT,
                numerator: 1,
                denominator: 2,
            },
        ),
        ClockDefinition::new(
            CONTROL_CLOCK,
            "host.control",
            ClockKind::Derived {
                source: ROOT,
                numerator: 1,
                denominator: 10,
            },
        ),
    ];

    let nodes = vec![
        node(
            1,
            "control.source.value",
            Vec::new(),
            vec![port(1, "samples", SOURCE_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            2,
            "control.source.value",
            Vec::new(),
            vec![port(1, "samples", SOURCE_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            3,
            "control.source.bool",
            Vec::new(),
            vec![port(1, "samples", SOURCE_BOOL_STREAM)],
            Vec::new(),
        ),
        node(
            4,
            "control.rate.value",
            vec![port(1, "source", SOURCE_VALUE_STREAM)],
            vec![port(2, "target", CONTROL_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            5,
            "control.rate.value",
            vec![port(1, "source", SOURCE_VALUE_STREAM)],
            vec![port(2, "target", CONTROL_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            6,
            "control.rate.bool",
            vec![port(1, "source", SOURCE_BOOL_STREAM)],
            vec![port(2, "target", CONTROL_BOOL_STREAM)],
            Vec::new(),
        ),
        node(
            7,
            "control.exact.subtract",
            vec![
                port(1, "left", CONTROL_VALUE_STREAM),
                port(2, "right", CONTROL_VALUE_STREAM),
            ],
            vec![port(3, "difference", CONTROL_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            8,
            "control.exact.scale",
            vec![port(1, "value", CONTROL_VALUE_STREAM)],
            vec![port(2, "scaled", CONTROL_VALUE_STREAM)],
            vec![parameter(
                &schema,
                1,
                "factor",
                FACTOR,
                Rational::from(200),
            )?],
        ),
        node(
            9,
            "control.exact.delay",
            vec![port(1, "next", CONTROL_VALUE_STREAM)],
            vec![port(2, "current", CONTROL_VALUE_STREAM)],
            vec![parameter(&schema, 1, "initial", VALUE, Rational::zero())?],
        ),
        node(
            10,
            "control.exact.add",
            vec![
                port(1, "left", CONTROL_VALUE_STREAM),
                port(2, "right", CONTROL_VALUE_STREAM),
            ],
            vec![port(3, "sum", CONTROL_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            11,
            "control.exact.scale",
            vec![port(1, "value", CONTROL_VALUE_STREAM)],
            vec![port(2, "scaled", CONTROL_VALUE_STREAM)],
            vec![parameter(&schema, 1, "factor", FACTOR, Rational::from(50))?],
        ),
        node(
            12,
            "control.exact.delay",
            vec![port(1, "next", CONTROL_VALUE_STREAM)],
            vec![port(2, "current", CONTROL_VALUE_STREAM)],
            vec![parameter(&schema, 1, "initial", VALUE, Rational::zero())?],
        ),
        node(
            13,
            "control.exact.subtract",
            vec![
                port(1, "left", CONTROL_VALUE_STREAM),
                port(2, "right", CONTROL_VALUE_STREAM),
            ],
            vec![port(3, "difference", CONTROL_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            14,
            "control.exact.scale",
            vec![port(1, "value", CONTROL_VALUE_STREAM)],
            vec![port(2, "scaled", CONTROL_VALUE_STREAM)],
            vec![parameter(
                &schema,
                1,
                "factor",
                FACTOR,
                Rational::from(100),
            )?],
        ),
        node(
            15,
            "control.exact.add",
            vec![
                port(1, "left", CONTROL_VALUE_STREAM),
                port(2, "right", CONTROL_VALUE_STREAM),
            ],
            vec![port(3, "sum", CONTROL_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            16,
            "control.exact.add",
            vec![
                port(1, "left", CONTROL_VALUE_STREAM),
                port(2, "right", CONTROL_VALUE_STREAM),
            ],
            vec![port(3, "sum", CONTROL_VALUE_STREAM)],
            Vec::new(),
        ),
        node(
            17,
            "control.exact.clamp",
            vec![port(1, "value", CONTROL_VALUE_STREAM)],
            vec![port(2, "clamped", CONTROL_VALUE_STREAM)],
            vec![
                parameter(&schema, 1, "minimum", VALUE, Rational::from(-5))?,
                parameter(&schema, 2, "maximum", VALUE, Rational::from(5))?,
            ],
        ),
        node(
            18,
            "control.exact.permit",
            vec![
                port(1, "value", CONTROL_VALUE_STREAM),
                port(2, "permit", CONTROL_BOOL_STREAM),
            ],
            vec![port(3, "safe_value", CONTROL_VALUE_STREAM)],
            vec![parameter(&schema, 1, "safe", VALUE, Rational::zero())?],
        ),
        node(
            19,
            "control.sink",
            vec![port(1, "samples", CONTROL_VALUE_STREAM)],
            Vec::new(),
            Vec::new(),
        ),
    ];

    let raw_wires = [
        (1, endpoint(1, 1), endpoint(4, 1)),
        (2, endpoint(2, 1), endpoint(5, 1)),
        (3, endpoint(3, 1), endpoint(6, 1)),
        (4, endpoint(4, 2), endpoint(7, 1)),
        (5, endpoint(5, 2), endpoint(7, 2)),
        (6, endpoint(7, 3), endpoint(8, 1)),
        (7, endpoint(9, 2), endpoint(10, 1)),
        (8, endpoint(7, 3), endpoint(10, 2)),
        (9, endpoint(10, 3), endpoint(9, 1)),
        (10, endpoint(10, 3), endpoint(11, 1)),
        (11, endpoint(7, 3), endpoint(12, 1)),
        (12, endpoint(7, 3), endpoint(13, 1)),
        (13, endpoint(12, 2), endpoint(13, 2)),
        (14, endpoint(13, 3), endpoint(14, 1)),
        (15, endpoint(8, 2), endpoint(15, 1)),
        (16, endpoint(11, 2), endpoint(15, 2)),
        (17, endpoint(15, 3), endpoint(16, 1)),
        (18, endpoint(14, 2), endpoint(16, 2)),
        (19, endpoint(16, 3), endpoint(17, 1)),
        (20, endpoint(17, 2), endpoint(18, 1)),
        (21, endpoint(6, 2), endpoint(18, 2)),
        (22, endpoint(18, 3), endpoint(19, 1)),
    ];
    let wires = raw_wires
        .into_iter()
        .map(|(id, source, target)| WireDefinition::new(GraphWireId::new(id), source, target))
        .collect();
    let document = GraphDocument::try_new(1, schema, clocks, nodes, wires)?;

    let value_source = NodeSchema::new(
        NodeKind::new("control.source.value", 1),
        ExecutionDomainSet::HOST_EXACT,
        Vec::new(),
        Vec::new(),
        vec![port(1, "samples", SOURCE_VALUE_STREAM)],
        Vec::new(),
        vec![dependency(1, &[])],
        Vec::new(),
        None,
    );
    let bool_source = NodeSchema::new(
        NodeKind::new("control.source.bool", 1),
        ExecutionDomainSet::HOST_EXACT,
        Vec::new(),
        Vec::new(),
        vec![port(1, "samples", SOURCE_BOOL_STREAM)],
        Vec::new(),
        vec![dependency(1, &[])],
        Vec::new(),
        None,
    );
    let value_rate = NodeSchema::new(
        NodeKind::new("control.rate.value", 1),
        ExecutionDomainSet::HOST_EXACT,
        vec![port(1, "source", SOURCE_VALUE_STREAM)],
        vec![queue(1, 5)],
        vec![port(2, "target", CONTROL_VALUE_STREAM)],
        Vec::new(),
        vec![dependency(2, &[1])],
        vec![NodeRateTransitionContract::new(
            GraphPortId::new(1),
            GraphPortId::new(2),
            RateTransitionKind::LatestAtOrBeforeSourceFirst,
        )],
        None,
    );
    let bool_rate = NodeSchema::new(
        NodeKind::new("control.rate.bool", 1),
        ExecutionDomainSet::HOST_EXACT,
        vec![port(1, "source", SOURCE_BOOL_STREAM)],
        vec![queue(1, 5)],
        vec![port(2, "target", CONTROL_BOOL_STREAM)],
        Vec::new(),
        vec![dependency(2, &[1])],
        vec![NodeRateTransitionContract::new(
            GraphPortId::new(1),
            GraphPortId::new(2),
            RateTransitionKind::LatestAtOrBeforeSourceFirst,
        )],
        None,
    );
    let binary = |kind: &str, output_name: &str| {
        NodeSchema::new(
            NodeKind::new(kind, 1),
            ExecutionDomainSet::HOST_EXACT,
            vec![
                port(1, "left", CONTROL_VALUE_STREAM),
                port(2, "right", CONTROL_VALUE_STREAM),
            ],
            vec![queue(1, 1), queue(2, 1)],
            vec![port(3, output_name, CONTROL_VALUE_STREAM)],
            Vec::new(),
            vec![dependency(3, &[1, 2])],
            Vec::new(),
            None,
        )
    };
    let scale = NodeSchema::new(
        NodeKind::new("control.exact.scale", 1),
        ExecutionDomainSet::HOST_EXACT,
        vec![port(1, "value", CONTROL_VALUE_STREAM)],
        vec![queue(1, 1)],
        vec![port(2, "scaled", CONTROL_VALUE_STREAM)],
        vec![parameter_contract(1, "factor", FACTOR)],
        vec![dependency(2, &[1])],
        Vec::new(),
        None,
    );
    let delay = NodeSchema::new(
        NodeKind::new("control.exact.delay", 1),
        ExecutionDomainSet::HOST_EXACT,
        vec![port(1, "next", CONTROL_VALUE_STREAM)],
        vec![queue(1, 1)],
        vec![port(2, "current", CONTROL_VALUE_STREAM)],
        vec![parameter_contract(1, "initial", VALUE)],
        vec![dependency(2, &[])],
        Vec::new(),
        Some(NodeStateContract::new(
            CONTROL_CLOCK,
            VALUE,
            1,
            GraphPortId::new(1),
            GraphPortId::new(2),
            128,
        )),
    );
    let clamp = NodeSchema::new(
        NodeKind::new("control.exact.clamp", 1),
        ExecutionDomainSet::HOST_EXACT,
        vec![port(1, "value", CONTROL_VALUE_STREAM)],
        vec![queue(1, 1)],
        vec![port(2, "clamped", CONTROL_VALUE_STREAM)],
        vec![
            parameter_contract(1, "minimum", VALUE),
            parameter_contract(2, "maximum", VALUE),
        ],
        vec![dependency(2, &[1])],
        Vec::new(),
        None,
    );
    let permit = NodeSchema::new(
        NodeKind::new("control.exact.permit", 1),
        ExecutionDomainSet::HOST_EXACT,
        vec![
            port(1, "value", CONTROL_VALUE_STREAM),
            port(2, "permit", CONTROL_BOOL_STREAM),
        ],
        vec![queue(1, 1), queue(2, 1)],
        vec![port(3, "safe_value", CONTROL_VALUE_STREAM)],
        vec![parameter_contract(1, "safe", VALUE)],
        vec![dependency(3, &[1, 2])],
        Vec::new(),
        None,
    );
    let sink = NodeSchema::new(
        NodeKind::new("control.sink", 1),
        ExecutionDomainSet::HOST_EXACT,
        vec![port(1, "samples", CONTROL_VALUE_STREAM)],
        vec![queue(1, 1)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
    );
    let semantic = GraphNodeRegistry::try_new(
        GraphAnalysisLimits::interactive(),
        &document,
        vec![
            value_source,
            bool_source,
            value_rate,
            bool_rate,
            binary("control.exact.subtract", "difference"),
            binary("control.exact.add", "sum"),
            scale,
            delay,
            clamp,
            permit,
            sink,
        ],
    )?;
    let implementations = vec![
        GraphSimulationImplementation::new(
            NodeKind::new("control.source.value", 1),
            GraphSimulationNodeKind::ExternalStreamSource {
                output: GraphPortId::new(1),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.source.bool", 1),
            GraphSimulationNodeKind::ExternalStreamSource {
                output: GraphPortId::new(1),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.rate.value", 1),
            GraphSimulationNodeKind::LatestRateTransition {
                input: GraphPortId::new(1),
                output: GraphPortId::new(2),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.rate.bool", 1),
            GraphSimulationNodeKind::LatestRateTransition {
                input: GraphPortId::new(1),
                output: GraphPortId::new(2),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.exact.subtract", 1),
            GraphSimulationNodeKind::ExactSubtract {
                left: GraphPortId::new(1),
                right: GraphPortId::new(2),
                output: GraphPortId::new(3),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.exact.add", 1),
            GraphSimulationNodeKind::ExactAdd {
                left: GraphPortId::new(1),
                right: GraphPortId::new(2),
                output: GraphPortId::new(3),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.exact.scale", 1),
            GraphSimulationNodeKind::ExactScale {
                input: GraphPortId::new(1),
                factor_parameter: 1,
                output: GraphPortId::new(2),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.exact.delay", 1),
            GraphSimulationNodeKind::UnitDelay {
                input: GraphPortId::new(1),
                initial_parameter: 1,
                output: GraphPortId::new(2),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.exact.clamp", 1),
            GraphSimulationNodeKind::ExactClamp {
                input: GraphPortId::new(1),
                minimum_parameter: 1,
                maximum_parameter: 2,
                output: GraphPortId::new(2),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.exact.permit", 1),
            GraphSimulationNodeKind::ExactPermitGate {
                value: GraphPortId::new(1),
                permit: GraphPortId::new(2),
                safe_parameter: 1,
                output: GraphPortId::new(3),
            },
        ),
        GraphSimulationImplementation::new(
            NodeKind::new("control.sink", 1),
            GraphSimulationNodeKind::StreamSink {
                input: GraphPortId::new(1),
            },
        ),
    ];
    let registry = GraphSimulationRegistry::try_new(semantic, implementations)?;
    Ok((document, registry))
}

fn samples(document: &GraphDocument) -> Result<Vec<ExternalStreamSample>, GraphSchemaError> {
    let mut samples = Vec::new();
    for tick in 0_u64..=25 {
        let control_tick = tick / 5;
        let measurement = control_tick.min(3);
        samples.push(ExternalStreamSample::new(
            endpoint(1, 1),
            tick,
            100 + tick,
            exact(document.schema(), VALUE, Rational::from(3))?,
        ));
        samples.push(ExternalStreamSample::new(
            endpoint(2, 1),
            tick,
            200 + tick,
            exact(document.schema(), VALUE, Rational::from(measurement))?,
        ));
        samples.push(ExternalStreamSample::new(
            endpoint(3, 1),
            tick,
            300 + tick,
            boolean(document.schema(), control_tick < 3)?,
        ));
    }
    Ok(samples)
}

/// Construct and simulate the canonical multi-rate exact PID/interlock example.
///
/// # Errors
///
/// Returns the exact schema, document, semantic-registry, implementation,
/// simulation, or trace rejection if this source fixture regresses.
pub fn compile_representative_exact_control_graph()
-> Result<RepresentativeExactControlGraph, RepresentativeExactControlError> {
    let (document, registry) = document_and_registry()?;
    let limits = GraphSimulationLimits::interactive();
    let simulation = simulate_graph(
        &document,
        &registry,
        GraphSimulationHorizon::new(ROOT, 50),
        &samples(&document)?,
        limits,
    )?;
    let trace = encode_graph_trace(&document, &simulation, limits)?;
    Ok(RepresentativeExactControlGraph {
        document,
        registry,
        simulation,
        trace,
    })
}

#[cfg(test)]
fn rational_trace(simulation: &GraphSimulation, endpoint: WireEndpoint) -> Vec<Rational> {
    simulation
        .entries()
        .iter()
        .filter(|entry| entry.endpoint() == endpoint)
        .map(|entry| match entry.value().value() {
            GraphValue::ExactRational(value) => value.clone(),
            value => panic!("unexpected trace value {value:?}"),
        })
        .collect()
}

#[cfg(test)]
#[test]
fn multirate_exact_pid_and_interlock_are_visible_deterministic_and_replayable() {
    let fixture = compile_representative_exact_control_graph().unwrap();
    let document = fixture.document();
    let registry = fixture.registry();
    let simulation = fixture.simulation();
    let limits = GraphSimulationLimits::interactive();
    let input = samples(document).unwrap();
    let mut reversed = input;
    reversed.reverse();
    assert_eq!(
        simulate_graph(
            document,
            registry,
            GraphSimulationHorizon::new(ROOT, 50),
            &reversed,
            limits,
        )
        .unwrap(),
        *simulation
    );

    assert_eq!(
        rational_trace(simulation, endpoint(9, 2)),
        [0, 3, 5, 6, 6, 6].map(Rational::from)
    );
    assert_eq!(
        rational_trace(simulation, endpoint(17, 2)),
        [5, 5, 4, 2, 3, 3].map(Rational::from)
    );
    assert_eq!(
        rational_trace(simulation, endpoint(18, 3)),
        [5, 5, 4, 0, 0, 0].map(Rational::from)
    );

    let trace = fixture.trace();
    assert_eq!(
        simulation.graph_digest().0,
        [
            0xfb, 0x17, 0x3f, 0xb3, 0x0b, 0xc5, 0xe0, 0x42, 0x69, 0xca, 0xea, 0x43, 0x9d, 0xea,
            0x8f, 0xa4, 0x55, 0x05, 0x01, 0x42, 0xfa, 0xc3, 0xa4, 0xaf, 0xc7, 0x8f, 0x5f, 0xd1,
            0x6e, 0x7a, 0xc5, 0x9a,
        ]
    );
    assert_eq!(
        simulation.registry_digest().0,
        [
            0x6b, 0xb6, 0xf8, 0x14, 0x94, 0x1b, 0x63, 0x2a, 0xc5, 0xc9, 0x85, 0x8f, 0xbb, 0xfe,
            0x59, 0x9f, 0xe8, 0xfe, 0xbb, 0x3a, 0x04, 0xb4, 0xdc, 0xc7, 0xbf, 0x4f, 0xbc, 0x8a,
            0xc2, 0xf6, 0x15, 0x37,
        ]
    );
    assert_eq!(trace.bytes().len(), 7_836);
    assert_eq!(
        trace.digest().0,
        [
            0x4d, 0x9b, 0x63, 0x63, 0x3b, 0xe3, 0xaf, 0xc6, 0x58, 0xca, 0xc8, 0xd6, 0x47, 0x5d,
            0x6e, 0xde, 0x60, 0x25, 0x68, 0xab, 0x08, 0x4d, 0xe0, 0x05, 0xac, 0x5d, 0xd2, 0xdf,
            0xcb, 0x75, 0x42, 0xa3,
        ]
    );
    let replay = super::replay_graph_trace(trace.bytes(), document, registry, limits).unwrap();
    assert_eq!(replay.simulation(), simulation);
    assert_eq!(replay.encoding().digest(), trace.digest());
}
