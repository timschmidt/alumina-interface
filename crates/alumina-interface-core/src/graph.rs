//! Bounded exact value and type registries for the greenfield graphical-control IR.
//!
//! This module owns no renderer, network transport, device task, or executor.
//! It establishes the value and saved editor-document authority those layers share:
//! registered multiplicative units, exact rational values, bounded physical
//! intervals, canonical integer lattices, typed composites, runtime-only
//! events/streams, and identity-bearing resource/job handles.

use core::fmt;
use std::collections::BTreeSet;

use alumina_protocol::{DeviceId, Digest};
use hyperreal::Rational;

mod analysis;
mod capability_catalog;
mod component;
mod control_fixture;
mod deployment;
mod document;
mod hierarchy;
mod probe;
mod simulation;
mod storage;
mod trace;
mod wire;
mod workspace;

pub use analysis::{
    ChannelFullPolicy, CombinationalCycle, DependencyLink, ExecutionDomainSet,
    GRAPH_CHANNEL_ENVELOPE_BYTES, GraphAnalysis, GraphAnalysisError, GraphAnalysisLimits,
    GraphChannelAllocation, GraphClockRate, GraphDraftAnalysis, GraphNodeRegistry,
    GraphRateTransition, InputConnectionRequirement, NodeInputChannelContract,
    NodeInputChannelKind, NodeOutputDependency, NodeParameterContract, NodeRateTransitionContract,
    NodeRegistryError, NodeSchema, NodeStateAllocation, NodeStateContract, RateTransitionKind,
    analyze_graph, analyze_graph_draft,
};
pub use capability_catalog::{
    GraphCapabilityCatalogError, GraphCapabilityCatalogLimits, GraphCapabilityNodeCatalog,
    GraphCapabilityNodeEntry, derive_graph_capability_node_catalog, graph_resource_label,
};
pub use component::{
    CanonicalGraphComponentEncoding, GRAPH_COMPONENT_MAGIC, GRAPH_COMPONENT_VERSION,
    GraphComponentDocument, GraphComponentError, GraphComponentInput, GraphComponentInputId,
    GraphComponentLimits, GraphComponentOutput, GraphComponentOutputId, GraphComponentReplay,
    GraphFrontPanelBinding, GraphFrontPanelItem, GraphFrontPanelItemId, GraphFrontPanelRect,
    encode_graph_component, replay_graph_component,
};
pub use control_fixture::{
    RepresentativeControlSignal, RepresentativeExactControlError, RepresentativeExactControlGraph,
    compile_representative_exact_control_graph,
};
pub use deployment::{
    GraphDeploymentError, GraphDeploymentImplementation, GraphDeploymentLimits,
    GraphDeploymentNodeKind, GraphDeploymentRegistry, GraphDeploymentReport, GraphDeploymentTarget,
    lower_graph_deployment,
};
pub use document::{
    ClockDefinition, ClockKind, ExecutionDomain, GraphDocument, GraphDocumentError, GraphNodeId,
    GraphPortId, GraphWireId, NodeDefinition, NodeKind, NodeParameter, PortDefinition,
    WireDefinition, WireEndpoint,
};
pub use hierarchy::{
    CanonicalGraphHierarchyEncoding, GRAPH_COMPONENT_INSTANCE_KIND,
    GRAPH_COMPONENT_INSTANCE_VERSION, GRAPH_HIERARCHY_MAGIC, GRAPH_HIERARCHY_VERSION,
    GraphComponentInstance, GraphFlattenedInstance, GraphFlattenedNode, GraphHierarchyDependency,
    GraphHierarchyDocument, GraphHierarchyError, GraphHierarchyFlattening, GraphHierarchyLimits,
    GraphHierarchyReplay, encode_graph_hierarchy, flatten_graph_hierarchy,
    graph_component_instance_input_port, graph_component_instance_output_port,
    graph_component_instance_prototype, replay_graph_hierarchy,
};
pub use probe::{
    CanonicalGraphProbeEncoding, GRAPH_PROBE_MAGIC, GRAPH_PROBE_VERSION, GraphProbeCapture,
    GraphProbeDefinition, GraphProbeDocument, GraphProbeError, GraphProbeId, GraphProbeLimits,
    GraphProbeReplay, encode_graph_probes, replay_graph_probes,
};
pub use simulation::{
    ExternalStreamSample, GraphSimulation, GraphSimulationError, GraphSimulationHorizon,
    GraphSimulationImplementation, GraphSimulationLimits, GraphSimulationNodeKind,
    GraphSimulationRegistry, GraphTraceEntry, GraphTraceEntryKind, simulate_graph,
};
pub use storage::{GraphStorageError, GraphTypeStorageBound, GraphTypeStorageKind};
pub use trace::{
    CanonicalGraphTrace, GRAPH_TRACE_MAGIC, GRAPH_TRACE_VERSION, GraphTraceError, GraphTraceReplay,
    encode_graph_trace, replay_graph_trace,
};
pub use wire::{
    CanonicalGraphEncoding, GRAPH_DOCUMENT_MAGIC, GRAPH_DOCUMENT_VERSION, GraphReplay,
    GraphWireError, encode_graph_document, replay_graph_document,
};
pub use workspace::{
    CanonicalGraphWorkspaceEncoding, GRAPH_WORKSPACE_MAGIC, GRAPH_WORKSPACE_VERSION,
    GraphNodePlacement, GraphNodePrototype, GraphWorkspaceDocument, GraphWorkspaceError,
    GraphWorkspaceHistory, GraphWorkspaceHistoryError, GraphWorkspaceHistoryLimits,
    GraphWorkspaceLimits, GraphWorkspaceReplay, encode_graph_workspace, replay_graph_workspace,
};

/// Stable identifier for one registered physical unit.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct UnitId(u32);

