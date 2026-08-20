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
    GraphSchemaError, GraphValue, GraphWorkspaceDocument, GraphWorkspaceError, NodeDefinition,
    NodeKind, NodeParameter, ResourceGraphHandle, TypeKind, TypedGraphValue, analyze_graph_draft,
    encode_graph_workspace,
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
    /// The existing node did not carry an exact value offered by this catalog.
    NodeNotCatalogBound(GraphNodeId),
    /// The selected entry belongs to a different reviewed node behavior.
    NodeKindMismatch(GraphNodeId),
    /// The node already carries the requested exact catalog entry.
    AlreadySelected {
        /// Exact graph node.
        node: GraphNodeId,
        /// Exact catalog entry.
        entry: usize,
    },
    /// Another graph node already carries the requested resource identity.
    ResourceAlreadySelected {
        /// Exact catalog entry requested by the caller.
        entry: usize,
        /// Existing graph node that retains the identity.
        node: GraphNodeId,
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
            Self::NodeNotCatalogBound(node) => write!(
                formatter,
                "graph node {node:?} is not bound by this exact capability catalog"
            ),
            Self::NodeKindMismatch(node) => write!(
                formatter,
                "graph node {node:?} cannot change reviewed kind through a resource selector"
            ),
            Self::AlreadySelected { node, entry } => write!(
                formatter,
                "graph node {node:?} already carries capability entry {entry}"
            ),
            Self::ResourceAlreadySelected { entry, node } => write!(
                formatter,
                "capability entry {entry} is already carried by graph node {node:?}"
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

/// Transactionally select a different capability-derived resource for one
/// existing catalog-managed node.
///
/// The existing node must already resolve to an exact entry in `catalog`; a
/// merely well-typed but unlisted handle, label, or numeric pin therefore
/// cannot gain selector authority. The replacement must retain the same
/// reviewed node kind and may not duplicate an identity held by another node.
/// The complete candidate is structurally validated, semantically analyzed
/// against `registry`, and canonically encoded before `workspace` changes.
pub fn select_graph_capability_node_resource(
    catalog: &GraphCapabilityNodeCatalog,
    registry: &GraphDeploymentRegistry,
    workspace: &mut GraphWorkspaceDocument,
    node_id: GraphNodeId,
    entry_index: usize,
) -> Result<CanonicalGraphWorkspaceEncoding, GraphCapabilityResourceSelectionError> {
    let entry = catalog.entries().get(entry_index).ok_or(
        GraphCapabilityResourceSelectionError::UnknownEntry(entry_index),
    )?;
    let node = workspace
        .graph()
        .node(node_id)
        .ok_or(GraphCapabilityResourceSelectionError::UnknownNode(node_id))?;
    let current_entry_index = catalog.entry_index_for_node(node).ok_or(
        GraphCapabilityResourceSelectionError::NodeNotCatalogBound(node_id),
    )?;
    if current_entry_index == entry_index {
        return Err(GraphCapabilityResourceSelectionError::AlreadySelected {
            node: node_id,
            entry: entry_index,
        });
    }
    if node.kind() != entry.kind() {
        return Err(GraphCapabilityResourceSelectionError::NodeKindMismatch(
            node_id,
        ));
    }
    let parameter =
        entry
            .resource_parameter()
            .ok_or(GraphCapabilityResourceSelectionError::InvalidEntry {
                entry: entry_index,
                aspect: "resource parameter",
            })?;
    if parameter.value().value_type()
        != node
            .parameters()
            .iter()
            .find(|candidate| candidate.id() == parameter.id())
            .ok_or(GraphCapabilityResourceSelectionError::InvalidEntry {
                entry: entry_index,
                aspect: "node resource parameter",
            })?
            .value()
            .value_type()
    {
        return Err(GraphCapabilityResourceSelectionError::InvalidEntry {
            entry: entry_index,
            aspect: "resource parameter type",
        });
    }
    let GraphValue::ResourceHandle(selected_handle) = parameter.value().value() else {
        return Err(GraphCapabilityResourceSelectionError::InvalidEntry {
            entry: entry_index,
            aspect: "resource value",
        });
    };
    if let Some(existing) = workspace.graph().nodes().iter().find(|candidate| {
        candidate.id() != node_id
            && candidate.parameters().iter().any(|candidate| {
                matches!(
                    candidate.value().value(),
                    GraphValue::ResourceHandle(handle) if handle == selected_handle
                )
            })
    }) {
        return Err(
            GraphCapabilityResourceSelectionError::ResourceAlreadySelected {
                entry: entry_index,
                node: existing.id(),
            },
        );
    }

    let mut candidate = workspace.clone();
    candidate.set_parameter(node_id, parameter.id(), parameter.value().clone())?;
    analyze_graph_draft(candidate.graph(), registry.semantic_registry())?;
    let encoding = encode_graph_workspace(&candidate)?;
    *workspace = candidate;
    Ok(encoding)
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
        PortDefinition, ResourceClassId, TypeDefinition,
    };

    const DEVICE: DeviceId = DeviceId([0x54; 16]);
    const BOOL: GraphTypeId = GraphTypeId::new(1);
    const STREAM: GraphTypeId = GraphTypeId::new(2);
    const RESOURCE: GraphTypeId = GraphTypeId::new(3);
    const ROOT: GraphClockId = GraphClockId::new(1);
    const SAMPLE: GraphClockId = GraphClockId::new(2);
    const CLASS: ResourceClassId = ResourceClassId::new(1);

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
        let semantic = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &context,
            vec![input_schema],
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

        let selected =
            select_graph_capability_node_resource(&catalog, &registry, &mut workspace, node, 2)
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
            select_graph_capability_node_resource(&catalog, &registry, &mut workspace, node, 2,),
            Err(GraphCapabilityResourceSelectionError::AlreadySelected { node, entry: 2 })
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
            select_graph_capability_node_resource(&catalog, &registry, &mut workspace, first, 1,),
            Err(
                GraphCapabilityResourceSelectionError::ResourceAlreadySelected {
                    entry: 1,
                    node: second,
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
            select_graph_capability_node_resource(&catalog, &registry, &mut workspace, first, 2,),
            Err(GraphCapabilityResourceSelectionError::NodeNotCatalogBound(
                first
            ))
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
                2,
            ),
            Err(GraphCapabilityResourceSelectionError::NodeNotCatalogBound(
                second
            ))
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), raw);
    }

    #[test]
    fn catalog_resource_selection_rejects_kind_and_semantic_mismatch_atomically() {
        let registry = registry();
        let mut catalog = derive_graph_capability_node_catalog(
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

        let source = catalog.entries()[2].clone();
        let other_kind = NodeKind::new("alumina.io.other-reviewed-input", 1);
        let prototype = GraphNodePrototype::new(
            other_kind.clone(),
            source.prototype().label(),
            source.prototype().domain(),
            source.prototype().inputs().to_vec(),
            source.prototype().outputs().to_vec(),
            source.prototype().parameters().to_vec(),
        );
        catalog.entries.push(GraphCapabilityNodeEntry {
            resource: source.resource(),
            kind: other_kind,
            resource_parameter: source.resource_parameter_id(),
            prototype,
        });
        let other_index = catalog.entries().len() - 1;
        let retained = encode_graph_workspace(&workspace).unwrap();
        assert_eq!(
            select_graph_capability_node_resource(
                &catalog,
                &registry,
                &mut workspace,
                node,
                other_index,
            ),
            Err(GraphCapabilityResourceSelectionError::NodeKindMismatch(
                node
            ))
        );
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);

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
