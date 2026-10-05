//! Capability-derived physical-resource node authoring.
//!
//! A capability document describes what one caller-authenticated firmware image
//! can execute. This module verifies content identity; it does not authenticate
//! a transport or device session. A deployment registry separately describes reviewed graph node
//! semantics and their fixed opcode bindings. This module intersects those
//! two authorities and produces concrete editor prototypes whose resource
//! handles are already bound to one device and capability digest.
//!
//! The catalog is authoring assistance only. Creating one of its prototypes
//! does not bypass structural validation, semantic analysis, implementation
//! admission, scheduling proof, or final capability-bound lowering.

use core::fmt;

use alumina_board::{
    GraphResourceAccess, GraphResourceDescriptor, OwnerDomain, ResourceId, SupportLevel,
};
use alumina_capability::{
    CapabilityDocumentError, CapabilityIdentity, decode_graph_execution, encode_resource_id,
};
use alumina_graph_ir::GraphIrOpcode;
use alumina_protocol::Digest;

use super::{
    CanonicalGraphWorkspaceEncoding, ExecutionDomain, GraphAnalysisError, GraphDeploymentNodeKind,
    GraphDeploymentRegistry, GraphDeploymentTarget, GraphNodeId, GraphNodePrototype,
    GraphSchemaError, GraphValue, GraphValuePathError, GraphValuePathSegment,
    GraphWorkspaceDocument, GraphWorkspaceError, NodeDefinition, NodeKind, NodeParameter,
    ResourceGraphHandle, TypeKind, TypedGraphValue, analyze_graph_draft, encode_graph_workspace,
};

/// Caller-owned bounds for one derived target-resource palette.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphCapabilityCatalogLimits {
    /// Maximum complete capability-document bytes accepted for derivation.
    pub maximum_capability_bytes: usize,
    /// Maximum concrete kind/resource entries retained in the palette.
    pub maximum_entries: usize,
}

impl GraphCapabilityCatalogLimits {
    /// Bounded first interactive policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_capability_bytes: 4 * 1024 * 1024,
            maximum_entries: 4_096,
        }
    }

    fn validate(self) -> Result<(), GraphCapabilityCatalogError> {
        if self.maximum_capability_bytes == 0 || self.maximum_entries == 0 {
            Err(GraphCapabilityCatalogError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphCapabilityCatalogLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// One concrete, caller-authenticated resource-node choice offered by an editor.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphCapabilityNodeEntry {
    resource: GraphResourceDescriptor,
    kind: NodeKind,
    resource_parameter: u32,
    prototype: GraphNodePrototype,
}

impl GraphCapabilityNodeEntry {
    /// Return the exact graph-addressable physical resource capability.
    pub const fn resource(&self) -> GraphResourceDescriptor {
        self.resource
    }

    /// Return the reviewed node kind that will consume the resource handle.
    pub const fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Return the reviewed node-local parameter that carries this resource.
    pub const fn resource_parameter_id(&self) -> u32 {
        self.resource_parameter
    }

    /// Borrow the exact target-bound resource parameter.
    ///
    /// Derived catalogs always return `Some`. The optional result keeps
    /// consumers fail-closed if an in-memory invariant is contradicted.
    pub fn resource_parameter(&self) -> Option<&NodeParameter> {
        self.prototype
            .parameters()
            .iter()
            .find(|parameter| parameter.id() == self.resource_parameter)
    }

    /// Borrow the fully materialized, target-bound editor prototype.
    pub const fn prototype(&self) -> &GraphNodePrototype {
        &self.prototype
    }

    /// Clone the prototype for transactional insertion into an `ALGW` draft.
    pub fn instantiate(&self) -> GraphNodePrototype {
        self.prototype.clone()
    }
}

/// Derived intersection of one caller-authenticated image and reviewed registry.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphCapabilityNodeCatalog {
    identity: CapabilityIdentity,
    target: GraphDeploymentTarget,
    advertised_resource_count: usize,
    entries: Vec<GraphCapabilityNodeEntry>,
}

impl GraphCapabilityNodeCatalog {
    /// Return the exact complete capability-document identity.
    pub const fn capability_identity(&self) -> CapabilityIdentity {
        self.identity
    }

    /// Return the device/capability/configuration identities used in handles.
    pub const fn target(&self) -> GraphDeploymentTarget {
        self.target
    }

    /// Number of graph resources advertised by the image before intersection
    /// with the reviewed host registry.
    pub const fn advertised_resource_count(&self) -> usize {
        self.advertised_resource_count
    }

    /// Borrow concrete entries in canonical kind/resource order.
    pub fn entries(&self) -> &[GraphCapabilityNodeEntry] {
        &self.entries
    }

    /// Resolve the catalog entry that supplies one exact capability-bound
    /// resource handle.
    ///
    /// This compares the complete device, board-package, class, and canonical
    /// resource selector. A merely well-shaped handle is not catalog evidence.
    pub fn entry_index_for_handle(&self, handle: ResourceGraphHandle) -> Option<usize> {
        self.entries.iter().position(|entry| {
            matches!(
                entry.resource_parameter().map(|parameter| parameter.value().value()),
                Some(GraphValue::ResourceHandle(candidate)) if *candidate == handle
            )
        })
    }

    /// Resolve the catalog entry that exactly supplies one existing node's
    /// physical-resource identity.
    ///
    /// This checks kind, concrete execution owner, port shape, parameter ID,
    /// name, registered type, and complete typed handle value. It does not
    /// infer authority from the node label or a numeric resource selector.
    pub fn entry_index_for_node(&self, node: &NodeDefinition) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| node_matches_entry(node, entry))
    }
}