impl UnitId {
    /// Construct an identifier. Zero is rejected when a schema is validated.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identifier.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable identifier for one registered graph value type.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphTypeId(u32);

impl GraphTypeId {
    /// Construct an identifier. Zero is rejected when a schema is validated.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identifier.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable identifier for one graph clock definition.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphClockId(u32);

impl GraphClockId {
    /// Construct an identifier. Zero is never a valid clock reference.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identifier.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable identifier for one field inside a record type.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct RecordFieldId(u32);

impl RecordFieldId {
    /// Construct an identifier. Zero is rejected in record schemas.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identifier.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable resource class emitted by a capability-derived graph palette.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ResourceClassId(u32);

impl ResourceClassId {
    /// Construct an identifier. Zero is rejected in resource-handle types.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identifier.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// SI base-dimension exponents in metre, kilogram, second, ampere, kelvin,
/// mole, and candela order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct BaseDimensions([i8; 7]);

impl BaseDimensions {
    /// Dimensionless scalar.
    pub const DIMENSIONLESS: Self = Self([0; 7]);
    /// Length dimension.
    pub const LENGTH: Self = Self([1, 0, 0, 0, 0, 0, 0]);
    /// Time dimension.
    pub const TIME: Self = Self([0, 0, 1, 0, 0, 0, 0]);
    /// Electric-current dimension.
    pub const CURRENT: Self = Self([0, 0, 0, 1, 0, 0, 0]);

    /// Construct an arbitrary exact SI dimension vector.
    pub const fn new(exponents: [i8; 7]) -> Self {
        Self(exponents)
    }

    /// Return the seven canonical exponents.
    pub const fn exponents(self) -> [i8; 7] {
        self.0
    }
}

/// One exact multiplicative unit relative to SI base units.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitDefinition {
    id: UnitId,
    symbol: String,
    dimensions: BaseDimensions,
    scale: Rational,
}

impl UnitDefinition {
    /// Construct a unit definition. [`GraphSchema::try_new`] validates it.
    pub fn new(
        id: UnitId,
        symbol: impl Into<String>,
        dimensions: BaseDimensions,
        scale: Rational,
    ) -> Self {
        Self {
            id,
            symbol: symbol.into(),
            dimensions,
            scale,
        }
    }

    /// Return the stable unit identifier.
    pub const fn id(&self) -> UnitId {
        self.id
    }

    /// Return the bounded display symbol. It is not dimensional authority.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Return the exact SI base-dimension vector.
    pub const fn dimensions(&self) -> BaseDimensions {
        self.dimensions
    }

    /// Return the exact positive scale relative to the SI base unit.
    pub const fn scale(&self) -> &Rational {
        &self.scale
    }
}

/// One named field in a record type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordField {
    id: RecordFieldId,
    name: String,
    value_type: GraphTypeId,
}

impl RecordField {
    /// Construct a record field. Schema construction sorts and validates it.
    pub fn new(id: RecordFieldId, name: impl Into<String>, value_type: GraphTypeId) -> Self {
        Self {
            id,
            name: name.into(),
            value_type,
        }
    }

    /// Return the field identifier.
    pub const fn id(&self) -> RecordFieldId {
        self.id
    }

    /// Return the stable field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the registered field value type.
    pub const fn value_type(&self) -> GraphTypeId {
        self.value_type
    }
}

/// Registered value shape. References are by stable type ID, so composite
/// schemas remain bounded and can be cycle-checked before any value is read.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    /// Boolean scalar.
    Boolean,
    /// Exact reduced rational carrying one registered unit.
    ExactRational {
        /// Physical unit.
        unit: UnitId,
    },
    /// Closed exact rational physical interval carrying one unit.
    MeasurementInterval {
        /// Physical unit.
        unit: UnitId,
    },
    /// Signed canonical integer multiplied by an exact positive lattice quantum.
    CanonicalI64 {
        /// Physical unit of the resulting value.
        unit: UnitId,
        /// Exact unit value represented by one integer count.
        quantum: Rational,
    },
    /// Unsigned canonical integer multiplied by an exact positive lattice quantum.
    CanonicalU64 {
        /// Physical unit of the resulting value.
        unit: UnitId,
        /// Exact unit value represented by one integer count.
        quantum: Rational,
    },
    /// UTF-8 text with an exact byte bound.
    Text {
        /// Maximum UTF-8 bytes.
        maximum_bytes: u32,
    },
    /// Opaque bytes with an exact bound.
    Bytes {
        /// Maximum bytes.
        maximum_bytes: u32,
    },
    /// Homogeneous bounded array.
    Array {
        /// Element type.
        element: GraphTypeId,
        /// Maximum element count.
        maximum_items: u32,
    },
    /// Required named fields.
    Record {
        /// Fields, canonicalized by field ID.
        fields: Vec<RecordField>,
    },
    /// Optional value.
    Option {
        /// Present-value type.
        value: GraphTypeId,
    },
    /// Explicit success or failure value.
    Result {
        /// Success type.
        ok: GraphTypeId,
        /// Failure type.
        error: GraphTypeId,
    },
    /// One instantaneous runtime event. Events are not document literals.
    Event {
        /// Event payload type.
        payload: GraphTypeId,
        /// Timestamp/ordering authority.
        clock: GraphClockId,
    },
    /// Bounded runtime stream in one explicit clock domain.
    Stream {
        /// Sample type.
        sample: GraphTypeId,
        /// Clock authority, resolved by the complete graph document.
        clock: GraphClockId,
        /// Maximum retained sample count.
        capacity: u32,
    },
    /// Capability-derived physical resource handle.
    ResourceHandle {
        /// Resource class required by this port/value.
        class: ResourceClassId,
    },
    /// Immutable cached global/local job identity.
    JobHandle,
}

/// One stable named type definition.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeDefinition {
    id: GraphTypeId,
    name: String,
    kind: TypeKind,
}

impl TypeDefinition {
    /// Construct a definition. Schema construction canonicalizes and validates it.
    pub fn new(id: GraphTypeId, name: impl Into<String>, kind: TypeKind) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
        }
    }

    /// Return the stable type identifier.
    pub const fn id(&self) -> GraphTypeId {
        self.id
    }

    /// Return the stable type name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the registered type shape.
    pub const fn kind(&self) -> &TypeKind {
        &self.kind
    }
}

