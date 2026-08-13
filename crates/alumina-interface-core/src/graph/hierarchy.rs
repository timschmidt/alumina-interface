//! Canonical component-library bindings and deterministic hierarchy flattening.
//!
//! `ALGH` binds authoring-only instance nodes in one root `ALGW` to exact
//! canonical `ALGC` dependencies. Flattening removes every instance node and
//! produces an ordinary workspace containing only the component's structural
//! `ALGR` nodes and wires. Neither the hierarchy package nor flattening admits
//! node semantics, implementations, resources, timing, or deployment.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::{
    CanonicalGraphComponentEncoding, CanonicalGraphWorkspaceEncoding, ExecutionDomain,
    GraphComponentDocument, GraphComponentError, GraphComponentInputId, GraphComponentLimits,
    GraphComponentOutputId, GraphLimits, GraphNodeId, GraphNodePrototype, GraphPortId,
    GraphWorkspaceDocument, GraphWorkspaceError, GraphWorkspaceLimits, NodeKind, PortDefinition,
    WireEndpoint, encode_graph_component, encode_graph_workspace, replay_graph_component,
    replay_graph_workspace,
};

/// Magic bytes at the beginning of each canonical graph hierarchy.
pub const GRAPH_HIERARCHY_MAGIC: [u8; 4] = *b"ALGH";

/// Exact canonical graph-hierarchy format implemented by this source tree.
pub const GRAPH_HIERARCHY_VERSION: u16 = 1;

/// Reserved authoring-only node kind used for component instances before
/// flattening. It is never an executable semantic or firmware opcode kind.
pub const GRAPH_COMPONENT_INSTANCE_KIND: &str = "alumina.component.instance";

/// Version of the reserved authoring-only instance shape.
pub const GRAPH_COMPONENT_INSTANCE_VERSION: u16 = 1;

const GRAPH_HIERARCHY_FLAGS: u16 = 0;
const HIERARCHY_LIMIT_FIELD_COUNT: usize = 5;

/// Caller-owned and embedded bounds for one hierarchy and its flattened result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphHierarchyLimits {
    /// Maximum complete canonical `ALGH` bytes, including root and dependencies.
    pub maximum_hierarchy_bytes: usize,
    /// Maximum distinct canonical component dependencies.
    pub maximum_components: usize,
    /// Maximum component instances in the root workspace.
    pub maximum_instances: usize,
    /// Maximum nodes after complete flattening.
    pub maximum_flattened_nodes: usize,
    /// Maximum wires after complete flattening.
    pub maximum_flattened_wires: usize,
}

impl GraphHierarchyLimits {
    /// Bounded first hierarchy policy. The embedded root workspace may impose
    /// a lower placement ceiling.
    pub const fn interactive() -> Self {
        Self {
            maximum_hierarchy_bytes: 32 * 1024 * 1024,
            maximum_components: 64,
            maximum_instances: 256,
            maximum_flattened_nodes: 4_096,
            maximum_flattened_wires: 8_192,
        }
    }

    fn validate(self) -> Result<(), GraphHierarchyError> {
        if self.maximum_hierarchy_bytes == 0
            || self.maximum_components == 0
            || self.maximum_instances == 0
            || self.maximum_flattened_nodes == 0
            || self.maximum_flattened_wires == 0
        {
            Err(GraphHierarchyError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphHierarchyLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Exact binding from one root authoring node to a canonical component digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphComponentInstance {
    node: GraphNodeId,
    component: Digest,
}

impl GraphComponentInstance {
    /// Construct an instance binding. Complete hierarchy validation resolves
    /// the node, dependency, and exact derived connector shape.
    pub const fn new(node: GraphNodeId, component: Digest) -> Self {
        Self { node, component }
    }

    /// Return the root-workspace instance-node identity.
    pub const fn node(self) -> GraphNodeId {
        self.node
    }

    /// Return the exact canonical `ALGC` dependency identity.
    pub const fn component(self) -> Digest {
        self.component
    }
}

/// One canonical component retained by a hierarchy library.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphHierarchyDependency {
    document: GraphComponentDocument,
    encoding: CanonicalGraphComponentEncoding,
}

impl GraphHierarchyDependency {
    /// Borrow the validated component document.
    pub const fn document(&self) -> &GraphComponentDocument {
        &self.document
    }

    /// Borrow the exact canonical component bytes.
    pub const fn encoding(&self) -> &CanonicalGraphComponentEncoding {
        &self.encoding
    }

    /// Return the exact canonical component identity.
    pub const fn digest(&self) -> Digest {
        self.encoding.digest()
    }
}

/// Canonical hierarchy authoring document.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphHierarchyDocument {
    limits: GraphHierarchyLimits,
    revision: u64,
    root: GraphWorkspaceDocument,
    root_digest: Digest,
    dependencies: Vec<GraphHierarchyDependency>,
    instances: Vec<GraphComponentInstance>,
    flattened_nodes: usize,
    flattened_wires: usize,
}

impl GraphHierarchyDocument {
    /// Validate and canonicalize one complete root/library/instance package.
    pub fn try_new(
        limits: GraphHierarchyLimits,
        revision: u64,
        root: GraphWorkspaceDocument,
        components: Vec<GraphComponentDocument>,
        mut instances: Vec<GraphComponentInstance>,
    ) -> Result<Self, GraphHierarchyError> {
        limits.validate()?;
        if components.len() > limits.maximum_components {
            return Err(GraphHierarchyError::LimitExceeded("component count"));
        }
        if instances.len() > limits.maximum_instances {
            return Err(GraphHierarchyError::LimitExceeded("instance count"));
        }
        let root_digest = encode_graph_workspace(&root)?.digest();
        let mut dependencies = Vec::with_capacity(components.len());
        for document in components {
            let encoding = encode_graph_component(&document)?;
            dependencies.push(GraphHierarchyDependency { document, encoding });
        }
        dependencies.sort_unstable_by_key(GraphHierarchyDependency::digest);
        for pair in dependencies.windows(2) {
            if pair[0].digest() == pair[1].digest() {
                return Err(GraphHierarchyError::DuplicateComponent(pair[0].digest()));
            }
        }
        instances.sort_unstable_by_key(|instance| instance.node);
        let (flattened_nodes, flattened_wires) =
            validate_hierarchy(&root, &dependencies, &instances, limits)?;
        Ok(Self {
            limits,
            revision,
            root,
            root_digest,
            dependencies,
            instances,
            flattened_nodes,
            flattened_wires,
        })
    }

    /// Return embedded hierarchy limits.
    pub const fn limits(&self) -> GraphHierarchyLimits {
        self.limits
    }

    /// Return monotonic hierarchy-document revision metadata.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Borrow the root authoring workspace.
    pub const fn root(&self) -> &GraphWorkspaceDocument {
        &self.root
    }

    /// Return the identity of the exact embedded root `ALGW` bytes.
    pub const fn root_digest(&self) -> Digest {
        self.root_digest
    }

    /// Borrow canonical dependencies in digest order.
    pub fn dependencies(&self) -> &[GraphHierarchyDependency] {
        &self.dependencies
    }

    /// Borrow instance bindings in root-node order.
    pub fn instances(&self) -> &[GraphComponentInstance] {
        &self.instances
    }

    /// Return the statically proved final node count.
    pub const fn flattened_node_count(&self) -> usize {
        self.flattened_nodes
    }

    /// Return the statically proved final wire count.
    pub const fn flattened_wire_count(&self) -> usize {
        self.flattened_wires
    }

    /// Resolve one exact component identity.
    pub fn dependency(&self, digest: Digest) -> Option<&GraphHierarchyDependency> {
        self.dependencies
            .binary_search_by_key(&digest, GraphHierarchyDependency::digest)
            .ok()
            .map(|index| &self.dependencies[index])
    }
}

/// Canonical hierarchy bytes paired with their SHA-256 content identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphHierarchyEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphHierarchyEncoding {
    /// Borrow complete canonical bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return SHA-256 identity of exactly [`Self::bytes`].
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// Consume the carrier and return canonical bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Successfully replayed hierarchy and its canonical identity.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphHierarchyReplay {
    document: GraphHierarchyDocument,
    encoding: CanonicalGraphHierarchyEncoding,
}

impl GraphHierarchyReplay {
    /// Borrow the reconstructed hierarchy.
    pub const fn document(&self) -> &GraphHierarchyDocument {
        &self.document
    }