/// Rejection while replacing one catalog-managed physical-resource identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphCapabilityResourceSelectionError {
    /// The requested catalog index was absent.
    UnknownEntry(usize),
    /// The requested graph node was absent.
    UnknownNode(GraphNodeId),
    /// The requested node parameter was absent.
    UnknownParameter {
        /// Exact graph node.
        node: GraphNodeId,
        /// Requested parameter identity.
        parameter: u32,
    },
    /// The supplied structural path was invalid for the registered value.
    ValuePath(GraphValuePathError),
    /// The selected leaf was not a registered resource-handle value.
    NotResourceHandleValue {
        /// Exact graph node.
        node: GraphNodeId,
        /// Exact node-local parameter.
        parameter: u32,
    },
    /// The existing leaf did not carry an exact value offered by this catalog.
    ParameterNotCatalogBound {
        /// Exact graph node.
        node: GraphNodeId,
        /// Exact node-local parameter.
        parameter: u32,
    },
    /// The selected leaf already carries the requested exact catalog entry.
    AlreadySelected {
        /// Exact graph node.
        node: GraphNodeId,
        /// Exact node-local parameter.
        parameter: u32,
        /// Exact catalog entry.
        entry: usize,
    },
    /// Another root or composite parameter leaf already carries the requested
    /// resource identity.
    ResourceAlreadySelected {
        /// Exact catalog entry requested by the caller.
        entry: usize,
        /// Existing graph node that retains the identity.
        node: GraphNodeId,
        /// Existing node-local parameter that retains the identity.
        parameter: u32,
    },
    /// A derived entry contradicted an internal invariant.
    InvalidEntry {
        /// Exact catalog entry.
        entry: usize,
        /// Contradictory fact.
        aspect: &'static str,
    },
    /// The transactional workspace edit failed structural validation.
    Workspace(GraphWorkspaceError),
    /// The complete candidate failed the reviewed semantic registry.
    Analysis(GraphAnalysisError),
}

impl fmt::Display for GraphCapabilityResourceSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEntry(entry) => {
                write!(formatter, "graph capability entry {entry} is unavailable")
            }
            Self::UnknownNode(node) => write!(formatter, "graph node {node:?} is unavailable"),
            Self::UnknownParameter { node, parameter } => write!(
                formatter,
                "graph node {node:?} has no resource-selector parameter {parameter}"
            ),
            Self::ValuePath(error) => write!(
                formatter,
                "resource selector value path is invalid: {error}"
            ),
            Self::NotResourceHandleValue { node, parameter } => write!(
                formatter,
                "graph node {node:?} parameter {parameter} does not select a resource-handle leaf"
            ),
            Self::ParameterNotCatalogBound { node, parameter } => write!(
                formatter,
                "graph node {node:?} parameter {parameter} is not bound by this exact capability catalog"
            ),
            Self::AlreadySelected {
                node,
                parameter,
                entry,
            } => write!(
                formatter,
                "graph node {node:?} parameter {parameter} already carries capability entry {entry}"
            ),
            Self::ResourceAlreadySelected {
                entry,
                node,
                parameter,
            } => write!(
                formatter,
                "capability entry {entry} is already carried by graph node {node:?} parameter {parameter}"
            ),
            Self::InvalidEntry { entry, aspect } => {
                write!(
                    formatter,
                    "graph capability entry {entry} has invalid {aspect}"
                )
            }
            Self::Workspace(error) => write!(formatter, "resource selection failed: {error}"),
            Self::Analysis(error) => {
                write!(
                    formatter,
                    "resource selection semantic analysis failed: {error}"
                )
            }
        }
    }
}

impl std::error::Error for GraphCapabilityResourceSelectionError {}

impl From<GraphWorkspaceError> for GraphCapabilityResourceSelectionError {
    fn from(value: GraphWorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl From<GraphValuePathError> for GraphCapabilityResourceSelectionError {
    fn from(value: GraphValuePathError) -> Self {
        Self::ValuePath(value)
    }
}

impl From<GraphAnalysisError> for GraphCapabilityResourceSelectionError {
    fn from(value: GraphAnalysisError) -> Self {
        Self::Analysis(value)
    }
}

/// Failure while deriving a capability-bound editor palette.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphCapabilityCatalogError {
    /// A caller-owned limit was zero.
    ZeroLimit,
    /// The complete capability document exceeded caller policy.
    CapabilityDocumentTooLarge,
    /// Capability bytes did not expose a canonical graph-executor section.
    CapabilityDocument(CapabilityDocumentError),
    /// Device, capability, or configuration identity was the zero sentinel.
    MissingIdentity(&'static str),
    /// The authenticated target digest did not identify the supplied bytes.
    CapabilityIdentityMismatch {
        /// Digest expected by the target session.
        expected: Digest,
        /// Digest calculated over the complete supplied document.
        received: Digest,
    },
    /// Concrete target-bound entries exceeded caller policy.
    EntryLimitExceeded,
    /// A reviewed implementation contradicted its already validated schema.
    InvalidImplementation {
        /// Exact reviewed kind.
        kind: NodeKind,
        /// Contradictory fact.
        aspect: &'static str,
    },
    /// A concrete resource handle failed the graph value schema.
    ResourceValue(GraphSchemaError),
}

impl fmt::Display for GraphCapabilityCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph capability-catalog limit is zero"),
            Self::CapabilityDocumentTooLarge => {
                formatter.write_str("capability document exceeds catalog policy")
            }
            Self::CapabilityDocument(error) => {
                write!(formatter, "capability graph section is invalid: {error:?}")
            }
            Self::MissingIdentity(identity) => {
                write!(formatter, "catalog target {identity} is missing")
            }
            Self::CapabilityIdentityMismatch { .. } => formatter.write_str(
                "catalog target capability digest does not identify the supplied document",
            ),
            Self::EntryLimitExceeded => {
                formatter.write_str("capability-derived node entries exceed catalog policy")
            }
            Self::InvalidImplementation { kind, aspect } => write!(
                formatter,
                "reviewed catalog kind {} v{} has invalid {aspect}",
                kind.name(),
                kind.version()
            ),
            Self::ResourceValue(error) => {
                write!(formatter, "capability resource handle is invalid: {error}")
            }
        }
    }
}

impl std::error::Error for GraphCapabilityCatalogError {}

impl From<GraphSchemaError> for GraphCapabilityCatalogError {
    fn from(value: GraphSchemaError) -> Self {
        Self::ResourceValue(value)
    }
}