/// Explicit schema/value allocation limits. The later graph document adds
/// node/wire limits without weakening these value bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphLimits {
    /// Maximum complete canonical graph-document byte length.
    pub maximum_document_bytes: usize,
    /// Maximum decimal digits in either exact-rational magnitude component.
    pub maximum_rational_digits: usize,
    /// Maximum registered units.
    pub maximum_units: usize,
    /// Maximum registered types.
    pub maximum_types: usize,
    /// Maximum fields in one record type.
    pub maximum_record_fields: usize,
    /// Maximum nested composite depth.
    pub maximum_value_depth: usize,
    /// Maximum value nodes in one literal tree.
    pub maximum_value_nodes: usize,
    /// Global ceiling for one array literal or type.
    pub maximum_array_items: usize,
    /// Global UTF-8 text ceiling.
    pub maximum_text_bytes: usize,
    /// Global opaque-byte ceiling.
    pub maximum_blob_bytes: usize,
    /// Global runtime stream capacity ceiling.
    pub maximum_stream_capacity: usize,
    /// Maximum registered graph clocks.
    pub maximum_clocks: usize,
    /// Maximum nodes in one document.
    pub maximum_nodes: usize,
    /// Maximum wires in one document.
    pub maximum_wires: usize,
    /// Maximum combined input/output ports on one node.
    pub maximum_ports_per_node: usize,
    /// Maximum retained parameters on one node.
    pub maximum_parameters_per_node: usize,
    /// Maximum UTF-8 bytes in one node display label.
    pub maximum_label_bytes: usize,
}

impl GraphLimits {
    /// Bounded first-release browser/editor policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_document_bytes: 16 * 1024 * 1024,
            maximum_rational_digits: 4_096,
            maximum_units: 64,
            maximum_types: 256,
            maximum_record_fields: 64,
            maximum_value_depth: 32,
            maximum_value_nodes: 4_096,
            maximum_array_items: 1_024,
            maximum_text_bytes: 64 * 1_024,
            maximum_blob_bytes: 1024 * 1024,
            maximum_stream_capacity: 4_096,
            maximum_clocks: 64,
            maximum_nodes: 4_096,
            maximum_wires: 8_192,
            maximum_ports_per_node: 64,
            maximum_parameters_per_node: 64,
            maximum_label_bytes: 256,
        }
    }

    fn validate(self) -> Result<(), GraphSchemaError> {
        if [
            self.maximum_document_bytes,
            self.maximum_rational_digits,
            self.maximum_units,
            self.maximum_types,
            self.maximum_record_fields,
            self.maximum_value_depth,
            self.maximum_value_nodes,
            self.maximum_array_items,
            self.maximum_text_bytes,
            self.maximum_blob_bytes,
            self.maximum_stream_capacity,
            self.maximum_clocks,
            self.maximum_nodes,
            self.maximum_wires,
            self.maximum_ports_per_node,
            self.maximum_parameters_per_node,
            self.maximum_label_bytes,
        ]
        .contains(&0)
        {
            return Err(GraphSchemaError::ZeroLimit);
        }
        Ok(())
    }
}

impl Default for GraphLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Capability-bound resource reference retained as data, never inferred from a
/// node label or a renderer hotspot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceGraphHandle {
    /// Stable physical MCU identity.
    pub device_id: DeviceId,
    /// Exact board-package capability identity.
    pub board_package_digest: Digest,
    /// Capability-derived resource class.
    pub class: ResourceClassId,
    /// Board-package-local canonical resource selector.
    pub resource_selector: u32,
}

/// Cached job identity retained without making it boot-local or executable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobGraphHandle {
    /// Stable participant MCU identity.
    pub device_id: DeviceId,
    /// Shared global job identity.
    pub global_job_digest: Digest,
    /// Participant-local immutable partition identity.
    pub partition_digest: Digest,
}

/// One field/value pair in a record literal.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordValueField {
    /// Stable schema field identity.
    pub field: RecordFieldId,
    /// Field value; its type is supplied by the record schema.
    pub value: GraphValue,
}

/// Exact typed graph literal. Runtime events and streams deliberately have no
/// literal variant.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphValue {
    /// Boolean scalar.
    Boolean(bool),
    /// Exact rational scalar.
    ExactRational(Rational),
    /// Closed measured interval.
    MeasurementInterval {
        /// Exact lower endpoint.
        lower: Rational,
        /// Exact upper endpoint.
        upper: Rational,
    },
    /// Canonical signed lattice count.
    CanonicalI64(i64),
    /// Canonical unsigned lattice count.
    CanonicalU64(u64),
    /// UTF-8 text.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// Homogeneous array.
    Array(Vec<Self>),
    /// Complete required record fields.
    Record(Vec<RecordValueField>),
    /// Absent optional value.
    OptionNone,
    /// Present optional value.
    OptionSome(Box<Self>),
    /// Successful result.
    ResultOk(Box<Self>),
    /// Failed result.
    ResultError(Box<Self>),
    /// Physical resource reference.
    ResourceHandle(ResourceGraphHandle),
    /// Immutable cached job reference.
    JobHandle(JobGraphHandle),
}

impl GraphValue {
    /// Return the structural literal variant for diagnostics.
    pub const fn kind(&self) -> GraphValueKind {
        match self {
            Self::Boolean(_) => GraphValueKind::Boolean,
            Self::ExactRational(_) => GraphValueKind::ExactRational,
            Self::MeasurementInterval { .. } => GraphValueKind::MeasurementInterval,
            Self::CanonicalI64(_) => GraphValueKind::CanonicalI64,
            Self::CanonicalU64(_) => GraphValueKind::CanonicalU64,
            Self::Text(_) => GraphValueKind::Text,
            Self::Bytes(_) => GraphValueKind::Bytes,
            Self::Array(_) => GraphValueKind::Array,
            Self::Record(_) => GraphValueKind::Record,
            Self::OptionNone | Self::OptionSome(_) => GraphValueKind::Option,
            Self::ResultOk(_) | Self::ResultError(_) => GraphValueKind::Result,
            Self::ResourceHandle(_) => GraphValueKind::ResourceHandle,
            Self::JobHandle(_) => GraphValueKind::JobHandle,
        }
    }
}