    /// Borrow byte-for-byte verified canonical encoding.
    pub const fn encoding(&self) -> &CanonicalGraphHierarchyEncoding {
        &self.encoding
    }

    /// Consume replay and return the hierarchy document.
    pub fn into_document(self) -> GraphHierarchyDocument {
        self.document
    }
}

/// Mapping from one component-local node to its flattened root identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphFlattenedNode {
    component_node: GraphNodeId,
    flattened_node: GraphNodeId,
}

impl GraphFlattenedNode {
    /// Return the node identity inside its source `ALGC`.
    pub const fn component_node(self) -> GraphNodeId {
        self.component_node
    }

    /// Return the new monotonic identity in the flattened root.
    pub const fn flattened_node(self) -> GraphNodeId {
        self.flattened_node
    }
}

/// Complete mapping report for one removed instance node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphFlattenedInstance {
    instance_node: GraphNodeId,
    component: Digest,
    nodes: Vec<GraphFlattenedNode>,
}

impl GraphFlattenedInstance {
    /// Return the removed root instance identity.
    pub const fn instance_node(&self) -> GraphNodeId {
        self.instance_node
    }

    /// Return the exact source component identity.
    pub const fn component(&self) -> Digest {
        self.component
    }

    /// Borrow component-to-root node mappings in component-node order.
    pub fn nodes(&self) -> &[GraphFlattenedNode] {
        &self.nodes
    }
}

/// Deterministically flattened ordinary workspace and audit report.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphHierarchyFlattening {
    source_digest: Digest,
    workspace: GraphWorkspaceDocument,
    encoding: CanonicalGraphWorkspaceEncoding,
    instances: Vec<GraphFlattenedInstance>,
}

impl GraphHierarchyFlattening {
    /// Return the canonical source `ALGH` identity.
    pub const fn source_digest(&self) -> Digest {
        self.source_digest
    }

    /// Borrow the flattened ordinary workspace.
    pub const fn workspace(&self) -> &GraphWorkspaceDocument {
        &self.workspace
    }

    /// Borrow the canonical flattened `ALGW` bytes.
    pub const fn encoding(&self) -> &CanonicalGraphWorkspaceEncoding {
        &self.encoding
    }

    /// Borrow mappings in original instance-node order.
    pub fn instances(&self) -> &[GraphFlattenedInstance] {
        &self.instances
    }

    /// Consume the report and return the flattened workspace.
    pub fn into_workspace(self) -> GraphWorkspaceDocument {
        self.workspace
    }
}

/// Rejection at hierarchy construction, replay, or flattening.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphHierarchyError {
    /// A hierarchy policy contained zero.
    ZeroLimit,
    /// A byte length, count, embedded policy, or flattened result exceeded policy.
    LimitExceeded(&'static str),
    /// Input did not begin with [`GRAPH_HIERARCHY_MAGIC`].
    InvalidMagic,
    /// Hierarchy format version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved hierarchy flags were nonzero.
    UnsupportedFlags(u16),
    /// A fixed-width or length-delimited field ran past input.
    Truncated,
    /// A canonical field could not represent an in-memory value.
    IntegerOverflow(&'static str),
    /// Valid fields remained after the hierarchy.
    TrailingBytes,
    /// Decoding and canonical reconstruction changed at least one byte.
    NonCanonical,
    /// Two dependency records had the same canonical identity.
    DuplicateComponent(Digest),
    /// An instance referenced a dependency absent from the library.
    UnknownComponent(Digest),
    /// Two bindings referenced the same root node.
    DuplicateInstance(GraphNodeId),
    /// An instance binding referenced no root node.
    UnknownInstanceNode(GraphNodeId),
    /// A binding referenced a normal node rather than the reserved instance kind.
    NotComponentInstance(GraphNodeId),
    /// The reserved instance name used a version this hierarchy cannot derive.
    UnsupportedInstanceVersion {
        /// Root authoring node.
        node: GraphNodeId,
        /// Unsupported structural version.
        version: u16,
    },
    /// A reserved authoring instance node had no exact dependency binding.
    MissingInstanceBinding(GraphNodeId),
    /// A reserved instance node contradicted its component-derived shape.
    InstanceShapeMismatch {
        /// Root instance node.
        node: GraphNodeId,
        /// Mismatched shape collection.
        aspect: &'static str,
    },
    /// A component type registry or clock set differed from the root authority.
    SemanticContextMismatch(Digest),
    /// V1 dependencies must be leaf components and contained another instance.
    NestedComponentInstance {
        /// Source component identity.
        component: Digest,
        /// Forbidden internal instance node.
        node: GraphNodeId,
    },
    /// A connector port could not map to a validated component terminal.
    UnknownInstancePort(WireEndpoint),
    /// Presentation translation could not fit the root canvas lattice.
    PlacementOverflow {
        /// Removed instance node.
        instance: GraphNodeId,
        /// Component-local node whose placement failed.
        component_node: GraphNodeId,
    },
    /// Embedded component encoding/replay failed.
    Component(GraphComponentError),
    /// Root or flattened workspace encoding/edit/replay failed.
    Workspace(GraphWorkspaceError),
}

impl From<GraphComponentError> for GraphHierarchyError {
    fn from(value: GraphComponentError) -> Self {
        Self::Component(value)
    }
}

impl From<GraphWorkspaceError> for GraphHierarchyError {
    fn from(value: GraphWorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl fmt::Display for GraphHierarchyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph hierarchy policy contains zero"),
            Self::LimitExceeded(name) => {
                write!(formatter, "graph hierarchy {name} exceeds policy")
            }
            Self::InvalidMagic => formatter.write_str("graph hierarchy magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "graph hierarchy version {version} is unsupported"
                )
            }
            Self::UnsupportedFlags(flags) => {
                write!(
                    formatter,
                    "graph hierarchy flags {flags:#06x} are unsupported"
                )
            }
            Self::Truncated => formatter.write_str("graph hierarchy is truncated"),
            Self::IntegerOverflow(name) => {
                write!(
                    formatter,
                    "graph hierarchy {name} exceeds its integer width"
                )
            }
            Self::TrailingBytes => formatter.write_str("graph hierarchy has trailing bytes"),
            Self::NonCanonical => formatter.write_str("graph hierarchy bytes are noncanonical"),
            Self::DuplicateComponent(digest) => {
                write!(
                    formatter,
                    "graph hierarchy component {digest:?} is duplicated"
                )
            }
            Self::UnknownComponent(digest) => {
                write!(formatter, "graph hierarchy component {digest:?} is unknown")
            }
            Self::DuplicateInstance(node) => {
                write!(
                    formatter,
                    "graph hierarchy instance node {node:?} is duplicated"
                )
            }
            Self::UnknownInstanceNode(node) => {
                write!(
                    formatter,
                    "graph hierarchy instance node {node:?} is unknown"
                )
            }
            Self::NotComponentInstance(node) => {
                write!(
                    formatter,
                    "graph hierarchy node {node:?} is not a component instance"
                )
            }
            Self::UnsupportedInstanceVersion { node, version } => write!(
                formatter,
                "graph hierarchy instance node {node:?} uses unsupported version {version}"
            ),
            Self::MissingInstanceBinding(node) => {
                write!(
                    formatter,
                    "graph hierarchy instance node {node:?} has no binding"
                )
            }
            Self::InstanceShapeMismatch { node, aspect } => write!(
                formatter,
                "graph hierarchy instance node {node:?} has mismatched {aspect}"
            ),
            Self::SemanticContextMismatch(digest) => write!(
                formatter,
                "graph hierarchy component {digest:?} has a different type or clock context"
            ),
            Self::NestedComponentInstance { component, node } => write!(
                formatter,
                "graph hierarchy leaf component {component:?} contains instance node {node:?}"
            ),
            Self::UnknownInstancePort(endpoint) => {
                write!(
                    formatter,
                    "graph hierarchy instance port {endpoint:?} is unknown"
                )
            }
            Self::PlacementOverflow {
                instance,
                component_node,
            } => write!(
                formatter,
                "graph hierarchy instance {instance:?} component node {component_node:?} exceeds the root canvas"
            ),
            Self::Component(error) => write!(formatter, "component dependency failed: {error}"),
            Self::Workspace(error) => write!(formatter, "hierarchy workspace failed: {error}"),
        }
    }
}