/// Intersect caller-authenticated resources with reviewed deployment bindings.
///
/// V1 materializes the one physical operation currently implemented by the
/// fixed firmware executor: fresh debounced Boolean input. Resource-free node
/// kinds remain in their normal audited palette and future physical operations
/// must add an explicit access enum, opcode behavior, and derivation branch.
pub fn derive_graph_capability_node_catalog(
    capability_document: &[u8],
    target: GraphDeploymentTarget,
    registry: &GraphDeploymentRegistry,
    limits: GraphCapabilityCatalogLimits,
) -> Result<GraphCapabilityNodeCatalog, GraphCapabilityCatalogError> {
    limits.validate()?;
    if capability_document.len() > limits.maximum_capability_bytes {
        return Err(GraphCapabilityCatalogError::CapabilityDocumentTooLarge);
    }
    validate_target(target)?;
    let capability = decode_graph_execution(capability_document)
        .map_err(GraphCapabilityCatalogError::CapabilityDocument)?;
    if capability.identity().digest != target.capability_digest {
        return Err(GraphCapabilityCatalogError::CapabilityIdentityMismatch {
            expected: target.capability_digest,
            received: capability.identity().digest,
        });
    }

    let opcodes = capability.opcodes().collect::<Vec<_>>();
    let resources = capability.resources().collect::<Vec<_>>();
    let mut entries = Vec::new();
    for implementation in registry.implementations() {
        let GraphDeploymentNodeKind::StableBooleanInput {
            output: _,
            resource_parameter,
        } = implementation.behavior()
        else {
            continue;
        };
        let schema = registry
            .semantic_registry()
            .schema(implementation.kind())
            .ok_or_else(|| GraphCapabilityCatalogError::InvalidImplementation {
                kind: implementation.kind().clone(),
                aspect: "semantic schema",
            })?;
        let parameter = schema
            .parameters()
            .iter()
            .find(|candidate| candidate.id() == resource_parameter)
            .ok_or_else(|| GraphCapabilityCatalogError::InvalidImplementation {
                kind: implementation.kind().clone(),
                aspect: "resource parameter",
            })?;
        let Some(TypeKind::ResourceHandle { class }) = registry
            .semantic_registry()
            .context_schema()
            .value_type(parameter.value_type())
            .map(super::TypeDefinition::kind)
        else {
            return Err(GraphCapabilityCatalogError::InvalidImplementation {
                kind: implementation.kind().clone(),
                aspect: "resource parameter type",
            });
        };
        let class = alumina_board::GraphResourceClass::new(class.get());
        let opcode_available = opcodes.iter().any(|opcode| {
            opcode.opcode == GraphIrOpcode::StableBooleanInput.wire_value()
                && opcode.domain == OwnerDomain::Realtime
                && opcode.support >= SupportLevel::Compiles
                && opcode.resource_class == Some(class)
                && opcode.resource_access == Some(GraphResourceAccess::StableBooleanInput)
        });
        if !opcode_available {
            continue;
        }

        for resource in resources.iter().copied().filter(|resource| {
            resource.class == class
                && resource.access == GraphResourceAccess::StableBooleanInput
                && resource.support >= SupportLevel::Compiles
        }) {
            if entries.len() == limits.maximum_entries {
                return Err(GraphCapabilityCatalogError::EntryLimitExceeded);
            }
            let value = TypedGraphValue::try_new(
                registry.semantic_registry().context_schema(),
                parameter.value_type(),
                GraphValue::ResourceHandle(ResourceGraphHandle {
                    device_id: target.device_id,
                    board_package_digest: target.capability_digest,
                    class: super::ResourceClassId::new(class.get()),
                    resource_selector: u32::from_le_bytes(encode_resource_id(resource.resource)),
                }),
            )?;
            let label = format!("{} stable input", graph_resource_label(resource.resource));
            let prototype = GraphNodePrototype::new(
                implementation.kind().clone(),
                label,
                ExecutionDomain::Realtime {
                    device_id: target.device_id,
                },
                schema.inputs().to_vec(),
                schema.outputs().to_vec(),
                vec![NodeParameter::new(
                    resource_parameter,
                    parameter.name(),
                    value,
                )],
            );
            entries.push(GraphCapabilityNodeEntry {
                resource,
                kind: implementation.kind().clone(),
                resource_parameter,
                prototype,
            });
        }
    }
    entries.sort_unstable_by(|left, right| {
        left.kind
            .name()
            .cmp(right.kind.name())
            .then_with(|| left.kind.version().cmp(&right.kind.version()))
            .then_with(|| {
                encode_resource_id(left.resource.resource)
                    .cmp(&encode_resource_id(right.resource.resource))
            })
    });

    Ok(GraphCapabilityNodeCatalog {
        identity: capability.identity(),
        target,
        advertised_resource_count: resources.len(),
        entries,
    })
}