/// Structural literal variant used by typed mismatch diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphValueKind {
    /// Boolean.
    Boolean,
    /// Exact rational.
    ExactRational,
    /// Measured interval.
    MeasurementInterval,
    /// Signed canonical integer.
    CanonicalI64,
    /// Unsigned canonical integer.
    CanonicalU64,
    /// UTF-8 text.
    Text,
    /// Opaque bytes.
    Bytes,
    /// Array.
    Array,
    /// Record.
    Record,
    /// Option.
    Option,
    /// Result.
    Result,
    /// Resource handle.
    ResourceHandle,
    /// Job handle.
    JobHandle,
}

/// One root value paired with its registered type.
#[derive(Clone, Debug, PartialEq)]
pub struct TypedGraphValue {
    value_type: GraphTypeId,
    value: GraphValue,
}

impl TypedGraphValue {
    /// Construct and validate one complete bounded literal tree.
    pub fn try_new(
        schema: &GraphSchema,
        value_type: GraphTypeId,
        value: GraphValue,
    ) -> Result<Self, GraphSchemaError> {
        schema.validate_value(value_type, &value)?;
        Ok(Self { value_type, value })
    }

    /// Return the root registered type.
    pub const fn value_type(&self) -> GraphTypeId {
        self.value_type
    }

    /// Borrow the exact retained literal.
    pub const fn value(&self) -> &GraphValue {
        &self.value
    }
}

/// Canonicalized unit/type registry shared by graph documents and executors.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphSchema {
    limits: GraphLimits,
    units: Vec<UnitDefinition>,
    types: Vec<TypeDefinition>,
}

impl GraphSchema {
    /// Sort, validate, and retain one complete value schema.
    pub fn try_new(
        limits: GraphLimits,
        mut units: Vec<UnitDefinition>,
        mut types: Vec<TypeDefinition>,
    ) -> Result<Self, GraphSchemaError> {
        limits.validate()?;
        if units.len() > limits.maximum_units {
            return Err(GraphSchemaError::LimitExceeded("unit count"));
        }
        if types.len() > limits.maximum_types {
            return Err(GraphSchemaError::LimitExceeded("type count"));
        }
        units.sort_unstable_by_key(UnitDefinition::id);
        types.sort_unstable_by_key(TypeDefinition::id);
        for definition in &mut types {
            if let TypeKind::Record { fields } = &mut definition.kind {
                fields.sort_unstable_by_key(RecordField::id);
            }
        }

        let schema = Self {
            limits,
            units,
            types,
        };
        schema.validate_units()?;
        schema.validate_types()?;
        Ok(schema)
    }

    /// Return the active allocation limits.
    pub const fn limits(&self) -> GraphLimits {
        self.limits
    }

    /// Borrow units in canonical ID order.
    pub fn units(&self) -> &[UnitDefinition] {
        &self.units
    }

    /// Borrow types in canonical ID order.
    pub fn types(&self) -> &[TypeDefinition] {
        &self.types
    }

    /// Look up one unit by exact identifier.
    pub fn unit(&self, id: UnitId) -> Option<&UnitDefinition> {
        self.units
            .binary_search_by_key(&id, UnitDefinition::id)
            .ok()
            .map(|index| &self.units[index])
    }

    /// Look up one type by exact identifier.
    pub fn value_type(&self, id: GraphTypeId) -> Option<&TypeDefinition> {
        self.type_index(id).map(|index| &self.types[index])
    }

    /// Revalidate an already retained typed value against this exact schema.
    pub fn validate_typed_value(&self, value: &TypedGraphValue) -> Result<(), GraphSchemaError> {
        self.validate_value(value.value_type, &value.value)
    }

    /// Validate a raw root type/value pair without retaining it.
    pub fn validate_value(
        &self,
        value_type: GraphTypeId,
        value: &GraphValue,
    ) -> Result<(), GraphSchemaError> {
        let mut budget = ValueBudget::default();
        self.validate_value_inner(value_type, value, 1, &mut budget)
    }

    fn validate_units(&self) -> Result<(), GraphSchemaError> {
        let mut symbols = BTreeSet::new();
        let mut previous = None;
        for unit in &self.units {
            if unit.id.get() == 0 {
                return Err(GraphSchemaError::ZeroIdentifier("unit"));
            }
            if previous == Some(unit.id) {
                return Err(GraphSchemaError::DuplicateIdentifier("unit"));
            }
            previous = Some(unit.id);
            if !valid_symbol(&unit.symbol) {
                return Err(GraphSchemaError::InvalidName("unit symbol"));
            }
            if !symbols.insert(unit.symbol.as_str()) {
                return Err(GraphSchemaError::DuplicateName("unit symbol"));
            }
            if unit.scale.is_zero() || unit.scale.is_negative() {
                return Err(GraphSchemaError::InvalidUnitScale(unit.id));
            }
            validate_rational_magnitude(&unit.scale, self.limits.maximum_rational_digits)?;
        }
        Ok(())
    }

    fn validate_types(&self) -> Result<(), GraphSchemaError> {
        let mut names = BTreeSet::new();
        let mut previous = None;
        for definition in &self.types {
            if definition.id.get() == 0 {
                return Err(GraphSchemaError::ZeroIdentifier("type"));
            }
            if previous == Some(definition.id) {
                return Err(GraphSchemaError::DuplicateIdentifier("type"));
            }
            previous = Some(definition.id);
            if !valid_stable_name(&definition.name) {
                return Err(GraphSchemaError::InvalidName("type"));
            }
            if !names.insert(definition.name.as_str()) {
                return Err(GraphSchemaError::DuplicateName("type"));
            }
            self.validate_type_kind(definition)?;
        }

        let mut states = vec![0_u8; self.types.len()];
        for index in 0..self.types.len() {
            self.visit_type(index, 1, &mut states)?;
        }
        Ok(())
    }

