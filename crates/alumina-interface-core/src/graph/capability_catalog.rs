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
    ExecutionDomain, GraphDeploymentNodeKind, GraphDeploymentRegistry, GraphDeploymentTarget,
    GraphNodePrototype, GraphSchemaError, GraphValue, NodeKind, NodeParameter, ResourceGraphHandle,
    TypeKind, TypedGraphValue,
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
        let context = GraphDocument::try_new(
            0,
            registry.semantic_registry().context_schema().clone(),
            registry.semantic_registry().context_clocks().to_vec(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let mut workspace = super::super::GraphWorkspaceDocument::try_new(
            super::super::GraphWorkspaceLimits::interactive(),
            0,
            1,
            1,
            context,
            Vec::new(),
        )
        .unwrap();
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