impl std::error::Error for GraphHierarchyError {}

/// Derive the one structural placeholder shape for a canonical component.
/// The returned node is authoring-only and must be flattened before semantic
/// analysis or execution.
pub fn graph_component_instance_prototype(
    component: &GraphComponentDocument,
    label: impl Into<String>,
) -> Result<GraphNodePrototype, GraphHierarchyError> {
    let (inputs, outputs) = component_instance_ports(component)?;
    Ok(GraphNodePrototype::new(
        NodeKind::new(
            GRAPH_COMPONENT_INSTANCE_KIND,
            GRAPH_COMPONENT_INSTANCE_VERSION,
        ),
        label,
        ExecutionDomain::HostExact,
        inputs,
        outputs,
        Vec::new(),
    ))
}

/// Resolve one public input identity to its derived instance-node input port.
pub fn graph_component_instance_input_port(
    component: &GraphComponentDocument,
    input: GraphComponentInputId,
) -> Option<GraphPortId> {
    let index = component
        .inputs()
        .binary_search_by_key(&input, super::GraphComponentInput::id)
        .ok()?;
    u32::try_from(index.checked_add(1)?)
        .ok()
        .map(GraphPortId::new)
}

/// Resolve one public output identity to its derived instance-node output port.
pub fn graph_component_instance_output_port(
    component: &GraphComponentDocument,
    output: GraphComponentOutputId,
) -> Option<GraphPortId> {
    let index = component
        .outputs()
        .binary_search_by_key(&output, super::GraphComponentOutput::id)
        .ok()?;
    component
        .inputs()
        .len()
        .checked_add(index)?
        .checked_add(1)
        .and_then(|value| u32::try_from(value).ok())
        .map(GraphPortId::new)
}

/// Encode one validated hierarchy and compute its content identity.
pub fn encode_graph_hierarchy(
    document: &GraphHierarchyDocument,
) -> Result<CanonicalGraphHierarchyEncoding, GraphHierarchyError> {
    let root = encode_graph_workspace(document.root())?;
    if root.digest() != document.root_digest {
        return Err(GraphHierarchyError::NonCanonical);
    }
    let mut encoder = Encoder::default();
    encoder.bytes(&GRAPH_HIERARCHY_MAGIC);
    encoder.u16(GRAPH_HIERARCHY_VERSION);
    encoder.u16(GRAPH_HIERARCHY_FLAGS);
    encode_limits(&mut encoder, document.limits)?;
    encoder.u64(document.revision);
    encoder.length_prefixed(root.bytes(), "root workspace length")?;
    encoder.u32(
        u32::try_from(document.dependencies.len())
            .map_err(|_| GraphHierarchyError::IntegerOverflow("component count"))?,
    );
    for dependency in &document.dependencies {
        let encoding = encode_graph_component(&dependency.document)?;
        if encoding != dependency.encoding {
            return Err(GraphHierarchyError::NonCanonical);
        }
        encoder.length_prefixed(encoding.bytes(), "component length")?;
    }
    encoder.u32(
        u32::try_from(document.instances.len())
            .map_err(|_| GraphHierarchyError::IntegerOverflow("instance count"))?,
    );
    for instance in &document.instances {
        encoder.u32(instance.node.get());
        encoder.bytes(&instance.component.0);
    }
    if encoder.0.len() > document.limits.maximum_hierarchy_bytes {
        return Err(GraphHierarchyError::LimitExceeded("document byte length"));
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphHierarchyEncoding {
        bytes: encoder.0,
        digest,
    })
}