    fn validate_type_kind(&self, definition: &TypeDefinition) -> Result<(), GraphSchemaError> {
        match &definition.kind {
            TypeKind::Boolean | TypeKind::JobHandle => {}
            TypeKind::ExactRational { unit }
            | TypeKind::MeasurementInterval { unit }
            | TypeKind::CanonicalI64 { unit, .. }
            | TypeKind::CanonicalU64 { unit, .. } => {
                if self.unit(*unit).is_none() {
                    return Err(GraphSchemaError::UnknownUnit(*unit));
                }
                if let TypeKind::CanonicalI64 { quantum, .. }
                | TypeKind::CanonicalU64 { quantum, .. } = &definition.kind
                    && (quantum.is_zero() || quantum.is_negative())
                {
                    return Err(GraphSchemaError::InvalidLatticeQuantum(definition.id));
                }
                if let TypeKind::CanonicalI64 { quantum, .. }
                | TypeKind::CanonicalU64 { quantum, .. } = &definition.kind
                {
                    validate_rational_magnitude(quantum, self.limits.maximum_rational_digits)?;
                }
            }
            TypeKind::Text { maximum_bytes } => {
                validate_u32_limit(
                    *maximum_bytes,
                    self.limits.maximum_text_bytes,
                    "text byte bound",
                )?;
            }
            TypeKind::Bytes { maximum_bytes } => {
                validate_u32_limit(
                    *maximum_bytes,
                    self.limits.maximum_blob_bytes,
                    "blob byte bound",
                )?;
            }
            TypeKind::Array {
                element,
                maximum_items,
            } => {
                self.require_type(*element)?;
                validate_u32_limit(
                    *maximum_items,
                    self.limits.maximum_array_items,
                    "array item bound",
                )?;
            }
            TypeKind::Record { fields } => {
                if fields.len() > self.limits.maximum_record_fields {
                    return Err(GraphSchemaError::LimitExceeded("record field count"));
                }
                let mut names = BTreeSet::new();
                let mut previous = None;
                for field in fields {
                    if field.id.get() == 0 {
                        return Err(GraphSchemaError::ZeroIdentifier("record field"));
                    }
                    if previous == Some(field.id) {
                        return Err(GraphSchemaError::DuplicateIdentifier("record field"));
                    }
                    previous = Some(field.id);
                    if !valid_stable_name(&field.name) {
                        return Err(GraphSchemaError::InvalidName("record field"));
                    }
                    if !names.insert(field.name.as_str()) {
                        return Err(GraphSchemaError::DuplicateName("record field"));
                    }
                    self.require_type(field.value_type)?;
                }
            }
            TypeKind::Option { value } => {
                self.require_type(*value)?;
            }
            TypeKind::Result { ok, error } => {
                self.require_type(*ok)?;
                self.require_type(*error)?;
            }
            TypeKind::Event { payload, clock } => {
                self.require_type(*payload)?;
                if clock.get() == 0 {
                    return Err(GraphSchemaError::ZeroIdentifier("event clock"));
                }
            }
            TypeKind::Stream {
                sample,
                clock,
                capacity,
            } => {
                self.require_type(*sample)?;
                if clock.get() == 0 {
                    return Err(GraphSchemaError::ZeroIdentifier("stream clock"));
                }
                validate_u32_limit(
                    *capacity,
                    self.limits.maximum_stream_capacity,
                    "stream capacity",
                )?;
            }
            TypeKind::ResourceHandle { class } => {
                if class.get() == 0 {
                    return Err(GraphSchemaError::ZeroIdentifier("resource class"));
                }
            }
        }
        Ok(())
    }

    fn visit_type(
        &self,
        index: usize,
        depth: usize,
        states: &mut [u8],
    ) -> Result<(), GraphSchemaError> {
        if depth > self.limits.maximum_value_depth {
            return Err(GraphSchemaError::TypeDepthExceeded(self.types[index].id));
        }
        match states[index] {
            2 => return Ok(()),
            1 => return Err(GraphSchemaError::RecursiveType(self.types[index].id)),
            _ => {}
        }
        states[index] = 1;
        for reference in referenced_types(&self.types[index].kind) {
            let target = self.require_type(reference)?;
            self.visit_type(target, depth + 1, states)?;
        }
        states[index] = 2;
        Ok(())
    }

    fn validate_value_inner(
        &self,
        value_type: GraphTypeId,
        value: &GraphValue,
        depth: usize,
        budget: &mut ValueBudget,
    ) -> Result<(), GraphSchemaError> {
        if depth > self.limits.maximum_value_depth {
            return Err(GraphSchemaError::ValueDepthExceeded);
        }
        budget.nodes = budget
            .nodes
            .checked_add(1)
            .ok_or(GraphSchemaError::ValueNodeLimitExceeded)?;
        if budget.nodes > self.limits.maximum_value_nodes {
            return Err(GraphSchemaError::ValueNodeLimitExceeded);
        }
        let definition = self
            .value_type(value_type)
            .ok_or(GraphSchemaError::UnknownType(value_type))?;
        match (&definition.kind, value) {
            (TypeKind::Boolean, GraphValue::Boolean(_))
            | (TypeKind::CanonicalI64 { .. }, GraphValue::CanonicalI64(_))
            | (TypeKind::CanonicalU64 { .. }, GraphValue::CanonicalU64(_)) => Ok(()),
            (TypeKind::ExactRational { .. }, GraphValue::ExactRational(value)) => {
                validate_rational_magnitude(value, self.limits.maximum_rational_digits)
            }
            (
                TypeKind::MeasurementInterval { .. },
                GraphValue::MeasurementInterval { lower, upper },
            ) => {
                if lower > upper {
                    Err(GraphSchemaError::ReversedMeasurement)
                } else {
                    validate_rational_magnitude(lower, self.limits.maximum_rational_digits)?;
                    validate_rational_magnitude(upper, self.limits.maximum_rational_digits)
                }
            }
            (TypeKind::Text { maximum_bytes }, GraphValue::Text(text)) => {
                if text.len() > *maximum_bytes as usize
                    || text.len() > self.limits.maximum_text_bytes
                {
                    Err(GraphSchemaError::LimitExceeded("text literal"))
                } else {
                    Ok(())
                }
            }
            (TypeKind::Bytes { maximum_bytes }, GraphValue::Bytes(bytes)) => {
                if bytes.len() > *maximum_bytes as usize
                    || bytes.len() > self.limits.maximum_blob_bytes
                {
                    Err(GraphSchemaError::LimitExceeded("byte literal"))
                } else {
                    Ok(())
                }
            }
            (
                TypeKind::Array {
                    element,
                    maximum_items,
                },
                GraphValue::Array(items),
            ) => {
                if items.len() > *maximum_items as usize
                    || items.len() > self.limits.maximum_array_items
                {
                    return Err(GraphSchemaError::LimitExceeded("array literal"));
                }
                for item in items {
                    self.validate_value_inner(*element, item, depth + 1, budget)?;
                }
                Ok(())
            }
            (TypeKind::Record { fields }, GraphValue::Record(values)) => {
                if fields.len() != values.len() {
                    return Err(GraphSchemaError::RecordShape);
                }
                for (field, value) in fields.iter().zip(values) {
                    if field.id != value.field {
                        return Err(GraphSchemaError::RecordShape);
                    }
                    self.validate_value_inner(field.value_type, &value.value, depth + 1, budget)?;
                }
                Ok(())
            }
            (TypeKind::Option { .. }, GraphValue::OptionNone) => Ok(()),
            (TypeKind::Option { value: inner }, GraphValue::OptionSome(value)) => {
                self.validate_value_inner(*inner, value, depth + 1, budget)
            }
            (TypeKind::Result { ok, .. }, GraphValue::ResultOk(value)) => {
                self.validate_value_inner(*ok, value, depth + 1, budget)
            }
            (TypeKind::Result { error, .. }, GraphValue::ResultError(value)) => {
                self.validate_value_inner(*error, value, depth + 1, budget)
            }
            (TypeKind::ResourceHandle { class }, GraphValue::ResourceHandle(handle)) => {
                if handle.class != *class
                    || device_id_is_zero(handle.device_id)
                    || handle.board_package_digest.is_zero()
                {
                    Err(GraphSchemaError::InvalidHandle)
                } else {
                    Ok(())
                }
            }
            (TypeKind::JobHandle, GraphValue::JobHandle(handle)) => {
                if device_id_is_zero(handle.device_id)
                    || handle.global_job_digest.is_zero()
                    || handle.partition_digest.is_zero()
                {
                    Err(GraphSchemaError::InvalidHandle)
                } else {
                    Ok(())
                }
            }
            (TypeKind::Event { .. } | TypeKind::Stream { .. }, _) => {
                Err(GraphSchemaError::RuntimeOnlyType(value_type))
            }
            _ => Err(GraphSchemaError::TypeMismatch {
                expected: value_type,
                received: value.kind(),
            }),
        }
    }