/// Transactionally select a different capability-derived resource at one
/// exact registered value path.
///
/// The existing leaf must already resolve to an exact entry in `catalog`; a
/// merely well-typed but unlisted handle, label, or numeric pin therefore
/// cannot gain selector authority. The replacement must retain the registered
/// resource-handle type and may not duplicate an identity held by any other
/// root or composite parameter leaf. The complete reconstructed value and
/// candidate graph are structurally validated, semantically analyzed against
/// `registry`, and canonically encoded before `workspace` changes.
pub fn select_graph_capability_node_resource(
    catalog: &GraphCapabilityNodeCatalog,
    registry: &GraphDeploymentRegistry,
    workspace: &mut GraphWorkspaceDocument,
    node_id: GraphNodeId,
    parameter_id: u32,
    path: &[GraphValuePathSegment],
    entry_index: usize,
) -> Result<CanonicalGraphWorkspaceEncoding, GraphCapabilityResourceSelectionError> {
    let entry = catalog.entries().get(entry_index).ok_or(
        GraphCapabilityResourceSelectionError::UnknownEntry(entry_index),
    )?;
    let selected_parameter =
        entry
            .resource_parameter()
            .ok_or(GraphCapabilityResourceSelectionError::InvalidEntry {
                entry: entry_index,
                aspect: "resource parameter",
            })?;
    let GraphValue::ResourceHandle(selected_handle) = selected_parameter.value().value() else {
        return Err(GraphCapabilityResourceSelectionError::InvalidEntry {
            entry: entry_index,
            aspect: "resource value",
        });
    };
    let node = workspace
        .graph()
        .node(node_id)
        .ok_or(GraphCapabilityResourceSelectionError::UnknownNode(node_id))?;
    let parameter = node
        .parameters()
        .iter()
        .find(|parameter| parameter.id() == parameter_id)
        .ok_or(GraphCapabilityResourceSelectionError::UnknownParameter {
            node: node_id,
            parameter: parameter_id,
        })?;
    let (leaf_type, leaf) = parameter
        .value()
        .value_at_path(workspace.graph().schema(), path)?;
    let Some(TypeKind::ResourceHandle { .. }) = workspace
        .graph()
        .schema()
        .value_type(leaf_type)
        .map(super::TypeDefinition::kind)
    else {
        return Err(
            GraphCapabilityResourceSelectionError::NotResourceHandleValue {
                node: node_id,
                parameter: parameter_id,
            },
        );
    };
    if leaf_type != selected_parameter.value().value_type() {
        return Err(
            GraphCapabilityResourceSelectionError::ParameterNotCatalogBound {
                node: node_id,
                parameter: parameter_id,
            },
        );
    }
    let GraphValue::ResourceHandle(current_handle) = leaf else {
        return Err(
            GraphCapabilityResourceSelectionError::NotResourceHandleValue {
                node: node_id,
                parameter: parameter_id,
            },
        );
    };
    if catalog.entry_index_for_handle(*current_handle).is_none() {
        return Err(
            GraphCapabilityResourceSelectionError::ParameterNotCatalogBound {
                node: node_id,
                parameter: parameter_id,
            },
        );
    }
    if current_handle == selected_handle {
        return Err(GraphCapabilityResourceSelectionError::AlreadySelected {
            node: node_id,
            parameter: parameter_id,
            entry: entry_index,
        });
    }
    if let Some((existing_node, existing_parameter)) =
        workspace.graph().nodes().iter().find_map(|candidate| {
            candidate
                .parameters()
                .iter()
                .find_map(|candidate_parameter| {
                    graph_value_contains_resource_handle(
                        candidate_parameter.value().value(),
                        *selected_handle,
                    )
                    .then_some((candidate.id(), candidate_parameter.id()))
                })
        })
    {
        return Err(
            GraphCapabilityResourceSelectionError::ResourceAlreadySelected {
                entry: entry_index,
                node: existing_node,
                parameter: existing_parameter,
            },
        );
    }

    let selected = parameter.value().replacing_value_at_path(
        workspace.graph().schema(),
        path,
        GraphValue::ResourceHandle(*selected_handle),
    )?;
    let mut candidate = workspace.clone();
    candidate.set_parameter(node_id, parameter_id, selected)?;
    analyze_graph_draft(candidate.graph(), registry.semantic_registry())?;
    let encoding = encode_graph_workspace(&candidate)?;
    *workspace = candidate;
    Ok(encoding)
}

fn graph_value_contains_resource_handle(value: &GraphValue, selected: ResourceGraphHandle) -> bool {
    match value {
        GraphValue::Array(values) => values
            .iter()
            .any(|value| graph_value_contains_resource_handle(value, selected)),
        GraphValue::Record(fields) => fields
            .iter()
            .any(|field| graph_value_contains_resource_handle(&field.value, selected)),
        GraphValue::OptionSome(value)
        | GraphValue::ResultOk(value)
        | GraphValue::ResultError(value) => graph_value_contains_resource_handle(value, selected),
        GraphValue::ResourceHandle(handle) => *handle == selected,
        GraphValue::Boolean(_)
        | GraphValue::ExactRational(_)
        | GraphValue::MeasurementInterval { .. }
        | GraphValue::CanonicalI64(_)
        | GraphValue::CanonicalU64(_)
        | GraphValue::Text(_)
        | GraphValue::Bytes(_)
        | GraphValue::OptionNone
        | GraphValue::JobHandle(_) => false,
    }
}

fn node_matches_entry(node: &NodeDefinition, entry: &GraphCapabilityNodeEntry) -> bool {
    let Some(expected) = entry.resource_parameter() else {
        return false;
    };
    node.kind() == entry.kind()
        && node.domain() == entry.prototype().domain()
        && node.inputs() == entry.prototype().inputs()
        && node.outputs() == entry.prototype().outputs()
        && node.parameters().iter().any(|parameter| {
            parameter.id() == expected.id()
                && parameter.name() == expected.name()
                && parameter.value() == expected.value()
        })
}

/// Stable, allocation-backed display label for one typed board resource.
pub fn graph_resource_label(resource: ResourceId) -> String {
    match resource {
        ResourceId::Gpio(index) => format!("GPIO {index}"),
        ResourceId::I2sOut { engine, bit } => format!("I2S {engine} output bit {bit}"),
        ResourceId::Adc { unit, channel } => format!("ADC {unit} channel {channel}"),
        ResourceId::Timer { group, index } => format!("timer {group}:{index}"),
        ResourceId::I2s(index) => format!("I2S {index}"),
        ResourceId::Rmt(index) => format!("RMT {index}"),
        ResourceId::TimedOutput { engine, channel } => {
            format!("timed output {engine}:{channel}")
        }
        ResourceId::I2c(index) => format!("I2C {index}"),
        ResourceId::Spi(index) => format!("SPI {index}"),
        ResourceId::Uart(index) => format!("UART {index}"),
        ResourceId::Pcnt(index) => format!("PCNT {index}"),
        ResourceId::Dma(index) => format!("DMA {index}"),
        ResourceId::Twai(index) => format!("TWAI {index}"),
        ResourceId::Storage(index) => format!("storage {index}"),
        ResourceId::Radio(index) => format!("radio {index}"),
        ResourceId::SafetyInput(index) => format!("safety input {index}"),
        ResourceId::Device(index) => format!("board device {index}"),
    }
}