/// Decode, validate, canonically re-encode, and identify untrusted `ALGH` bytes.
#[allow(
    clippy::too_many_arguments,
    reason = "each nested canonical envelope retains an independent caller-owned admission policy"
)]
pub fn replay_graph_hierarchy(
    bytes: &[u8],
    hierarchy_admission: GraphHierarchyLimits,
    component_admission: GraphComponentLimits,
    workspace_admission: GraphWorkspaceLimits,
    graph_admission: GraphLimits,
) -> Result<GraphHierarchyReplay, GraphHierarchyError> {
    hierarchy_admission.validate()?;
    if bytes.len() > hierarchy_admission.maximum_hierarchy_bytes {
        return Err(GraphHierarchyError::LimitExceeded(
            "admitted document byte length",
        ));
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_HIERARCHY_MAGIC.len())? != GRAPH_HIERARCHY_MAGIC {
        return Err(GraphHierarchyError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_HIERARCHY_VERSION {
        return Err(GraphHierarchyError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_HIERARCHY_FLAGS {
        return Err(GraphHierarchyError::UnsupportedFlags(flags));
    }
    let limits = decode_limits(&mut decoder)?;
    if !limits_within(limits, hierarchy_admission) {
        return Err(GraphHierarchyError::LimitExceeded(
            "embedded admission limit",
        ));
    }
    limits.validate()?;
    if bytes.len() > limits.maximum_hierarchy_bytes {
        return Err(GraphHierarchyError::LimitExceeded(
            "embedded document byte length",
        ));
    }
    let revision = decoder.u64()?;
    let root_bytes = decoder.length_prefixed(
        workspace_admission.maximum_workspace_bytes,
        "root workspace length",
    )?;
    let root =
        replay_graph_workspace(root_bytes, workspace_admission, graph_admission)?.into_document();
    let component_count = decoder.count(limits.maximum_components, "component count")?;
    let mut components = Vec::with_capacity(component_count);
    for _ in 0..component_count {
        let component_bytes = decoder.length_prefixed(
            component_admission.maximum_component_bytes,
            "component length",
        )?;
        components.push(
            replay_graph_component(
                component_bytes,
                component_admission,
                workspace_admission,
                graph_admission,
            )?
            .into_document(),
        );
    }
    let instance_count = decoder.count(limits.maximum_instances, "instance count")?;
    let mut instances = Vec::with_capacity(instance_count);
    for _ in 0..instance_count {
        let node = GraphNodeId::new(decoder.u32()?);
        let digest: [u8; 32] = decoder
            .take(32)?
            .try_into()
            .map_err(|_| GraphHierarchyError::Truncated)?;
        instances.push(GraphComponentInstance::new(node, Digest(digest)));
    }
    if !decoder.is_empty() {
        return Err(GraphHierarchyError::TrailingBytes);
    }
    let document = GraphHierarchyDocument::try_new(limits, revision, root, components, instances)?;
    let encoding = encode_graph_hierarchy(&document)?;
    if encoding.bytes() != bytes {
        return Err(GraphHierarchyError::NonCanonical);
    }
    Ok(GraphHierarchyReplay { document, encoding })
}

/// Deterministically remove every authoring instance and reconnect all public
/// terminals to freshly allocated component-internal nodes.
pub fn flatten_graph_hierarchy(
    hierarchy: &GraphHierarchyDocument,
) -> Result<GraphHierarchyFlattening, GraphHierarchyError> {
    let source_digest = encode_graph_hierarchy(hierarchy)?.digest();
    let mut workspace = hierarchy.root.clone();
    let mut reports = Vec::with_capacity(hierarchy.instances.len());
    for instance in &hierarchy.instances {
        let dependency = hierarchy
            .dependency(instance.component)
            .ok_or(GraphHierarchyError::UnknownComponent(instance.component))?;
        reports.push(flatten_instance(
            &mut workspace,
            *instance,
            dependency.document(),
        )?);
    }
    if workspace.graph().nodes().len() != hierarchy.flattened_nodes {
        return Err(GraphHierarchyError::NonCanonical);
    }
    if workspace.graph().wires().len() != hierarchy.flattened_wires {
        return Err(GraphHierarchyError::NonCanonical);
    }
    if workspace
        .graph()
        .nodes()
        .iter()
        .any(has_component_instance_name)
    {
        return Err(GraphHierarchyError::NonCanonical);
    }
    let encoding = encode_graph_workspace(&workspace)?;
    Ok(GraphHierarchyFlattening {
        source_digest,
        workspace,
        encoding,
        instances: reports,
    })
}

fn validate_hierarchy(
    root: &GraphWorkspaceDocument,
    dependencies: &[GraphHierarchyDependency],
    instances: &[GraphComponentInstance],
    limits: GraphHierarchyLimits,
) -> Result<(usize, usize), GraphHierarchyError> {
    let mut prior_instance = None;
    let mut bound_nodes = BTreeSet::new();
    let mut flattened_nodes = root.graph().nodes().len();
    let mut flattened_wires = root.graph().wires().len();
    for node in root
        .graph()
        .nodes()
        .iter()
        .filter(|node| has_component_instance_name(node))
    {
        if node.kind().version() != GRAPH_COMPONENT_INSTANCE_VERSION {
            return Err(GraphHierarchyError::UnsupportedInstanceVersion {
                node: node.id(),
                version: node.kind().version(),
            });
        }
    }
    for dependency in dependencies {
        validate_dependency_context(root, dependency)?;
    }
    for instance in instances {
        if instance.node.get() == 0 {
            return Err(GraphHierarchyError::UnknownInstanceNode(instance.node));
        }
        if prior_instance == Some(instance.node) {
            return Err(GraphHierarchyError::DuplicateInstance(instance.node));
        }
        prior_instance = Some(instance.node);
        let node = root
            .graph()
            .node(instance.node)
            .ok_or(GraphHierarchyError::UnknownInstanceNode(instance.node))?;
        if !is_component_instance_node(node) {
            return Err(GraphHierarchyError::NotComponentInstance(instance.node));
        }
        let dependency = dependency_by_digest(dependencies, instance.component)
            .ok_or(GraphHierarchyError::UnknownComponent(instance.component))?;
        validate_instance_node(node, dependency.document())?;
        bound_nodes.insert(instance.node);
        flattened_nodes = flattened_nodes
            .checked_sub(1)
            .and_then(|count| {
                count.checked_add(dependency.document().workspace().graph().nodes().len())
            })
            .ok_or(GraphHierarchyError::IntegerOverflow("flattened node count"))?;
        flattened_wires = flattened_wires
            .checked_add(dependency.document().workspace().graph().wires().len())
            .ok_or(GraphHierarchyError::IntegerOverflow("flattened wire count"))?;
    }
    for node in root
        .graph()
        .nodes()
        .iter()
        .filter(|node| is_component_instance_node(node))
    {
        if !bound_nodes.contains(&node.id()) {
            return Err(GraphHierarchyError::MissingInstanceBinding(node.id()));
        }
    }
    if flattened_nodes > limits.maximum_flattened_nodes
        || flattened_nodes > root.graph().schema().limits().maximum_nodes
        || flattened_nodes > root.limits().maximum_placements
    {
        return Err(GraphHierarchyError::LimitExceeded("flattened node count"));
    }
    if flattened_wires > limits.maximum_flattened_wires
        || flattened_wires > root.graph().schema().limits().maximum_wires
    {
        return Err(GraphHierarchyError::LimitExceeded("flattened wire count"));
    }
    Ok((flattened_nodes, flattened_wires))
}

fn validate_dependency_context(
    root: &GraphWorkspaceDocument,
    dependency: &GraphHierarchyDependency,
) -> Result<(), GraphHierarchyError> {
    let component = dependency.document();
    if component.workspace().graph().schema() != root.graph().schema()
        || component.workspace().graph().clocks() != root.graph().clocks()
    {
        return Err(GraphHierarchyError::SemanticContextMismatch(
            dependency.digest(),
        ));
    }
    if let Some(node) = component
        .workspace()
        .graph()
        .nodes()
        .iter()
        .find(|node| has_component_instance_name(node))
    {
        return Err(GraphHierarchyError::NestedComponentInstance {
            component: dependency.digest(),
            node: node.id(),
        });
    }
    Ok(())
}

fn dependency_by_digest(
    dependencies: &[GraphHierarchyDependency],
    digest: Digest,
) -> Option<&GraphHierarchyDependency> {
    dependencies
        .binary_search_by_key(&digest, GraphHierarchyDependency::digest)
        .ok()
        .map(|index| &dependencies[index])
}

fn validate_instance_node(
    node: &super::NodeDefinition,
    component: &GraphComponentDocument,
) -> Result<(), GraphHierarchyError> {
    let prototype = graph_component_instance_prototype(component, node.label())?;
    if node.kind() != prototype.kind() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            node: node.id(),
            aspect: "kind",
        });
    }
    if node.domain() != prototype.domain() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            node: node.id(),
            aspect: "domain",
        });
    }
    if node.inputs() != prototype.inputs() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            node: node.id(),
            aspect: "inputs",
        });
    }
    if node.outputs() != prototype.outputs() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            node: node.id(),
            aspect: "outputs",
        });
    }
    if !node.parameters().is_empty() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            node: node.id(),
            aspect: "parameters",
        });
    }
    Ok(())
}