    fn require_type(&self, id: GraphTypeId) -> Result<usize, GraphSchemaError> {
        self.type_index(id).ok_or(GraphSchemaError::UnknownType(id))
    }

    fn type_index(&self, id: GraphTypeId) -> Option<usize> {
        self.types
            .binary_search_by_key(&id, TypeDefinition::id)
            .ok()
    }
}

#[derive(Default)]
struct ValueBudget {
    nodes: usize,
}

fn referenced_types(kind: &TypeKind) -> Vec<GraphTypeId> {
    match kind {
        TypeKind::Array { element, .. } => vec![*element],
        TypeKind::Record { fields } => fields.iter().map(RecordField::value_type).collect(),
        TypeKind::Option { value } => vec![*value],
        TypeKind::Result { ok, error } => vec![*ok, *error],
        TypeKind::Event { payload, .. } => vec![*payload],
        TypeKind::Stream { sample, .. } => vec![*sample],
        TypeKind::Boolean
        | TypeKind::ExactRational { .. }
        | TypeKind::MeasurementInterval { .. }
        | TypeKind::CanonicalI64 { .. }
        | TypeKind::CanonicalU64 { .. }
        | TypeKind::Text { .. }
        | TypeKind::Bytes { .. }
        | TypeKind::ResourceHandle { .. }
        | TypeKind::JobHandle => Vec::new(),
    }
}

fn validate_u32_limit(
    value: u32,
    maximum: usize,
    name: &'static str,
) -> Result<(), GraphSchemaError> {
    if value == 0 || value as usize > maximum {
        Err(GraphSchemaError::LimitExceeded(name))
    } else {
        Ok(())
    }
}

fn validate_rational_magnitude(
    value: &Rational,
    maximum_digits: usize,
) -> Result<(), GraphSchemaError> {
    if value.numerator().to_string().len() > maximum_digits
        || value.denominator().to_string().len() > maximum_digits
    {
        Err(GraphSchemaError::LimitExceeded("rational magnitude"))
    } else {
        Ok(())
    }
}

fn valid_stable_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_alphabetic()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn valid_symbol(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .chars()
            .all(|character| !character.is_control() && !character.is_whitespace())
}

fn device_id_is_zero(id: DeviceId) -> bool {
    id.0.iter().all(|byte| *byte == 0)
}