fn validate_target(target: GraphDeploymentTarget) -> Result<(), GraphCapabilityCatalogError> {
    if target.device_id.0.iter().all(|byte| *byte == 0) {
        return Err(GraphCapabilityCatalogError::MissingIdentity(
            "device identity",
        ));
    }
    if target.capability_digest.is_zero() {
        return Err(GraphCapabilityCatalogError::MissingIdentity(
            "capability digest",
        ));
    }
    if target.config_digest.is_zero() {
        return Err(GraphCapabilityCatalogError::MissingIdentity(
            "configuration digest",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alumina_board::ResourceId;
    use alumina_capability::{MAX_CAPABILITY_CHUNK_BYTES, calculate_identity, read_verified_range};
    use alumina_protocol::{DeviceId, Digest};

    use super::*;
    use crate::graph::{
        ClockDefinition, ClockKind, ExecutionDomainSet, GraphAnalysisLimits, GraphClockId,
        GraphDeploymentImplementation, GraphDocument, GraphLimits, GraphNodeRegistry, GraphPortId,
        GraphSchema, GraphTypeId, NodeOutputDependency, NodeParameterContract, NodeSchema,
        PortDefinition, RecordField, RecordFieldId, RecordValueField, ResourceClassId,
        TypeDefinition,
    };

    const DEVICE: DeviceId = DeviceId([0x54; 16]);
    const BOOL: GraphTypeId = GraphTypeId::new(1);
    const STREAM: GraphTypeId = GraphTypeId::new(2);
    const RESOURCE: GraphTypeId = GraphTypeId::new(3);
    const RESOURCE_OPTION: GraphTypeId = GraphTypeId::new(4);
    const RESOURCE_ARRAY: GraphTypeId = GraphTypeId::new(5);
    const RESOURCE_SET: GraphTypeId = GraphTypeId::new(6);
    const ROOT: GraphClockId = GraphClockId::new(1);
    const SAMPLE: GraphClockId = GraphClockId::new(2);
    const CLASS: ResourceClassId = ResourceClassId::new(1);
    const PRIMARY: RecordFieldId = RecordFieldId::new(1);
    const FALLBACK: RecordFieldId = RecordFieldId::new(2);
    const MIRRORS: RecordFieldId = RecordFieldId::new(3);
    const PRIMARY_PATH: [GraphValuePathSegment; 1] = [GraphValuePathSegment::RecordField(PRIMARY)];
    const FALLBACK_PATH: [GraphValuePathSegment; 2] = [
        GraphValuePathSegment::RecordField(FALLBACK),
        GraphValuePathSegment::OptionSome,
    ];
    const MIRROR_PATH: [GraphValuePathSegment; 2] = [
        GraphValuePathSegment::RecordField(MIRRORS),
        GraphValuePathSegment::ArrayIndex(0),
    ];

    fn capability_document() -> Vec<u8> {
        let package = &board_mks_tinybee::PACKAGE;
        let identity = calculate_identity(package).unwrap();
        let mut document = vec![0_u8; usize::try_from(identity.byte_len).unwrap()];
        let mut offset = 0_u32;
        while offset < identity.byte_len {
            let mut chunk = [0_u8; MAX_CAPABILITY_CHUNK_BYTES];
            let read = read_verified_range(package, offset, &mut chunk).unwrap();
            let start = usize::try_from(offset).unwrap();
            let count = usize::from(read.byte_len);
            document[start..start + count].copy_from_slice(&chunk[..count]);
            offset += u32::from(read.byte_len);
        }
        document
    }

    fn port(id: u32, name: &str, value_type: GraphTypeId) -> PortDefinition {
        PortDefinition::new(GraphPortId::new(id), name, value_type)
    }

    fn registry() -> GraphDeploymentRegistry {
        let schema = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![
                TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    STREAM,
                    "stream.input.bool",
                    TypeKind::Stream {
                        sample: BOOL,
                        clock: SAMPLE,
                        capacity: 1,
                    },
                ),
                TypeDefinition::new(
                    RESOURCE,
                    "resource.stable-bool",
                    TypeKind::ResourceHandle { class: CLASS },
                ),
                TypeDefinition::new(
                    RESOURCE_OPTION,
                    "resource.optional-stable-bool",
                    TypeKind::Option { value: RESOURCE },
                ),
                TypeDefinition::new(
                    RESOURCE_ARRAY,
                    "resource.stable-bool-array",
                    TypeKind::Array {
                        element: RESOURCE,
                        maximum_items: 4,
                    },
                ),
                TypeDefinition::new(
                    RESOURCE_SET,
                    "resource.stable-bool-reference-set",
                    TypeKind::Record {
                        fields: vec![
                            RecordField::new(PRIMARY, "primary", RESOURCE),
                            RecordField::new(FALLBACK, "fallback", RESOURCE_OPTION),
                            RecordField::new(MIRRORS, "mirrors", RESOURCE_ARRAY),
                        ],
                    },
                ),
            ],
        )
        .unwrap();
        let clocks = vec![
            ClockDefinition::new(
                ROOT,
                "tinybee.cpu",
                ClockKind::DeviceCycle {
                    device_id: DEVICE,
                    ticks_per_second: 240_000_000,
                },
            ),
            ClockDefinition::new(
                SAMPLE,
                "tinybee.input.1khz",
                ClockKind::Derived {
                    source: ROOT,
                    numerator: 1,
                    denominator: 240_000,
                },
            ),
        ];
        let context = GraphDocument::try_new(0, schema, clocks, Vec::new(), Vec::new()).unwrap();
        let input_kind = NodeKind::new("alumina.io.stable-boolean-input", 1);
        let input_schema = NodeSchema::new(
            input_kind.clone(),
            ExecutionDomainSet::REALTIME,
            Vec::new(),
            Vec::new(),
            vec![port(1, "samples", STREAM)],
            vec![NodeParameterContract::new(1, "resource", RESOURCE)],
            vec![NodeOutputDependency::new(GraphPortId::new(1), Vec::new())],
            Vec::new(),
            None,
        );
        let reference_schema = NodeSchema::new(
            NodeKind::new("alumina.io.capability-reference-set", 1),
            ExecutionDomainSet::HOST_EXACT,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![NodeParameterContract::new(1, "references", RESOURCE_SET)],
            Vec::new(),
            Vec::new(),
            None,
        );
        let semantic = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &context,
            vec![input_schema, reference_schema],
        )
        .unwrap();
        GraphDeploymentRegistry::try_new(
            semantic,
            vec![GraphDeploymentImplementation::new(
                input_kind,
                GraphDeploymentNodeKind::StableBooleanInput {
                    output: GraphPortId::new(1),
                    resource_parameter: 1,
                },
                SAMPLE,
                100,
            )],
        )
        .unwrap()
    }

    fn target() -> GraphDeploymentTarget {
        GraphDeploymentTarget {
            device_id: DEVICE,
            capability_digest: board_mks_tinybee::PACKAGE.board.capability_digest,
            config_digest: Digest([0x43; 32]),
        }
    }

    fn empty_workspace(registry: &GraphDeploymentRegistry) -> GraphWorkspaceDocument {
        let context = GraphDocument::try_new(
            0,
            registry.semantic_registry().context_schema().clone(),
            registry.semantic_registry().context_clocks().to_vec(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        GraphWorkspaceDocument::try_new(
            super::super::GraphWorkspaceLimits::interactive(),
            0,
            1,
            1,
            context,
            Vec::new(),
        )
        .unwrap()
    }

    fn catalog_handle(catalog: &GraphCapabilityNodeCatalog, entry: usize) -> ResourceGraphHandle {
        let GraphValue::ResourceHandle(handle) = catalog.entries()[entry]
            .resource_parameter()
            .unwrap()
            .value()
            .value()
        else {
            panic!("catalog parameter was not a resource handle")
        };
        *handle
    }

    fn reference_set_prototype(
        catalog: &GraphCapabilityNodeCatalog,
        registry: &GraphDeploymentRegistry,
    ) -> GraphNodePrototype {
        let value = TypedGraphValue::try_new(
            registry.semantic_registry().context_schema(),
            RESOURCE_SET,
            GraphValue::Record(vec![
                RecordValueField {
                    field: PRIMARY,
                    value: GraphValue::ResourceHandle(catalog_handle(catalog, 0)),
                },
                RecordValueField {
                    field: FALLBACK,
                    value: GraphValue::OptionSome(Box::new(GraphValue::ResourceHandle(
                        catalog_handle(catalog, 1),
                    ))),
                },
                RecordValueField {
                    field: MIRRORS,
                    value: GraphValue::Array(vec![GraphValue::ResourceHandle(catalog_handle(
                        catalog, 2,
                    ))]),
                },
            ]),
        )
        .unwrap();
        GraphNodePrototype::new(
            NodeKind::new("alumina.io.capability-reference-set", 1),
            "Capability resource references",
            ExecutionDomain::HostExact,
            Vec::new(),
            Vec::new(),
            vec![NodeParameter::new(1, "references", value)],
        )
    }

    #[test]
    fn tinybee_catalog_materializes_only_authenticated_reviewed_resources() {
        let registry = registry();
        let catalog = derive_graph_capability_node_catalog(
            &capability_document(),
            target(),
            &registry,
            GraphCapabilityCatalogLimits::interactive(),
        )
        .unwrap();
        assert_eq!(catalog.advertised_resource_count(), 4);
        assert_eq!(catalog.entries().len(), 4);
        assert_eq!(
            catalog
                .entries()
                .iter()
                .map(|entry| entry.resource().resource)
                .collect::<Vec<_>>(),
            vec![
                ResourceId::Gpio(22),
                ResourceId::Gpio(32),
                ResourceId::Gpio(33),
                ResourceId::Gpio(35),
            ]
        );
        for entry in catalog.entries() {
            assert_eq!(
                entry.kind(),
                &NodeKind::new("alumina.io.stable-boolean-input", 1)
            );
            assert_eq!(
                entry.prototype().domain(),
                ExecutionDomain::Realtime { device_id: DEVICE }
            );
            let GraphValue::ResourceHandle(handle) =
                entry.prototype().parameters()[0].value().value()
            else {
                panic!("catalog parameter was not a resource handle")
            };
            assert_eq!(handle.device_id, DEVICE);
            assert_eq!(
                handle.board_package_digest,
                board_mks_tinybee::PACKAGE.board.capability_digest
            );
            assert_eq!(handle.class, CLASS);
            assert_eq!(
                alumina_capability::decode_resource_id(&handle.resource_selector.to_le_bytes()),
                Ok(entry.resource().resource)
            );
        }
    }

    #[test]
    fn catalog_entries_insert_transactionally_into_matching_context() {
        let registry = registry();
        let catalog = derive_graph_capability_node_catalog(
            &capability_document(),
            target(),
            &registry,
            GraphCapabilityCatalogLimits::interactive(),
        )
        .unwrap();
        let mut workspace = empty_workspace(&registry);
        for (index, entry) in catalog.entries().iter().enumerate() {
            workspace
                .create_node(entry.instantiate(), i32::try_from(index).unwrap() * 240, 0)
                .unwrap();
        }
        assert_eq!(workspace.graph().nodes().len(), 4);
        let draft =
            super::super::analyze_graph_draft(workspace.graph(), registry.semantic_registry())
                .unwrap();
        assert!(draft.required_unconnected_inputs().is_empty());
    }

    #[test]
    fn catalog_resource_selection_preserves_node_identity_and_exact_authority() {
        let registry = registry();
        let catalog = derive_graph_capability_node_catalog(
            &capability_document(),
            target(),
            &registry,
            GraphCapabilityCatalogLimits::interactive(),
        )
        .unwrap();
        let mut workspace = empty_workspace(&registry);
        let node = workspace
            .create_node(catalog.entries()[0].instantiate(), 17, -23)
            .unwrap();
        let before = encode_graph_workspace(&workspace).unwrap();
        let before_revision = workspace.revision();
        let before_graph_revision = workspace.graph().revision();
        let before_node_cursor = workspace.next_node_id();
        let before_wire_cursor = workspace.next_wire_id();
        let before_placement = workspace.placement(node).unwrap();

        let selected = select_graph_capability_node_resource(
            &catalog,
            &registry,
            &mut workspace,
            node,
            1,
            &[],
            2,
        )
        .unwrap();
        assert_ne!(selected, before);
        assert_eq!(selected, encode_graph_workspace(&workspace).unwrap());
        assert_eq!(workspace.revision(), before_revision + 1);
        assert_eq!(workspace.graph().revision(), before_graph_revision + 1);
        assert_eq!(workspace.next_node_id(), before_node_cursor);
        assert_eq!(workspace.next_wire_id(), before_wire_cursor);
        assert_eq!(workspace.placement(node), Some(before_placement));
        assert_eq!(
            catalog.entry_index_for_node(workspace.graph().node(node).unwrap()),
            Some(2)
        );
        let GraphValue::ResourceHandle(handle) = workspace.graph().node(node).unwrap().parameters()
            [0]
        .value()
        .value() else {
            panic!("selected catalog parameter was not a resource handle")
        };
        assert_eq!(handle.device_id, target().device_id);
        assert_eq!(handle.board_package_digest, target().capability_digest);
        assert_eq!(handle.class, CLASS);
        assert_eq!(
            alumina_capability::decode_resource_id(&handle.resource_selector.to_le_bytes()),
            Ok(ResourceId::Gpio(33))
        );

        let retained = selected;
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                1,
                &[],
                2,
            ),
            Err(GraphCapabilityResourceSelectionError::AlreadySelected {
                node,
                parameter: 1,
                entry: 2,
            })
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);
    }

    #[test]
    fn catalog_resource_selection_rebinds_only_one_composite_leaf() {
        let registry = registry();
        let catalog = derive_graph_capability_node_catalog(
            &capability_document(),
            target(),
            &registry,
            GraphCapabilityCatalogLimits::interactive(),
        )
        .unwrap();
        let mut workspace = empty_workspace(&registry);
        let node = workspace
            .create_node(reference_set_prototype(&catalog, &registry), 17, -23)
            .unwrap();
        let before = encode_graph_workspace(&workspace).unwrap();
        let before_node_cursor = workspace.next_node_id();
        let before_wire_cursor = workspace.next_wire_id();
        let before_placement = workspace.placement(node);

        let selected = select_graph_capability_node_resource(
            &catalog,
            &registry,
            &mut workspace,
            node,
            1,
            &FALLBACK_PATH,
            3,
        )
        .unwrap();
        assert_ne!(selected, before);
        assert_eq!(selected, encode_graph_workspace(&workspace).unwrap());
        assert_eq!(workspace.next_node_id(), before_node_cursor);
        assert_eq!(workspace.next_wire_id(), before_wire_cursor);
        assert_eq!(workspace.placement(node), before_placement);
        let parameter = &workspace.graph().node(node).unwrap().parameters()[0];
        for (path, entry) in [
            (&PRIMARY_PATH[..], 0),
            (&FALLBACK_PATH[..], 3),
            (&MIRROR_PATH[..], 2),
        ] {
            let (_, GraphValue::ResourceHandle(handle)) = parameter
                .value()
                .value_at_path(workspace.graph().schema(), path)
                .unwrap()
            else {
                panic!("composite path did not resolve to a resource handle")
            };
            assert_eq!(catalog.entry_index_for_handle(*handle), Some(entry));
        }

        let retained = selected;
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                1,
                &MIRROR_PATH,
                0,
            ),
            Err(
                GraphCapabilityResourceSelectionError::ResourceAlreadySelected {
                    entry: 0,
                    node,
                    parameter: 1,
                }
            )
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                1,
                &FALLBACK_PATH,
                3,
            ),
            Err(GraphCapabilityResourceSelectionError::AlreadySelected {
                node,
                parameter: 1,
                entry: 3,
            })
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);

        let invalid_path = [GraphValuePathSegment::OptionSome];
        assert!(matches!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                1,
                &invalid_path,
                1,
            ),
            Err(GraphCapabilityResourceSelectionError::ValuePath(
                GraphValuePathError::InvalidSegment { .. }
            ))
        ));
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                1,
                &[],
                1,
            ),
            Err(
                GraphCapabilityResourceSelectionError::NotResourceHandleValue {
                    node,
                    parameter: 1,
                }
            )
        );
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                77,
                &[],
                1,
            ),
            Err(GraphCapabilityResourceSelectionError::UnknownParameter {
                node,
                parameter: 77,
            })
        );
        let out_of_bounds = [
            GraphValuePathSegment::RecordField(MIRRORS),
            GraphValuePathSegment::ArrayIndex(9),
        ];
        assert!(matches!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                1,
                &out_of_bounds,
                1,
            ),
            Err(GraphCapabilityResourceSelectionError::ValuePath(
                GraphValuePathError::ArrayIndexOutOfBounds { .. }
            ))
        ));
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);
    }

    #[test]
    fn composite_resource_selection_rejects_inactive_and_raw_leaves_atomically() {
        let registry = registry();
        let catalog = derive_graph_capability_node_catalog(
            &capability_document(),
            target(),
            &registry,
            GraphCapabilityCatalogLimits::interactive(),
        )
        .unwrap();
        let mut workspace = empty_workspace(&registry);
        let node = workspace
            .create_node(reference_set_prototype(&catalog, &registry), 0, 0)
            .unwrap();
        let parameter = workspace.graph().node(node).unwrap().parameters()[0].clone();
        let GraphValue::Record(mut fields) = parameter.value().value().clone() else {
            panic!("reference set was not a record")
        };
        fields
            .iter_mut()
            .find(|field| field.field == FALLBACK)
            .unwrap()
            .value = GraphValue::OptionNone;
        let inactive = TypedGraphValue::try_new(
            workspace.graph().schema(),
            RESOURCE_SET,
            GraphValue::Record(fields),
        )
        .unwrap();
        workspace.set_parameter(node, 1, inactive).unwrap();
        let retained = encode_graph_workspace(&workspace).unwrap();
        assert!(matches!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                1,
                &FALLBACK_PATH,
                3,
            ),
            Err(GraphCapabilityResourceSelectionError::ValuePath(
                GraphValuePathError::InactiveBranch { .. }
            ))
        ));
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);

        let mut raw_handle = catalog_handle(&catalog, 0);
        raw_handle.resource_selector = u32::from_le_bytes(encode_resource_id(ResourceId::Gpio(2)));
        let raw = workspace.graph().node(node).unwrap().parameters()[0]
            .value()
            .replacing_value_at_path(
                workspace.graph().schema(),
                &PRIMARY_PATH,
                GraphValue::ResourceHandle(raw_handle),
            )
            .unwrap();
        workspace.set_parameter(node, 1, raw).unwrap();
        let retained = encode_graph_workspace(&workspace).unwrap();
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                1,
                &PRIMARY_PATH,
                3,
            ),
            Err(
                GraphCapabilityResourceSelectionError::ParameterNotCatalogBound {
                    node,
                    parameter: 1,
                }
            )
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);
    }

    #[test]
    fn catalog_resource_selection_rejects_duplicates_and_raw_or_foreign_handles() {
        let registry = registry();
        let document = capability_document();
        let catalog = derive_graph_capability_node_catalog(
            &document,
            target(),
            &registry,
            GraphCapabilityCatalogLimits::interactive(),
        )
        .unwrap();
        let mut workspace = empty_workspace(&registry);
        let first = workspace
            .create_node(catalog.entries()[0].instantiate(), 0, 0)
            .unwrap();
        let second = workspace
            .create_node(catalog.entries()[1].instantiate(), 200, 0)
            .unwrap();
        let retained = encode_graph_workspace(&workspace).unwrap();
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                first,
                1,
                &[],
                1,
            ),
            Err(
                GraphCapabilityResourceSelectionError::ResourceAlreadySelected {
                    entry: 1,
                    node: second,
                    parameter: 1,
                }
            )
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                GraphNodeId::new(77),
                1,
                &[],
                0,
            ),
            Err(GraphCapabilityResourceSelectionError::UnknownNode(
                GraphNodeId::new(77)
            ))
        );
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                first,
                1,
                &[],
                catalog.entries().len(),
            ),
            Err(GraphCapabilityResourceSelectionError::UnknownEntry(
                catalog.entries().len()
            ))
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);

        let expected = catalog.entries()[0].resource_parameter().unwrap();
        let GraphValue::ResourceHandle(mut raw_handle) = expected.value().value().clone() else {
            panic!("catalog parameter was not a resource handle")
        };
        raw_handle.resource_selector = u32::from_le_bytes(encode_resource_id(ResourceId::Gpio(2)));
        let raw_value = TypedGraphValue::try_new(
            workspace.graph().schema(),
            expected.value().value_type(),
            GraphValue::ResourceHandle(raw_handle),
        )
        .unwrap();
        workspace
            .set_parameter(first, expected.id(), raw_value)
            .unwrap();
        let raw = encode_graph_workspace(&workspace).unwrap();
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                first,
                1,
                &[],
                2,
            ),
            Err(
                GraphCapabilityResourceSelectionError::ParameterNotCatalogBound {
                    node: first,
                    parameter: 1,
                }
            )
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), raw);

        let mut foreign_target = target();
        foreign_target.device_id = DeviceId([0x55; 16]);
        let foreign_catalog = derive_graph_capability_node_catalog(
            &document,
            foreign_target,
            &registry,
            GraphCapabilityCatalogLimits::interactive(),
        )
        .unwrap();
        assert_eq!(
            select_graph_capability_node_resource(
                &foreign_catalog,
                &registry,
                &mut workspace,
                second,
                1,
                &[],
                2,
            ),
            Err(
                GraphCapabilityResourceSelectionError::ParameterNotCatalogBound {
                    node: second,
                    parameter: 1,
                }
            )
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), raw);
    }

    #[test]
    fn catalog_resource_selection_rejects_semantic_mismatch_atomically() {
        let registry = registry();
        let catalog = derive_graph_capability_node_catalog(
            &capability_document(),
            target(),
            &registry,
            GraphCapabilityCatalogLimits::interactive(),
        )
        .unwrap();
        let mut workspace = empty_workspace(&registry);
        let node = workspace
            .create_node(catalog.entries()[0].instantiate(), 0, 0)
            .unwrap();
        let retained = encode_graph_workspace(&workspace).unwrap();

        let empty_context = GraphDocument::try_new(
            0,
            registry.semantic_registry().context_schema().clone(),
            registry.semantic_registry().context_clocks().to_vec(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let unreviewed_semantics = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &empty_context,
            Vec::new(),
        )
        .unwrap();
        let unreviewed_registry =
            GraphDeploymentRegistry::try_new(unreviewed_semantics, Vec::new()).unwrap();
        assert!(matches!(
            select_graph_capability_node_resource(
                &catalog,
                &unreviewed_registry,
                &mut workspace,
                node,
                1,
                &[],
                1,
            ),
            Err(GraphCapabilityResourceSelectionError::Analysis(
                GraphAnalysisError::UnresolvedNode { node: rejected, .. }
            )) if rejected == node
        ));
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);
    }

    #[test]
    fn catalog_fails_closed_on_identity_and_bounds() {
        let document = capability_document();
        let registry = registry();
        let mut wrong = target();
        wrong.capability_digest = Digest([0xa5; 32]);
        assert!(matches!(
            derive_graph_capability_node_catalog(
                &document,
                wrong,
                &registry,
                GraphCapabilityCatalogLimits::interactive(),
            ),
            Err(GraphCapabilityCatalogError::CapabilityIdentityMismatch { .. })
        ));
        assert_eq!(
            derive_graph_capability_node_catalog(
                &document,
                target(),
                &registry,
                GraphCapabilityCatalogLimits {
                    maximum_capability_bytes: document.len() - 1,
                    maximum_entries: 4,
                },
            ),
            Err(GraphCapabilityCatalogError::CapabilityDocumentTooLarge)
        );
        assert_eq!(
            derive_graph_capability_node_catalog(
                &document,
                target(),
                &registry,
                GraphCapabilityCatalogLimits {
                    maximum_capability_bytes: document.len(),
                    maximum_entries: 3,
                },
            ),
            Err(GraphCapabilityCatalogError::EntryLimitExceeded)
        );
    }

    #[test]
    fn labels_cover_typed_resource_namespaces_without_aliasing() {
        assert_eq!(graph_resource_label(ResourceId::Gpio(2)), "GPIO 2");
        assert_eq!(
            graph_resource_label(ResourceId::Timer { group: 1, index: 0 }),
            "timer 1:0"
        );
        assert_ne!(
            graph_resource_label(ResourceId::Gpio(0)),
            graph_resource_label(ResourceId::Uart(0))
        );
        assert_eq!(CLASS.get(), 1);
    }
}