fn is_component_instance_node(node: &super::NodeDefinition) -> bool {
    has_component_instance_name(node) && node.kind().version() == GRAPH_COMPONENT_INSTANCE_VERSION
}

fn has_component_instance_name(node: &super::NodeDefinition) -> bool {
    node.kind().name() == GRAPH_COMPONENT_INSTANCE_KIND
}

fn component_instance_ports(
    component: &GraphComponentDocument,
) -> Result<(Vec<PortDefinition>, Vec<PortDefinition>), GraphHierarchyError> {
    let total = component
        .inputs()
        .len()
        .checked_add(component.outputs().len())
        .ok_or(GraphHierarchyError::IntegerOverflow("instance port count"))?;
    u32::try_from(total)
        .map_err(|_| GraphHierarchyError::IntegerOverflow("instance port count"))?;
    let mut inputs = Vec::with_capacity(component.inputs().len());
    for (index, input) in component.inputs().iter().enumerate() {
        let port = u32::try_from(index + 1)
            .map_err(|_| GraphHierarchyError::IntegerOverflow("instance input port"))?;
        let value_type = component
            .input_value_type(input.id())
            .ok_or(GraphHierarchyError::NonCanonical)?;
        inputs.push(PortDefinition::new(
            GraphPortId::new(port),
            input.name(),
            value_type,
        ));
    }
    let mut outputs = Vec::with_capacity(component.outputs().len());
    for (index, output) in component.outputs().iter().enumerate() {
        let port = component
            .inputs()
            .len()
            .checked_add(index)
            .and_then(|value| value.checked_add(1))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(GraphHierarchyError::IntegerOverflow("instance output port"))?;
        let value_type = component
            .output_value_type(output.id())
            .ok_or(GraphHierarchyError::NonCanonical)?;
        outputs.push(PortDefinition::new(
            GraphPortId::new(port),
            output.name(),
            value_type,
        ));
    }
    Ok((inputs, outputs))
}

fn flatten_instance(
    root: &mut GraphWorkspaceDocument,
    instance: GraphComponentInstance,
    component: &GraphComponentDocument,
) -> Result<GraphFlattenedInstance, GraphHierarchyError> {
    let placement = root
        .placement(instance.node)
        .ok_or(GraphHierarchyError::UnknownInstanceNode(instance.node))?;
    let incident = root
        .graph()
        .wires()
        .iter()
        .copied()
        .filter(|wire| wire.source().node == instance.node || wire.target().node == instance.node)
        .collect::<Vec<_>>();
    root.delete_node(instance.node)?;

    let component_workspace = component.workspace();
    let origin_x = component_workspace
        .placements()
        .iter()
        .map(|placement| placement.x())
        .min()
        .unwrap_or(0);
    let origin_y = component_workspace
        .placements()
        .iter()
        .map(|placement| placement.y())
        .min()
        .unwrap_or(0);
    let mut node_map = BTreeMap::new();
    let mut nodes = Vec::with_capacity(component_workspace.graph().nodes().len());
    for node in component_workspace.graph().nodes() {
        let component_placement = component_workspace
            .placement(node.id())
            .ok_or(GraphHierarchyError::NonCanonical)?;
        let x = translated_coordinate(
            placement.x(),
            component_placement.x(),
            origin_x,
            instance.node,
            node.id(),
        )?;
        let y = translated_coordinate(
            placement.y(),
            component_placement.y(),
            origin_y,
            instance.node,
            node.id(),
        )?;
        let prototype = GraphNodePrototype::new(
            node.kind().clone(),
            node.label(),
            node.domain(),
            node.inputs().to_vec(),
            node.outputs().to_vec(),
            node.parameters().to_vec(),
        );
        let flattened = root.create_node(prototype, x, y)?;
        node_map.insert(node.id(), flattened);
        nodes.push(GraphFlattenedNode {
            component_node: node.id(),
            flattened_node: flattened,
        });
    }
    for wire in component_workspace.graph().wires() {
        root.connect(
            remap_component_endpoint(wire.source(), &node_map)?,
            remap_component_endpoint(wire.target(), &node_map)?,
        )?;
    }
    for wire in incident {
        let source = if wire.source().node == instance.node {
            remap_public_output(component, wire.source(), &node_map)?
        } else {
            wire.source()
        };
        let target = if wire.target().node == instance.node {
            remap_public_input(component, wire.target(), &node_map)?
        } else {
            wire.target()
        };
        root.connect(source, target)?;
    }
    Ok(GraphFlattenedInstance {
        instance_node: instance.node,
        component: instance.component,
        nodes,
    })
}

fn translated_coordinate(
    root: i32,
    child: i32,
    origin: i32,
    instance: GraphNodeId,
    component_node: GraphNodeId,
) -> Result<i32, GraphHierarchyError> {
    let translated = i64::from(root)
        .checked_add(i64::from(child) - i64::from(origin))
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(GraphHierarchyError::PlacementOverflow {
            instance,
            component_node,
        })?;
    Ok(translated)
}

fn remap_component_endpoint(
    endpoint: WireEndpoint,
    node_map: &BTreeMap<GraphNodeId, GraphNodeId>,
) -> Result<WireEndpoint, GraphHierarchyError> {
    Ok(WireEndpoint {
        node: *node_map
            .get(&endpoint.node)
            .ok_or(GraphHierarchyError::NonCanonical)?,
        port: endpoint.port,
    })
}

fn remap_public_input(
    component: &GraphComponentDocument,
    endpoint: WireEndpoint,
    node_map: &BTreeMap<GraphNodeId, GraphNodeId>,
) -> Result<WireEndpoint, GraphHierarchyError> {
    let index = usize::try_from(endpoint.port.get())
        .ok()
        .and_then(|port| port.checked_sub(1))
        .ok_or(GraphHierarchyError::UnknownInstancePort(endpoint))?;
    let target = component
        .inputs()
        .get(index)
        .ok_or(GraphHierarchyError::UnknownInstancePort(endpoint))?
        .target();
    remap_component_endpoint(target, node_map)
}