/// Rejection at a bounded graph value/schema boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphSchemaError {
    /// A caller supplied a zero global limit.
    ZeroLimit,
    /// A count or per-type bound exceeded policy.
    LimitExceeded(&'static str),
    /// A stable identifier was zero.
    ZeroIdentifier(&'static str),
    /// A stable identifier was duplicated.
    DuplicateIdentifier(&'static str),
    /// A stable name or unit symbol was duplicated.
    DuplicateName(&'static str),
    /// A stable name or symbol was malformed.
    InvalidName(&'static str),
    /// A unit scale was zero or negative.
    InvalidUnitScale(UnitId),
    /// A lattice quantum was zero or negative.
    InvalidLatticeQuantum(GraphTypeId),
    /// A type referenced an unknown unit.
    UnknownUnit(UnitId),
    /// A schema or value referenced an unknown type.
    UnknownType(GraphTypeId),
    /// A registered type directly or indirectly referenced itself.
    RecursiveType(GraphTypeId),
    /// Registered type nesting exceeded the value-depth policy.
    TypeDepthExceeded(GraphTypeId),
    /// A literal nesting depth exceeded policy.
    ValueDepthExceeded,
    /// A literal tree contained too many nodes.
    ValueNodeLimitExceeded,
    /// A literal variant did not match its registered type.
    TypeMismatch {
        /// Registered expected type.
        expected: GraphTypeId,
        /// Supplied literal variant.
        received: GraphValueKind,
    },
    /// A measured interval had lower > upper.
    ReversedMeasurement,
    /// Record fields were missing, extra, duplicated, or out of canonical order.
    RecordShape,
    /// An event or stream was incorrectly represented as a document literal.
    RuntimeOnlyType(GraphTypeId),
    /// A resource/job handle omitted or contradicted identity authority.
    InvalidHandle,
}

impl fmt::Display for GraphSchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph limit is zero"),
            Self::LimitExceeded(name) => write!(formatter, "graph {name} exceeds its limit"),
            Self::ZeroIdentifier(kind) => write!(formatter, "graph {kind} identifier is zero"),
            Self::DuplicateIdentifier(kind) => {
                write!(formatter, "graph {kind} identifier is duplicated")
            }
            Self::DuplicateName(kind) => write!(formatter, "graph {kind} name is duplicated"),
            Self::InvalidName(kind) => write!(formatter, "graph {kind} name is invalid"),
            Self::InvalidUnitScale(id) => write!(formatter, "unit {id:?} scale is not positive"),
            Self::InvalidLatticeQuantum(id) => {
                write!(formatter, "type {id:?} lattice quantum is not positive")
            }
            Self::UnknownUnit(id) => write!(formatter, "unknown graph unit {id:?}"),
            Self::UnknownType(id) => write!(formatter, "unknown graph type {id:?}"),
            Self::RecursiveType(id) => write!(formatter, "graph type {id:?} is recursive"),
            Self::TypeDepthExceeded(id) => {
                write!(formatter, "graph type {id:?} exceeds nesting policy")
            }
            Self::ValueDepthExceeded => formatter.write_str("graph literal is nested too deeply"),
            Self::ValueNodeLimitExceeded => {
                formatter.write_str("graph literal has too many value nodes")
            }
            Self::TypeMismatch { expected, received } => {
                write!(formatter, "graph type {expected:?} rejects {received:?}")
            }
            Self::ReversedMeasurement => {
                formatter.write_str("graph measurement interval is reversed")
            }
            Self::RecordShape => formatter.write_str("graph record literal shape is not canonical"),
            Self::RuntimeOnlyType(id) => {
                write!(formatter, "runtime graph type {id:?} cannot be a literal")
            }
            Self::InvalidHandle => formatter.write_str("graph handle identity is invalid"),
        }
    }
}

impl std::error::Error for GraphSchemaError {}

#[cfg(test)]
mod tests {
    use super::*;

    const MM: UnitId = UnitId::new(1);
    const SECOND: UnitId = UnitId::new(2);
    const BOOLEAN: GraphTypeId = GraphTypeId::new(1);
    const EXACT_MM: GraphTypeId = GraphTypeId::new(2);
    const MEASURED_MM: GraphTypeId = GraphTypeId::new(3);
    const TEXT: GraphTypeId = GraphTypeId::new(4);
    const ARRAY: GraphTypeId = GraphTypeId::new(5);
    const RECORD: GraphTypeId = GraphTypeId::new(6);
    const OPTION: GraphTypeId = GraphTypeId::new(7);
    const RESULT: GraphTypeId = GraphTypeId::new(8);
    const EVENT: GraphTypeId = GraphTypeId::new(9);
    const STREAM: GraphTypeId = GraphTypeId::new(10);
    const RESOURCE: GraphTypeId = GraphTypeId::new(11);
    const JOB: GraphTypeId = GraphTypeId::new(12);
    const RESOURCE_CLASS: ResourceClassId = ResourceClassId::new(7);

    fn schema() -> GraphSchema {
        GraphSchema::try_new(
            GraphLimits::interactive(),
            vec![
                UnitDefinition::new(SECOND, "s", BaseDimensions::TIME, Rational::from(1)),
                UnitDefinition::new(
                    MM,
                    "mm",
                    BaseDimensions::LENGTH,
                    Rational::fraction(1, 1_000).unwrap(),
                ),
            ],
            vec![
                TypeDefinition::new(JOB, "core.job", TypeKind::JobHandle),
                TypeDefinition::new(
                    RECORD,
                    "fixture.point",
                    TypeKind::Record {
                        fields: vec![
                            RecordField::new(RecordFieldId::new(2), "label", TEXT),
                            RecordField::new(RecordFieldId::new(1), "position", EXACT_MM),
                        ],
                    },
                ),
                TypeDefinition::new(BOOLEAN, "core.bool", TypeKind::Boolean),
                TypeDefinition::new(EXACT_MM, "exact.mm", TypeKind::ExactRational { unit: MM }),
                TypeDefinition::new(
                    MEASURED_MM,
                    "measured.mm",
                    TypeKind::MeasurementInterval { unit: MM },
                ),
                TypeDefinition::new(TEXT, "core.text32", TypeKind::Text { maximum_bytes: 32 }),
                TypeDefinition::new(
                    ARRAY,
                    "fixture.points",
                    TypeKind::Array {
                        element: RECORD,
                        maximum_items: 4,
                    },
                ),
                TypeDefinition::new(
                    OPTION,
                    "fixture.optional-point",
                    TypeKind::Option { value: RECORD },
                ),
                TypeDefinition::new(
                    RESULT,
                    "fixture.point-result",
                    TypeKind::Result {
                        ok: RECORD,
                        error: TEXT,
                    },
                ),
                TypeDefinition::new(
                    EVENT,
                    "fixture.point-event",
                    TypeKind::Event {
                        payload: RECORD,
                        clock: GraphClockId::new(1),
                    },
                ),
                TypeDefinition::new(
                    STREAM,
                    "fixture.point-stream",
                    TypeKind::Stream {
                        sample: RECORD,
                        clock: GraphClockId::new(1),
                        capacity: 128,
                    },
                ),
                TypeDefinition::new(
                    RESOURCE,
                    "fixture.resource",
                    TypeKind::ResourceHandle {
                        class: RESOURCE_CLASS,
                    },
                ),
            ],
        )
        .unwrap()
    }

    fn point(position: Rational, label: &str) -> GraphValue {
        GraphValue::Record(vec![
            RecordValueField {
                field: RecordFieldId::new(1),
                value: GraphValue::ExactRational(position),
            },
            RecordValueField {
                field: RecordFieldId::new(2),
                value: GraphValue::Text(label.to_owned()),
            },
        ])
    }

    #[test]
    fn schema_canonicalizes_ids_fields_and_retains_exact_units() {
        let schema = schema();
        assert_eq!(schema.units()[0].id(), MM);
        assert_eq!(
            schema.units()[0].scale(),
            &Rational::fraction(1, 1_000).unwrap()
        );
        assert_eq!(schema.types()[0].id(), BOOLEAN);
        let TypeKind::Record { fields } = schema.value_type(RECORD).unwrap().kind() else {
            panic!("record type expected");
        };
        assert_eq!(fields[0].id(), RecordFieldId::new(1));
        assert_eq!(fields[1].id(), RecordFieldId::new(2));
    }

    #[test]
    fn composite_literals_validate_without_float_or_implicit_unit_conversion() {
        let schema = schema();
        let value = TypedGraphValue::try_new(
            &schema,
            ARRAY,
            GraphValue::Array(vec![
                point(Rational::fraction(1, 10).unwrap(), "origin"),
                point(Rational::fraction(2, 10).unwrap(), "target"),
            ]),
        )
        .unwrap();
        schema.validate_typed_value(&value).unwrap();

        let measured = TypedGraphValue::try_new(
            &schema,
            MEASURED_MM,
            GraphValue::MeasurementInterval {
                lower: Rational::fraction(99, 10).unwrap(),
                upper: Rational::fraction(101, 10).unwrap(),
            },
        )
        .unwrap();
        assert!(matches!(
            measured.value(),
            GraphValue::MeasurementInterval { lower, upper }
                if lower == &Rational::fraction(99, 10).unwrap()
                    && upper == &Rational::fraction(101, 10).unwrap()
        ));
    }

    #[test]
    fn wrong_variant_reversed_interval_and_record_order_fail_at_the_value() {
        let schema = schema();
        assert!(matches!(
            TypedGraphValue::try_new(&schema, EXACT_MM, GraphValue::CanonicalI64(1)),
            Err(GraphSchemaError::TypeMismatch { .. })
        ));
        assert_eq!(
            TypedGraphValue::try_new(
                &schema,
                MEASURED_MM,
                GraphValue::MeasurementInterval {
                    lower: Rational::from(2),
                    upper: Rational::from(1),
                }
            ),
            Err(GraphSchemaError::ReversedMeasurement)
        );
        let mut reversed = match point(Rational::zero(), "point") {
            GraphValue::Record(fields) => fields,
            _ => unreachable!(),
        };
        reversed.reverse();
        assert_eq!(
            TypedGraphValue::try_new(&schema, RECORD, GraphValue::Record(reversed)),
            Err(GraphSchemaError::RecordShape)
        );
    }

    #[test]
    fn arrays_text_and_entire_value_tree_are_bounded() {
        let schema = schema();
        assert!(matches!(
            TypedGraphValue::try_new(
                &schema,
                ARRAY,
                GraphValue::Array((0..5).map(|_| point(Rational::zero(), "p")).collect())
            ),
            Err(GraphSchemaError::LimitExceeded("array literal"))
        ));
        assert!(matches!(
            TypedGraphValue::try_new(&schema, TEXT, GraphValue::Text("x".repeat(33))),
            Err(GraphSchemaError::LimitExceeded("text literal"))
        ));
    }

    #[test]
    fn recursive_and_unknown_type_references_fail_before_values_exist() {
        let recursive = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![TypeDefinition::new(
                GraphTypeId::new(1),
                "recursive",
                TypeKind::Option {
                    value: GraphTypeId::new(1),
                },
            )],
        );
        assert_eq!(
            recursive,
            Err(GraphSchemaError::RecursiveType(GraphTypeId::new(1)))
        );

        let unknown = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![TypeDefinition::new(
                GraphTypeId::new(1),
                "unknown-element",
                TypeKind::Array {
                    element: GraphTypeId::new(2),
                    maximum_items: 1,
                },
            )],
        );
        assert_eq!(
            unknown,
            Err(GraphSchemaError::UnknownType(GraphTypeId::new(2)))
        );
    }

    #[test]
    fn runtime_types_have_no_literal_and_handles_retain_identity() {
        let schema = schema();
        assert_eq!(
            TypedGraphValue::try_new(&schema, EVENT, GraphValue::Boolean(true)),
            Err(GraphSchemaError::RuntimeOnlyType(EVENT))
        );
        assert_eq!(
            TypedGraphValue::try_new(&schema, STREAM, GraphValue::Array(Vec::new())),
            Err(GraphSchemaError::RuntimeOnlyType(STREAM))
        );
        assert_eq!(
            TypedGraphValue::try_new(
                &schema,
                RESOURCE,
                GraphValue::ResourceHandle(ResourceGraphHandle {
                    device_id: DeviceId([0; 16]),
                    board_package_digest: Digest([1; 32]),
                    class: RESOURCE_CLASS,
                    resource_selector: 4,
                })
            ),
            Err(GraphSchemaError::InvalidHandle)
        );
        TypedGraphValue::try_new(
            &schema,
            JOB,
            GraphValue::JobHandle(JobGraphHandle {
                device_id: DeviceId([1; 16]),
                global_job_digest: Digest([2; 32]),
                partition_digest: Digest([3; 32]),
            }),
        )
        .unwrap();
    }

    #[test]
    fn zero_negative_duplicate_and_overwide_schema_facts_reject() {
        for unit in [
            UnitDefinition::new(
                UnitId::new(0),
                "bad",
                BaseDimensions::DIMENSIONLESS,
                Rational::from(1),
            ),
            UnitDefinition::new(
                UnitId::new(1),
                "bad",
                BaseDimensions::DIMENSIONLESS,
                Rational::zero(),
            ),
        ] {
            assert!(
                GraphSchema::try_new(GraphLimits::interactive(), vec![unit], Vec::new()).is_err()
            );
        }
        assert_eq!(
            GraphSchema::try_new(
                GraphLimits::interactive(),
                Vec::new(),
                vec![
                    TypeDefinition::new(BOOLEAN, "one", TypeKind::Boolean),
                    TypeDefinition::new(BOOLEAN, "two", TypeKind::Boolean),
                ]
            ),
            Err(GraphSchemaError::DuplicateIdentifier("type"))
        );
    }
}