fn remap_public_output(
    component: &GraphComponentDocument,
    endpoint: WireEndpoint,
    node_map: &BTreeMap<GraphNodeId, GraphNodeId>,
) -> Result<WireEndpoint, GraphHierarchyError> {
    let index = usize::try_from(endpoint.port.get())
        .ok()
        .and_then(|port| port.checked_sub(component.inputs().len()))
        .and_then(|port| port.checked_sub(1))
        .ok_or(GraphHierarchyError::UnknownInstancePort(endpoint))?;
    let source = component
        .outputs()
        .get(index)
        .ok_or(GraphHierarchyError::UnknownInstancePort(endpoint))?
        .source();
    remap_component_endpoint(source, node_map)
}

fn encode_limits(
    encoder: &mut Encoder,
    limits: GraphHierarchyLimits,
) -> Result<(), GraphHierarchyError> {
    for value in [
        limits.maximum_hierarchy_bytes,
        limits.maximum_components,
        limits.maximum_instances,
        limits.maximum_flattened_nodes,
        limits.maximum_flattened_wires,
    ] {
        encoder.u64(
            u64::try_from(value)
                .map_err(|_| GraphHierarchyError::IntegerOverflow("limit value"))?,
        );
    }
    Ok(())
}

fn decode_limits(decoder: &mut Decoder<'_>) -> Result<GraphHierarchyLimits, GraphHierarchyError> {
    let mut values = [0_usize; HIERARCHY_LIMIT_FIELD_COUNT];
    for value in &mut values {
        *value = usize::try_from(decoder.u64()?)
            .map_err(|_| GraphHierarchyError::IntegerOverflow("limit value"))?;
    }
    Ok(GraphHierarchyLimits {
        maximum_hierarchy_bytes: values[0],
        maximum_components: values[1],
        maximum_instances: values[2],
        maximum_flattened_nodes: values[3],
        maximum_flattened_wires: values[4],
    })
}

const fn limits_within(embedded: GraphHierarchyLimits, admission: GraphHierarchyLimits) -> bool {
    embedded.maximum_hierarchy_bytes <= admission.maximum_hierarchy_bytes
        && embedded.maximum_components <= admission.maximum_components
        && embedded.maximum_instances <= admission.maximum_instances
        && embedded.maximum_flattened_nodes <= admission.maximum_flattened_nodes
        && embedded.maximum_flattened_wires <= admission.maximum_flattened_wires
}

#[derive(Default)]
struct Encoder(Vec<u8>);

impl Encoder {
    fn bytes(&mut self, value: &[u8]) {
        self.0.extend_from_slice(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn length_prefixed(
        &mut self,
        value: &[u8],
        name: &'static str,
    ) -> Result<(), GraphHierarchyError> {
        self.u32(
            u32::try_from(value.len()).map_err(|_| GraphHierarchyError::IntegerOverflow(name))?,
        );
        self.bytes(value);
        Ok(())
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphHierarchyError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(GraphHierarchyError::Truncated)?;
        let result = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GraphHierarchyError::Truncated)?;
        self.cursor = end;
        Ok(result)
    }

    fn u16(&mut self) -> Result<u16, GraphHierarchyError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| GraphHierarchyError::Truncated)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, GraphHierarchyError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| GraphHierarchyError::Truncated)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, GraphHierarchyError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| GraphHierarchyError::Truncated)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn count(&mut self, maximum: usize, name: &'static str) -> Result<usize, GraphHierarchyError> {
        let count =
            usize::try_from(self.u32()?).map_err(|_| GraphHierarchyError::IntegerOverflow(name))?;
        if count > maximum {
            Err(GraphHierarchyError::LimitExceeded(name))
        } else {
            Ok(count)
        }
    }

    fn length_prefixed(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<&'a [u8], GraphHierarchyError> {
        let length =
            usize::try_from(self.u32()?).map_err(|_| GraphHierarchyError::IntegerOverflow(name))?;
        if length > maximum {
            return Err(GraphHierarchyError::LimitExceeded(name));
        }
        self.take(length)
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        ClockDefinition, ClockKind, GraphClockId, GraphComponentInput, GraphComponentInputId,
        GraphComponentOutput, GraphComponentOutputId, GraphDocument, GraphNodePlacement,
        GraphWireId, NodeDefinition, RepresentativeControlSignal, WireDefinition, analyze_graph,
        compile_representative_exact_control_graph,
    };

    fn endpoint(node: u32, port: u32) -> WireEndpoint {
        WireEndpoint {
            node: GraphNodeId::new(node),
            port: GraphPortId::new(port),
        }
    }

    fn component_workspace() -> GraphWorkspaceDocument {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let mut workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            20,
            23,
            fixture.document().clone(),
            fixture
                .document()
                .nodes()
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let index = i32::try_from(index).unwrap();
                    GraphNodePlacement::new(node.id(), 20 + index * 30, 40 + index * 12)
                })
                .collect(),
        )
        .unwrap();
        workspace.disconnect(GraphWireId::new(1)).unwrap();
        workspace.delete_node(GraphNodeId::new(1)).unwrap();
        workspace
    }

    fn component() -> GraphComponentDocument {
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.pid_leaf",
            2,
            2,
            1,
            component_workspace(),
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "setpoint_samples",
                endpoint(4, 1),
            )],
            vec![GraphComponentOutput::new(
                GraphComponentOutputId::new(1),
                "permitted_output",
                RepresentativeControlSignal::PermittedOutput.endpoint(),
            )],
            Vec::new(),
        )
        .unwrap()
    }

    fn chain_component() -> GraphComponentDocument {
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.chain_leaf",
            2,
            3,
            1,
            component_workspace(),
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "setpoint_samples",
                endpoint(4, 1),
            )],
            vec![
                GraphComponentOutput::new(
                    GraphComponentOutputId::new(1),
                    "permitted_output",
                    RepresentativeControlSignal::PermittedOutput.endpoint(),
                ),
                GraphComponentOutput::new(
                    GraphComponentOutputId::new(2),
                    "source_samples",
                    endpoint(2, 1),
                ),
            ],
            Vec::new(),
        )
        .unwrap()
    }

    fn root(component: &GraphComponentDocument) -> GraphWorkspaceDocument {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let source = fixture
            .document()
            .node(GraphNodeId::new(1))
            .unwrap()
            .clone();
        let sink = fixture.document().node(GraphNodeId::new(19)).unwrap();
        let sink = NodeDefinition::new(
            GraphNodeId::new(3),
            sink.kind().clone(),
            "External flattened sink",
            sink.domain(),
            sink.inputs().to_vec(),
            sink.outputs().to_vec(),
            sink.parameters().to_vec(),
        );
        let prototype = graph_component_instance_prototype(component, "PID leaf instance").unwrap();
        let instance = NodeDefinition::new(
            GraphNodeId::new(2),
            prototype.kind().clone(),
            "PID leaf instance",
            prototype.domain(),
            prototype.inputs().to_vec(),
            prototype.outputs().to_vec(),
            prototype.parameters().to_vec(),
        );
        let graph = GraphDocument::try_new(
            1,
            fixture.document().schema().clone(),
            fixture.document().clocks().to_vec(),
            vec![source, instance, sink],
            vec![
                WireDefinition::new(GraphWireId::new(1), endpoint(1, 1), endpoint(2, 1)),
                WireDefinition::new(GraphWireId::new(2), endpoint(2, 2), endpoint(3, 1)),
            ],
        )
        .unwrap();
        GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            4,
            3,
            graph,
            vec![
                GraphNodePlacement::new(GraphNodeId::new(1), 0, 100),
                GraphNodePlacement::new(GraphNodeId::new(2), 300, 100),
                GraphNodePlacement::new(GraphNodeId::new(3), 900, 100),
            ],
        )
        .unwrap()
    }

    fn hierarchy() -> GraphHierarchyDocument {
        let component = component();
        let digest = encode_graph_component(&component).unwrap().digest();
        GraphHierarchyDocument::try_new(
            GraphHierarchyLimits::interactive(),
            1,
            root(&component),
            vec![component],
            vec![GraphComponentInstance::new(GraphNodeId::new(2), digest)],
        )
        .unwrap()
    }

    fn root_with_replacement(
        root: &GraphWorkspaceDocument,
        replacement: NodeDefinition,
    ) -> GraphWorkspaceDocument {
        let graph = GraphDocument::try_new(
            root.graph().revision(),
            root.graph().schema().clone(),
            root.graph().clocks().to_vec(),
            root.graph()
                .nodes()
                .iter()
                .map(|node| {
                    if node.id() == replacement.id() {
                        replacement.clone()
                    } else {
                        node.clone()
                    }
                })
                .collect(),
            root.graph().wires().to_vec(),
        )
        .unwrap();
        GraphWorkspaceDocument::try_new(
            root.limits(),
            root.revision(),
            root.next_node_id(),
            root.next_wire_id(),
            graph,
            root.placements().to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn canonical_hierarchy_round_trips_and_flattens_exact_connectors() {
        let hierarchy = hierarchy();
        let component = hierarchy.dependencies()[0].document();
        assert_eq!(
            graph_component_instance_input_port(component, GraphComponentInputId::new(1)),
            Some(GraphPortId::new(1))
        );
        assert_eq!(
            graph_component_instance_output_port(component, GraphComponentOutputId::new(1)),
            Some(GraphPortId::new(2))
        );
        assert_eq!(hierarchy.flattened_node_count(), 20);
        assert_eq!(hierarchy.flattened_wire_count(), 23);
        let encoding = encode_graph_hierarchy(&hierarchy).unwrap();
        let replay = replay_graph_hierarchy(
            encoding.bytes(),
            GraphHierarchyLimits::interactive(),
            GraphComponentLimits::interactive(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.document(), &hierarchy);
        assert_eq!(replay.encoding(), &encoding);

        let flattened = flatten_graph_hierarchy(&hierarchy).unwrap();
        assert_eq!(flattened.source_digest(), encoding.digest());
        assert_eq!(flattened.workspace().graph().nodes().len(), 20);
        assert_eq!(flattened.workspace().graph().wires().len(), 23);
        assert_eq!(flattened.instances().len(), 1);
        assert_eq!(flattened.instances()[0].nodes().len(), 18);
        let map = flattened.instances()[0].nodes();
        let remapped_four = map
            .iter()
            .find(|mapping| mapping.component_node() == GraphNodeId::new(4))
            .unwrap()
            .flattened_node();
        let remapped_eighteen = map
            .iter()
            .find(|mapping| mapping.component_node() == GraphNodeId::new(18))
            .unwrap()
            .flattened_node();
        assert!(flattened.workspace().graph().wires().iter().any(|wire| {
            wire.source() == endpoint(1, 1)
                && wire.target()
                    == WireEndpoint {
                        node: remapped_four,
                        port: GraphPortId::new(1),
                    }
        }));
        assert!(flattened.workspace().graph().wires().iter().any(|wire| {
            wire.source()
                == WireEndpoint {
                    node: remapped_eighteen,
                    port: GraphPortId::new(3),
                }
                && wire.target() == endpoint(3, 1)
        }));
        let fixture = compile_representative_exact_control_graph().unwrap();
        assert!(
            analyze_graph(
                flattened.workspace().graph(),
                fixture.registry().semantic_registry()
            )
            .is_ok()
        );
        let replay = replay_graph_workspace(
            flattened.encoding().bytes(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.document(), flattened.workspace());
    }

    #[test]
    fn unresolved_missing_and_misshaped_instances_fail_before_flattening() {
        let initial_component = component();
        let digest = encode_graph_component(&initial_component).unwrap().digest();
        let initial_root = root(&initial_component);
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                initial_root.clone(),
                vec![initial_component.clone()],
                Vec::new(),
            ),
            Err(GraphHierarchyError::MissingInstanceBinding(
                GraphNodeId::new(2)
            ))
        );
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                initial_root.clone(),
                vec![initial_component.clone()],
                vec![GraphComponentInstance::new(
                    GraphNodeId::new(2),
                    Digest([9; 32]),
                )],
            ),
            Err(GraphHierarchyError::UnknownComponent(Digest([9; 32])))
        );
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                initial_root,
                vec![initial_component],
                vec![
                    GraphComponentInstance::new(GraphNodeId::new(2), digest),
                    GraphComponentInstance::new(GraphNodeId::new(2), digest),
                ],
            ),
            Err(GraphHierarchyError::DuplicateInstance(GraphNodeId::new(2)))
        );

        let component = component();
        let digest = encode_graph_component(&component).unwrap().digest();
        let root = root(&component);
        let instance = root.graph().node(GraphNodeId::new(2)).unwrap();
        let wrong_shape = NodeDefinition::new(
            instance.id(),
            instance.kind().clone(),
            instance.label(),
            instance.domain(),
            instance.inputs().to_vec(),
            vec![PortDefinition::new(
                instance.outputs()[0].id(),
                "wrong_output_name",
                instance.outputs()[0].value_type(),
            )],
            Vec::new(),
        );
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                root_with_replacement(&root, wrong_shape),
                vec![component.clone()],
                vec![GraphComponentInstance::new(GraphNodeId::new(2), digest)],
            ),
            Err(GraphHierarchyError::InstanceShapeMismatch {
                node: GraphNodeId::new(2),
                aspect: "outputs",
            })
        );

        let unsupported = NodeDefinition::new(
            instance.id(),
            NodeKind::new(GRAPH_COMPONENT_INSTANCE_KIND, 2),
            instance.label(),
            instance.domain(),
            instance.inputs().to_vec(),
            instance.outputs().to_vec(),
            Vec::new(),
        );
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                root_with_replacement(&root, unsupported),
                vec![component],
                Vec::new(),
            ),
            Err(GraphHierarchyError::UnsupportedInstanceVersion {
                node: GraphNodeId::new(2),
                version: 2,
            })
        );
    }

    #[test]
    fn dependency_order_is_canonical_and_nested_leaf_instances_reject() {
        let first = component();
        let first_digest = encode_graph_component(&first).unwrap().digest();
        let root = root(&first);
        let instances = vec![GraphComponentInstance::new(
            GraphNodeId::new(2),
            first_digest,
        )];
        let mut second = first.clone();
        let mut moved = second.workspace().clone();
        moved.move_node(GraphNodeId::new(2), 777, 888).unwrap();
        second.replace_workspace(moved).unwrap();
        let left = GraphHierarchyDocument::try_new(
            GraphHierarchyLimits::interactive(),
            1,
            root.clone(),
            vec![first.clone(), second.clone()],
            instances.clone(),
        )
        .unwrap();
        let right = GraphHierarchyDocument::try_new(
            GraphHierarchyLimits::interactive(),
            1,
            root.clone(),
            vec![second, first.clone()],
            instances,
        )
        .unwrap();
        assert_eq!(left, right);
        assert_eq!(
            encode_graph_hierarchy(&left).unwrap(),
            encode_graph_hierarchy(&right).unwrap()
        );

        let mut nested = first;
        let mut nested_workspace = nested.workspace().clone();
        nested_workspace
            .create_node(
                GraphNodePrototype::new(
                    NodeKind::new(
                        GRAPH_COMPONENT_INSTANCE_KIND,
                        GRAPH_COMPONENT_INSTANCE_VERSION,
                    ),
                    "Forbidden nested instance",
                    ExecutionDomain::HostExact,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                10,
                10,
            )
            .unwrap();
        nested.replace_workspace(nested_workspace).unwrap();
        let nested_digest = encode_graph_component(&nested).unwrap().digest();
        let nested_node = nested
            .workspace()
            .graph()
            .nodes()
            .iter()
            .find(|node| node.kind().name() == GRAPH_COMPONENT_INSTANCE_KIND)
            .unwrap()
            .id();
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                root,
                vec![nested],
                vec![GraphComponentInstance::new(
                    GraphNodeId::new(2),
                    nested_digest,
                )],
            ),
            Err(GraphHierarchyError::NestedComponentInstance {
                component: nested_digest,
                node: nested_node,
            })
        );
    }

    #[test]
    fn flattened_limits_and_outer_replay_fail_closed() {
        let hierarchy = hierarchy();
        let mut limits = GraphHierarchyLimits::interactive();
        limits.maximum_flattened_nodes = 19;
        assert_eq!(
            GraphHierarchyDocument::try_new(
                limits,
                hierarchy.revision(),
                hierarchy.root().clone(),
                hierarchy
                    .dependencies()
                    .iter()
                    .map(|dependency| dependency.document().clone())
                    .collect(),
                hierarchy.instances().to_vec(),
            ),
            Err(GraphHierarchyError::LimitExceeded("flattened node count"))
        );

        let encoding = encode_graph_hierarchy(&hierarchy).unwrap();
        let mut bad_magic = encoding.bytes().to_vec();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            replay_graph_hierarchy(
                &bad_magic,
                GraphHierarchyLimits::interactive(),
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            ),
            Err(GraphHierarchyError::InvalidMagic)
        );
        let mut trailing = encoding.bytes().to_vec();
        trailing.push(0);
        assert_eq!(
            replay_graph_hierarchy(
                &trailing,
                GraphHierarchyLimits::interactive(),
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            ),
            Err(GraphHierarchyError::TrailingBytes)
        );
    }

    #[test]
    fn component_clock_context_must_match_before_flattening() {
        let component = component();
        let digest = encode_graph_component(&component).unwrap().digest();
        let root = root(&component);
        let mut clocks = root.graph().clocks().to_vec();
        clocks[0] = ClockDefinition::new(
            GraphClockId::new(1),
            "host.root",
            ClockKind::HostMonotonic {
                ticks_per_second: 200,
            },
        );
        let graph = GraphDocument::try_new(
            root.graph().revision(),
            root.graph().schema().clone(),
            clocks,
            root.graph().nodes().to_vec(),
            root.graph().wires().to_vec(),
        )
        .unwrap();
        let changed_root = GraphWorkspaceDocument::try_new(
            root.limits(),
            root.revision(),
            root.next_node_id(),
            root.next_wire_id(),
            graph,
            root.placements().to_vec(),
        )
        .unwrap();
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                changed_root,
                vec![component],
                vec![GraphComponentInstance::new(GraphNodeId::new(2), digest)],
            ),
            Err(GraphHierarchyError::SemanticContextMismatch(digest))
        );
    }

    #[test]
    fn a_wire_between_instances_is_resolved_at_both_endpoints() {
        let component = chain_component();
        let digest = encode_graph_component(&component).unwrap().digest();
        let fixture = compile_representative_exact_control_graph().unwrap();
        let prototype = graph_component_instance_prototype(&component, "Chain instance").unwrap();
        let instance_node = |id, label| {
            NodeDefinition::new(
                GraphNodeId::new(id),
                prototype.kind().clone(),
                label,
                prototype.domain(),
                prototype.inputs().to_vec(),
                prototype.outputs().to_vec(),
                Vec::new(),
            )
        };
        let graph = GraphDocument::try_new(
            1,
            fixture.document().schema().clone(),
            fixture.document().clocks().to_vec(),
            vec![
                instance_node(1, "First chain"),
                instance_node(2, "Second chain"),
            ],
            vec![WireDefinition::new(
                GraphWireId::new(1),
                endpoint(1, 3),
                endpoint(2, 1),
            )],
        )
        .unwrap();
        let root = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            3,
            2,
            graph,
            vec![
                GraphNodePlacement::new(GraphNodeId::new(1), 0, 0),
                GraphNodePlacement::new(GraphNodeId::new(2), 900, 0),
            ],
        )
        .unwrap();
        let hierarchy = GraphHierarchyDocument::try_new(
            GraphHierarchyLimits::interactive(),
            1,
            root,
            vec![component],
            vec![
                GraphComponentInstance::new(GraphNodeId::new(1), digest),
                GraphComponentInstance::new(GraphNodeId::new(2), digest),
            ],
        )
        .unwrap();
        assert_eq!(hierarchy.flattened_node_count(), 36);
        assert_eq!(hierarchy.flattened_wire_count(), 43);
        let flattened = flatten_graph_hierarchy(&hierarchy).unwrap();
        let first_source = flattened.instances()[0]
            .nodes()
            .iter()
            .find(|mapping| mapping.component_node() == GraphNodeId::new(2))
            .unwrap()
            .flattened_node();
        let second_target = flattened.instances()[1]
            .nodes()
            .iter()
            .find(|mapping| mapping.component_node() == GraphNodeId::new(4))
            .unwrap()
            .flattened_node();
        assert!(flattened.workspace().graph().wires().iter().any(|wire| {
            wire.source()
                == WireEndpoint {
                    node: first_source,
                    port: GraphPortId::new(1),
                }
                && wire.target()
                    == WireEndpoint {
                        node: second_target,
                        port: GraphPortId::new(1),
                    }
        }));
    }
}
