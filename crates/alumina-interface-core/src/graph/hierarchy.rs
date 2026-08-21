//! Canonical component-library bindings and deterministic hierarchy flattening.
//!
//! `ALGH` binds authoring-only instance nodes in a root `ALGW` and canonical
//! `ALGC` dependencies to exact component identities. The resulting dependency
//! graph must be acyclic and bounded. Flattening recursively removes every
//! instance node and produces an ordinary workspace containing only structural
//! `ALGR` nodes and wires. Neither the hierarchy package nor flattening admits
//! node semantics, implementations, resources, timing, or deployment.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::{
    CanonicalGraphComponentEncoding, CanonicalGraphWorkspaceEncoding, ExecutionDomain,
    GraphComponentDocument, GraphComponentError, GraphComponentInputId, GraphComponentLimits,
    GraphComponentOutputId, GraphLimits, GraphNodeId, GraphNodePrototype, GraphPortId, GraphWireId,
    GraphWorkspaceDocument, GraphWorkspaceError, GraphWorkspaceLimits, NodeKind, PortDefinition,
    WireEndpoint, encode_graph_component, encode_graph_workspace, replay_graph_component,
    replay_graph_workspace,
};

/// Magic bytes at the beginning of each canonical graph hierarchy.
pub const GRAPH_HIERARCHY_MAGIC: [u8; 4] = *b"ALGH";

/// Exact canonical graph-hierarchy format implemented by this source tree.
pub const GRAPH_HIERARCHY_VERSION: u16 = 2;

/// Reserved authoring-only node kind used for component instances before
/// flattening. It is never an executable semantic or firmware opcode kind.
pub const GRAPH_COMPONENT_INSTANCE_KIND: &str = "alumina.component.instance";

/// Version of the reserved authoring-only instance shape.
pub const GRAPH_COMPONENT_INSTANCE_VERSION: u16 = 1;

const GRAPH_HIERARCHY_FLAGS: u16 = 0;
const HIERARCHY_LIMIT_FIELD_COUNT: usize = 7;

/// Caller-owned and embedded bounds for one hierarchy and its flattened result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphHierarchyLimits {
    /// Maximum complete canonical `ALGH` bytes, including root and dependencies.
    pub maximum_hierarchy_bytes: usize,
    /// Maximum distinct canonical component dependencies.
    pub maximum_components: usize,
    /// Maximum scoped instance bindings retained in the package.
    pub maximum_instance_bindings: usize,
    /// Maximum recursively expanded instance path depth.
    pub maximum_nesting_depth: usize,
    /// Maximum component occurrences expanded into the root workspace.
    pub maximum_expanded_instances: usize,
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
            maximum_instance_bindings: 4_096,
            maximum_nesting_depth: 32,
            maximum_expanded_instances: 4_096,
            maximum_flattened_nodes: 4_096,
            maximum_flattened_wires: 8_192,
        }
    }

    fn validate(self) -> Result<(), GraphHierarchyError> {
        if self.maximum_hierarchy_bytes == 0
            || self.maximum_components == 0
            || self.maximum_instance_bindings == 0
            || self.maximum_nesting_depth == 0
            || self.maximum_expanded_instances == 0
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

/// Workspace scope containing one authoring-only component instance.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GraphInstanceScope {
    /// Top-level hierarchy workspace.
    Root,
    /// Canonical component workspace named by its exact `ALGC` digest.
    Component(Digest),
}

/// Exact binding from one scoped authoring node to a canonical component digest.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GraphComponentInstance {
    scope: GraphInstanceScope,
    node: GraphNodeId,
    component: Digest,
}

impl GraphComponentInstance {
    /// Bind one root-workspace placeholder to an exact component.
    pub const fn root(node: GraphNodeId, component: Digest) -> Self {
        Self {
            scope: GraphInstanceScope::Root,
            node,
            component,
        }
    }

    /// Bind one placeholder inside an exact parent component.
    pub const fn nested(parent: Digest, node: GraphNodeId, component: Digest) -> Self {
        Self {
            scope: GraphInstanceScope::Component(parent),
            node,
            component,
        }
    }

    /// Return the exact workspace containing the placeholder node.
    pub const fn scope(self) -> GraphInstanceScope {
        self.scope
    }

    /// Return the placeholder identity inside [`Self::scope`].
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
    flattened_instances: usize,
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
        if instances.len() > limits.maximum_instance_bindings {
            return Err(GraphHierarchyError::LimitExceeded("instance binding count"));
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
        instances.sort_unstable();
        let (flattened_instances, flattened_nodes, flattened_wires) =
            validate_hierarchy(&root, &dependencies, &instances, limits)?;
        Ok(Self {
            limits,
            revision,
            root,
            root_digest,
            dependencies,
            instances,
            flattened_instances,
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

    /// Borrow bindings in canonical scope/node/component order.
    pub fn instances(&self) -> &[GraphComponentInstance] {
        &self.instances
    }

    /// Return the statically proved number of recursively expanded occurrences.
    pub const fn flattened_instance_count(&self) -> usize {
        self.flattened_instances
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

    /// Transactionally retain one additional exact component dependency. An
    /// encoding already present in the library is an exact no-op.
    pub fn add_component(
        &mut self,
        component: GraphComponentDocument,
    ) -> Result<Digest, GraphHierarchyError> {
        let encoding = encode_graph_component(&component)?;
        let digest = encoding.digest();
        if let Some(existing) = self.dependency(digest) {
            if existing.encoding == encoding {
                return Ok(digest);
            }
            return Err(GraphHierarchyError::NonCanonical);
        }
        let mut components = self.component_documents();
        components.push(component);
        let candidate = Self::try_new(
            self.limits,
            self.next_revision()?,
            self.root.clone(),
            components,
            self.instances.clone(),
        )?;
        *self = candidate;
        Ok(digest)
    }

    /// Transactionally remove one unreferenced exact dependency. A component
    /// remains in use when any binding names it as either containing scope or
    /// child component.
    pub fn remove_component(&mut self, component: Digest) -> Result<(), GraphHierarchyError> {
        let index = self
            .dependencies
            .binary_search_by_key(&component, GraphHierarchyDependency::digest)
            .map_err(|_| GraphHierarchyError::UnknownComponent(component))?;
        if self.instances.iter().any(|instance| {
            instance.component == component
                || instance.scope == GraphInstanceScope::Component(component)
        }) {
            return Err(GraphHierarchyError::ComponentInUse(component));
        }
        let mut components = self.component_documents();
        components.remove(index);
        let candidate = Self::try_new(
            self.limits,
            self.next_revision()?,
            self.root.clone(),
            components,
            self.instances.clone(),
        )?;
        *self = candidate;
        Ok(())
    }

    /// Transactionally replace one exact component dependency and remap every
    /// binding that names its old digest. Existing placeholder shapes must
    /// remain valid under the replacement's public connector pane.
    pub fn replace_component(
        &mut self,
        original: Digest,
        replacement: GraphComponentDocument,
    ) -> Result<Digest, GraphHierarchyError> {
        let index = self
            .dependencies
            .binary_search_by_key(&original, GraphHierarchyDependency::digest)
            .map_err(|_| GraphHierarchyError::UnknownComponent(original))?;
        let replacement_encoding = encode_graph_component(&replacement)?;
        let replacement_digest = replacement_encoding.digest();
        if replacement_digest == original {
            if replacement_encoding == self.dependencies[index].encoding {
                return Ok(original);
            }
            return Err(GraphHierarchyError::NonCanonical);
        }
        let mut components = self.component_documents();
        components[index] = replacement;
        let instances = self
            .instances
            .iter()
            .map(|instance| GraphComponentInstance {
                scope: match instance.scope {
                    GraphInstanceScope::Component(parent) if parent == original => {
                        GraphInstanceScope::Component(replacement_digest)
                    }
                    scope => scope,
                },
                node: instance.node,
                component: if instance.component == original {
                    replacement_digest
                } else {
                    instance.component
                },
            })
            .collect();
        let candidate = Self::try_new(
            self.limits,
            self.next_revision()?,
            self.root.clone(),
            components,
            instances,
        )?;
        *self = candidate;
        Ok(replacement_digest)
    }

    /// Transactionally append one root-workspace component instance using the
    /// root workspace's monotonic node allocator.
    pub fn add_root_instance(
        &mut self,
        component: Digest,
        label: impl Into<String>,
        x: i32,
        y: i32,
    ) -> Result<GraphNodeId, GraphHierarchyError> {
        let dependency = self
            .dependency(component)
            .ok_or(GraphHierarchyError::UnknownComponent(component))?
            .document
            .clone();
        let prototype = graph_component_instance_prototype(&dependency, label)?;
        let mut root = self.root.clone();
        let node = root.create_node(prototype, x, y)?;
        let mut instances = self.instances.clone();
        instances.push(GraphComponentInstance::root(node, component));
        let candidate = Self::try_new(
            self.limits,
            self.next_revision()?,
            root,
            self.component_documents(),
            instances,
        )?;
        *self = candidate;
        Ok(node)
    }

    /// Transactionally remove one root-workspace component instance and all
    /// of its root-level incident wires. Component library dependencies remain
    /// available for other occurrences.
    pub fn remove_root_instance(
        &mut self,
        node: GraphNodeId,
    ) -> Result<usize, GraphHierarchyError> {
        if !self
            .instances
            .iter()
            .any(|instance| instance.scope == GraphInstanceScope::Root && instance.node == node)
        {
            return Err(GraphHierarchyError::UnknownInstanceNode {
                scope: GraphInstanceScope::Root,
                node,
            });
        }
        let mut root = self.root.clone();
        let removed_wires = root.delete_node(node)?;
        let instances = self
            .instances
            .iter()
            .copied()
            .filter(|instance| {
                !(instance.scope == GraphInstanceScope::Root && instance.node == node)
            })
            .collect();
        let candidate = Self::try_new(
            self.limits,
            self.next_revision()?,
            root,
            self.component_documents(),
            instances,
        )?;
        *self = candidate;
        Ok(removed_wires)
    }

    /// Transactionally move one bound root component placeholder on the exact
    /// integer authoring canvas.
    pub fn move_root_instance(
        &mut self,
        node: GraphNodeId,
        x: i32,
        y: i32,
    ) -> Result<(), GraphHierarchyError> {
        if !self
            .instances
            .iter()
            .any(|instance| instance.scope == GraphInstanceScope::Root && instance.node == node)
        {
            return Err(GraphHierarchyError::UnknownInstanceNode {
                scope: GraphInstanceScope::Root,
                node,
            });
        }
        if self
            .root
            .placement(node)
            .is_some_and(|placement| placement.x() == x && placement.y() == y)
        {
            return Ok(());
        }
        let mut root = self.root.clone();
        root.move_node(node, x, y)?;
        let candidate = Self::try_new(
            self.limits,
            self.next_revision()?,
            root,
            self.component_documents(),
            self.instances.clone(),
        )?;
        *self = candidate;
        Ok(())
    }

    /// Transactionally add one typed wire inside the root authoring workspace.
    pub fn connect_root_wire(
        &mut self,
        source: WireEndpoint,
        target: WireEndpoint,
    ) -> Result<GraphWireId, GraphHierarchyError> {
        let mut root = self.root.clone();
        let wire = root.connect(source, target)?;
        let candidate = Self::try_new(
            self.limits,
            self.next_revision()?,
            root,
            self.component_documents(),
            self.instances.clone(),
        )?;
        *self = candidate;
        Ok(wire)
    }

    /// Transactionally remove one exact wire from the root authoring
    /// workspace without rewinding its monotonic wire cursor.
    pub fn disconnect_root_wire(&mut self, wire: GraphWireId) -> Result<(), GraphHierarchyError> {
        let mut root = self.root.clone();
        root.disconnect(wire)?;
        let candidate = Self::try_new(
            self.limits,
            self.next_revision()?,
            root,
            self.component_documents(),
            self.instances.clone(),
        )?;
        *self = candidate;
        Ok(())
    }

    fn component_documents(&self) -> Vec<GraphComponentDocument> {
        self.dependencies
            .iter()
            .map(|dependency| dependency.document.clone())
            .collect()
    }

    fn next_revision(&self) -> Result<u64, GraphHierarchyError> {
        self.revision
            .checked_add(1)
            .ok_or(GraphHierarchyError::IntegerOverflow("revision"))
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

/// Stable authoring origin of one final flattened node.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GraphHierarchyNodeOrigin {
    /// Ordinary node retained directly from the root workspace.
    Root(GraphNodeId),
    /// Surviving node copied from one exact component occurrence.
    Component {
        /// Root instance ID followed by nested component-local instance IDs.
        source_path: Vec<GraphNodeId>,
        /// Exact component definition containing `node`.
        component: Digest,
        /// Component-local node identity.
        node: GraphNodeId,
    },
}

impl GraphHierarchyNodeOrigin {
    /// Borrow the component occurrence path, or return `None` for a root node.
    pub fn source_path(&self) -> Option<&[GraphNodeId]> {
        match self {
            Self::Root(_) => None,
            Self::Component { source_path, .. } => Some(source_path),
        }
    }
}

/// Total provenance record for one node in the final flattened workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphFlattenedNodeProvenance {
    origin: GraphHierarchyNodeOrigin,
    flattened_node: GraphNodeId,
}

impl GraphFlattenedNodeProvenance {
    /// Borrow the exact root or component authoring origin.
    pub const fn origin(&self) -> &GraphHierarchyNodeOrigin {
        &self.origin
    }

    /// Return the final monotonic root-workspace node identity.
    pub const fn flattened_node(&self) -> GraphNodeId {
        self.flattened_node
    }
}

/// Stable authoring origin of one final flattened wire.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GraphHierarchyWireOrigin {
    /// Wire authored directly in the root workspace.
    Root(GraphWireId),
    /// Wire copied from one exact component occurrence.
    Component {
        /// Root instance ID followed by nested component-local instance IDs.
        source_path: Vec<GraphNodeId>,
        /// Exact component definition containing `wire`.
        component: Digest,
        /// Component-local wire identity.
        wire: GraphWireId,
    },
}

impl GraphHierarchyWireOrigin {
    /// Borrow the component occurrence path, or return `None` for a root wire.
    pub fn source_path(&self) -> Option<&[GraphNodeId]> {
        match self {
            Self::Root(_) => None,
            Self::Component { source_path, .. } => Some(source_path),
        }
    }
}

/// Total provenance record for one wire in the final flattened workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphFlattenedWireProvenance {
    origin: GraphHierarchyWireOrigin,
    flattened_wire: GraphWireId,
}

impl GraphFlattenedWireProvenance {
    /// Borrow the exact root or component authoring origin.
    pub const fn origin(&self) -> &GraphHierarchyWireOrigin {
        &self.origin
    }

    /// Return the final monotonic root-workspace wire identity.
    pub const fn flattened_wire(&self) -> GraphWireId {
        self.flattened_wire
    }
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
    source_path: Vec<GraphNodeId>,
    component: Digest,
    nodes: Vec<GraphFlattenedNode>,
}

impl GraphFlattenedInstance {
    /// Return the removed root or transient flattened instance identity.
    /// Prefer [`Self::source_path`] as the stable authoring identity.
    pub const fn instance_node(&self) -> GraphNodeId {
        self.instance_node
    }

    /// Borrow the stable source path: root node followed by nested local nodes.
    pub fn source_path(&self) -> &[GraphNodeId] {
        &self.source_path
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
    node_provenance: Vec<GraphFlattenedNodeProvenance>,
    wire_provenance: Vec<GraphFlattenedWireProvenance>,
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

    /// Borrow mappings in deterministic depth-first source-path order.
    pub fn instances(&self) -> &[GraphFlattenedInstance] {
        &self.instances
    }

    /// Borrow total node provenance in final flattened-node order.
    pub fn node_provenance(&self) -> &[GraphFlattenedNodeProvenance] {
        &self.node_provenance
    }

    /// Borrow total wire provenance in final flattened-wire order.
    pub fn wire_provenance(&self) -> &[GraphFlattenedWireProvenance] {
        &self.wire_provenance
    }

    /// Resolve one final node to its stable authoring origin.
    pub fn node_origin(&self, node: GraphNodeId) -> Option<&GraphHierarchyNodeOrigin> {
        self.node_provenance
            .binary_search_by_key(&node, GraphFlattenedNodeProvenance::flattened_node)
            .ok()
            .map(|index| self.node_provenance[index].origin())
    }

    /// Resolve one final wire to its stable authoring origin.
    pub fn wire_origin(&self, wire: GraphWireId) -> Option<&GraphHierarchyWireOrigin> {
        self.wire_provenance
            .binary_search_by_key(&wire, GraphFlattenedWireProvenance::flattened_wire)
            .ok()
            .map(|index| self.wire_provenance[index].origin())
    }

    /// Resolve one stable authoring origin to its final node identity.
    pub fn flattened_node(&self, origin: &GraphHierarchyNodeOrigin) -> Option<GraphNodeId> {
        self.node_provenance
            .iter()
            .find(|mapping| mapping.origin() == origin)
            .map(GraphFlattenedNodeProvenance::flattened_node)
    }

    /// Resolve one stable authoring origin to its final wire identity.
    pub fn flattened_wire(&self, origin: &GraphHierarchyWireOrigin) -> Option<GraphWireId> {
        self.wire_provenance
            .iter()
            .find(|mapping| mapping.origin() == origin)
            .map(GraphFlattenedWireProvenance::flattened_wire)
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
    /// A requested dependency removal still had a parent-scope or child binding.
    ComponentInUse(Digest),
    /// A nested binding named a parent component absent from the library.
    UnknownInstanceScope(Digest),
    /// Two bindings referenced the same node in one scope.
    DuplicateInstance {
        /// Exact containing workspace.
        scope: GraphInstanceScope,
        /// Duplicated placeholder node.
        node: GraphNodeId,
    },
    /// An instance binding referenced no node in its scope.
    UnknownInstanceNode {
        /// Exact containing workspace.
        scope: GraphInstanceScope,
        /// Unknown placeholder node.
        node: GraphNodeId,
    },
    /// A binding referenced a normal node rather than the reserved instance kind.
    NotComponentInstance {
        /// Exact containing workspace.
        scope: GraphInstanceScope,
        /// Non-instance node.
        node: GraphNodeId,
    },
    /// The reserved instance name used a version this hierarchy cannot derive.
    UnsupportedInstanceVersion {
        /// Exact containing workspace.
        scope: GraphInstanceScope,
        /// Scoped authoring node.
        node: GraphNodeId,
        /// Unsupported structural version.
        version: u16,
    },
    /// A reserved authoring instance node had no exact dependency binding.
    MissingInstanceBinding {
        /// Exact containing workspace.
        scope: GraphInstanceScope,
        /// Unbound placeholder node.
        node: GraphNodeId,
    },
    /// A reserved instance node contradicted its component-derived shape.
    InstanceShapeMismatch {
        /// Exact containing workspace.
        scope: GraphInstanceScope,
        /// Scoped instance node.
        node: GraphNodeId,
        /// Mismatched shape collection.
        aspect: &'static str,
    },
    /// A component type registry or clock set differed from the root authority.
    SemanticContextMismatch(Digest),
    /// Component dependency edges contain a recursive cycle.
    DependencyCycle(Digest),
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
            Self::ComponentInUse(digest) => write!(
                formatter,
                "graph hierarchy component {digest:?} is still referenced"
            ),
            Self::UnknownInstanceScope(component) => write!(
                formatter,
                "graph hierarchy instance scope component {component:?} is unknown"
            ),
            Self::DuplicateInstance { scope, node } => {
                write!(
                    formatter,
                    "graph hierarchy instance node {node:?} is duplicated in {scope:?}"
                )
            }
            Self::UnknownInstanceNode { scope, node } => {
                write!(
                    formatter,
                    "graph hierarchy instance node {node:?} is unknown in {scope:?}"
                )
            }
            Self::NotComponentInstance { scope, node } => {
                write!(
                    formatter,
                    "graph hierarchy node {node:?} in {scope:?} is not a component instance"
                )
            }
            Self::UnsupportedInstanceVersion {
                scope,
                node,
                version,
            } => write!(
                formatter,
                "graph hierarchy instance node {node:?} in {scope:?} uses unsupported version {version}"
            ),
            Self::MissingInstanceBinding { scope, node } => {
                write!(
                    formatter,
                    "graph hierarchy instance node {node:?} in {scope:?} has no binding"
                )
            }
            Self::InstanceShapeMismatch {
                scope,
                node,
                aspect,
            } => write!(
                formatter,
                "graph hierarchy instance node {node:?} in {scope:?} has mismatched {aspect}"
            ),
            Self::SemanticContextMismatch(digest) => write!(
                formatter,
                "graph hierarchy component {digest:?} has a different type or clock context"
            ),
            Self::DependencyCycle(component) => write!(
                formatter,
                "graph hierarchy component dependency cycle re-enters {component:?}"
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
        match instance.scope {
            GraphInstanceScope::Root => encoder.u8(0),
            GraphInstanceScope::Component(component) => {
                encoder.u8(1);
                encoder.bytes(&component.0);
            }
        }
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
    let instance_count =
        decoder.count(limits.maximum_instance_bindings, "instance binding count")?;
    let mut instances = Vec::with_capacity(instance_count);
    for _ in 0..instance_count {
        let scope = match decoder.u8()? {
            0 => GraphInstanceScope::Root,
            1 => {
                let digest: [u8; 32] = decoder
                    .take(32)?
                    .try_into()
                    .map_err(|_| GraphHierarchyError::Truncated)?;
                GraphInstanceScope::Component(Digest(digest))
            }
            _ => return Err(GraphHierarchyError::NonCanonical),
        };
        let node = GraphNodeId::new(decoder.u32()?);
        let digest: [u8; 32] = decoder
            .take(32)?
            .try_into()
            .map_err(|_| GraphHierarchyError::Truncated)?;
        instances.push(match scope {
            GraphInstanceScope::Root => GraphComponentInstance::root(node, Digest(digest)),
            GraphInstanceScope::Component(parent) => {
                GraphComponentInstance::nested(parent, node, Digest(digest))
            }
        });
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
    let mut reports = Vec::with_capacity(hierarchy.flattened_instances);
    let mut wire_origins = workspace
        .graph()
        .wires()
        .iter()
        .map(|wire| (wire.id(), GraphHierarchyWireOrigin::Root(wire.id())))
        .collect::<BTreeMap<_, _>>();
    for instance in hierarchy
        .instances
        .iter()
        .copied()
        .filter(|instance| instance.scope == GraphInstanceScope::Root)
    {
        flatten_instance_tree(
            &mut workspace,
            hierarchy,
            instance.node,
            instance,
            vec![instance.node],
            &mut reports,
            &mut wire_origins,
        )?;
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
    if reports.len() != hierarchy.flattened_instances {
        return Err(GraphHierarchyError::NonCanonical);
    }
    if wire_origins.len() != workspace.graph().wires().len()
        || !workspace
            .graph()
            .wires()
            .iter()
            .all(|wire| wire_origins.contains_key(&wire.id()))
    {
        return Err(GraphHierarchyError::NonCanonical);
    }
    let mut node_origins = hierarchy
        .root
        .graph()
        .nodes()
        .iter()
        .filter(|node| !has_component_instance_name(node))
        .map(|node| (node.id(), GraphHierarchyNodeOrigin::Root(node.id())))
        .collect::<BTreeMap<_, _>>();
    for report in &reports {
        for mapping in report.nodes() {
            if node_origins
                .insert(
                    mapping.flattened_node(),
                    GraphHierarchyNodeOrigin::Component {
                        source_path: report.source_path.clone(),
                        component: report.component,
                        node: mapping.component_node(),
                    },
                )
                .is_some()
            {
                return Err(GraphHierarchyError::NonCanonical);
            }
        }
    }
    if node_origins.len() != workspace.graph().nodes().len()
        || !workspace
            .graph()
            .nodes()
            .iter()
            .all(|node| node_origins.contains_key(&node.id()))
    {
        return Err(GraphHierarchyError::NonCanonical);
    }
    let node_provenance = node_origins
        .into_iter()
        .map(|(flattened_node, origin)| GraphFlattenedNodeProvenance {
            origin,
            flattened_node,
        })
        .collect();
    let wire_provenance = wire_origins
        .into_iter()
        .map(|(flattened_wire, origin)| GraphFlattenedWireProvenance {
            origin,
            flattened_wire,
        })
        .collect();
    let encoding = encode_graph_workspace(&workspace)?;
    Ok(GraphHierarchyFlattening {
        source_digest,
        workspace,
        encoding,
        instances: reports,
        node_provenance,
        wire_provenance,
    })
}

fn flatten_instance_tree(
    root: &mut GraphWorkspaceDocument,
    hierarchy: &GraphHierarchyDocument,
    instance_node: GraphNodeId,
    binding: GraphComponentInstance,
    source_path: Vec<GraphNodeId>,
    reports: &mut Vec<GraphFlattenedInstance>,
    wire_origins: &mut BTreeMap<GraphWireId, GraphHierarchyWireOrigin>,
) -> Result<(), GraphHierarchyError> {
    if source_path.len() > hierarchy.limits.maximum_nesting_depth {
        return Err(GraphHierarchyError::LimitExceeded("nesting depth"));
    }
    let dependency = hierarchy
        .dependency(binding.component)
        .ok_or(GraphHierarchyError::UnknownComponent(binding.component))?;
    let copied = flatten_instance(
        root,
        instance_node,
        binding.component,
        source_path.clone(),
        dependency.document(),
        wire_origins,
    )?;
    let node_map = copied.node_map;
    reports.push(copied.report);
    for nested in hierarchy
        .instances
        .iter()
        .copied()
        .filter(|instance| instance.scope == GraphInstanceScope::Component(binding.component))
    {
        let nested_node = *node_map
            .get(&nested.node)
            .ok_or(GraphHierarchyError::NonCanonical)?;
        let mut nested_path = source_path.clone();
        nested_path.push(nested.node);
        flatten_instance_tree(
            root,
            hierarchy,
            nested_node,
            nested,
            nested_path,
            reports,
            wire_origins,
        )?;
    }
    Ok(())
}

fn validate_hierarchy(
    root: &GraphWorkspaceDocument,
    dependencies: &[GraphHierarchyDependency],
    instances: &[GraphComponentInstance],
    limits: GraphHierarchyLimits,
) -> Result<(usize, usize, usize), GraphHierarchyError> {
    for dependency in dependencies {
        validate_dependency_context(root, dependency)?;
    }

    let mut prior_instance = None;
    let mut bound_nodes = BTreeSet::new();
    for instance in instances {
        let key = (instance.scope, instance.node);
        if instance.node.get() == 0 {
            return Err(GraphHierarchyError::UnknownInstanceNode {
                scope: instance.scope,
                node: instance.node,
            });
        }
        if prior_instance == Some(key) {
            return Err(GraphHierarchyError::DuplicateInstance {
                scope: instance.scope,
                node: instance.node,
            });
        }
        prior_instance = Some(key);
        let workspace = workspace_for_scope(root, dependencies, instance.scope)?;
        let node = workspace.graph().node(instance.node).ok_or(
            GraphHierarchyError::UnknownInstanceNode {
                scope: instance.scope,
                node: instance.node,
            },
        )?;
        if !has_component_instance_name(node) {
            return Err(GraphHierarchyError::NotComponentInstance {
                scope: instance.scope,
                node: instance.node,
            });
        }
        if node.kind().version() != GRAPH_COMPONENT_INSTANCE_VERSION {
            return Err(GraphHierarchyError::UnsupportedInstanceVersion {
                scope: instance.scope,
                node: instance.node,
                version: node.kind().version(),
            });
        }
        let dependency = dependency_by_digest(dependencies, instance.component)
            .ok_or(GraphHierarchyError::UnknownComponent(instance.component))?;
        validate_instance_node(instance.scope, node, dependency.document())?;
        bound_nodes.insert(key);
    }

    validate_scope_placeholders(GraphInstanceScope::Root, root, &bound_nodes)?;
    for dependency in dependencies {
        validate_scope_placeholders(
            GraphInstanceScope::Component(dependency.digest()),
            dependency.document().workspace(),
            &bound_nodes,
        )?;
    }

    let mut memo = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    for dependency in dependencies {
        let expansion = component_expansion(
            dependency.digest(),
            dependencies,
            instances,
            &mut memo,
            &mut visiting,
        )?;
        if expansion.depth > limits.maximum_nesting_depth {
            return Err(GraphHierarchyError::LimitExceeded("nesting depth"));
        }
    }

    let root_bindings = instances
        .iter()
        .filter(|instance| instance.scope == GraphInstanceScope::Root)
        .collect::<Vec<_>>();
    let mut flattened_instances = 0_usize;
    let mut flattened_nodes = root
        .graph()
        .nodes()
        .len()
        .checked_sub(root_bindings.len())
        .ok_or(GraphHierarchyError::IntegerOverflow("flattened node count"))?;
    let mut flattened_wires = root.graph().wires().len();
    for instance in root_bindings {
        let expansion = *memo
            .get(&instance.component)
            .ok_or(GraphHierarchyError::UnknownComponent(instance.component))?;
        flattened_instances = flattened_instances.checked_add(expansion.instances).ok_or(
            GraphHierarchyError::IntegerOverflow("expanded instance count"),
        )?;
        flattened_nodes = flattened_nodes
            .checked_add(expansion.nodes)
            .ok_or(GraphHierarchyError::IntegerOverflow("flattened node count"))?;
        flattened_wires = flattened_wires
            .checked_add(expansion.wires)
            .ok_or(GraphHierarchyError::IntegerOverflow("flattened wire count"))?;
    }
    if flattened_instances > limits.maximum_expanded_instances {
        return Err(GraphHierarchyError::LimitExceeded(
            "expanded instance count",
        ));
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
    Ok((flattened_instances, flattened_nodes, flattened_wires))
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
    Ok(())
}

#[derive(Clone, Copy)]
struct ComponentExpansion {
    instances: usize,
    nodes: usize,
    wires: usize,
    depth: usize,
}

fn component_expansion(
    component: Digest,
    dependencies: &[GraphHierarchyDependency],
    instances: &[GraphComponentInstance],
    memo: &mut BTreeMap<Digest, ComponentExpansion>,
    visiting: &mut BTreeSet<Digest>,
) -> Result<ComponentExpansion, GraphHierarchyError> {
    if let Some(expansion) = memo.get(&component) {
        return Ok(*expansion);
    }
    if !visiting.insert(component) {
        return Err(GraphHierarchyError::DependencyCycle(component));
    }
    let dependency = dependency_by_digest(dependencies, component)
        .ok_or(GraphHierarchyError::UnknownComponent(component))?;
    let nested = instances
        .iter()
        .filter(|instance| instance.scope == GraphInstanceScope::Component(component))
        .collect::<Vec<_>>();
    let mut expansion = ComponentExpansion {
        instances: 1,
        nodes: dependency
            .document()
            .workspace()
            .graph()
            .nodes()
            .len()
            .checked_sub(nested.len())
            .ok_or(GraphHierarchyError::IntegerOverflow(
                "component flattened node count",
            ))?,
        wires: dependency.document().workspace().graph().wires().len(),
        depth: 1,
    };
    for instance in nested {
        let child =
            component_expansion(instance.component, dependencies, instances, memo, visiting)?;
        expansion.instances = expansion.instances.checked_add(child.instances).ok_or(
            GraphHierarchyError::IntegerOverflow("expanded instance count"),
        )?;
        expansion.nodes = expansion.nodes.checked_add(child.nodes).ok_or(
            GraphHierarchyError::IntegerOverflow("component flattened node count"),
        )?;
        expansion.wires = expansion.wires.checked_add(child.wires).ok_or(
            GraphHierarchyError::IntegerOverflow("component flattened wire count"),
        )?;
        expansion.depth = expansion.depth.max(
            child
                .depth
                .checked_add(1)
                .ok_or(GraphHierarchyError::IntegerOverflow("nesting depth"))?,
        );
    }
    visiting.remove(&component);
    memo.insert(component, expansion);
    Ok(expansion)
}

fn workspace_for_scope<'a>(
    root: &'a GraphWorkspaceDocument,
    dependencies: &'a [GraphHierarchyDependency],
    scope: GraphInstanceScope,
) -> Result<&'a GraphWorkspaceDocument, GraphHierarchyError> {
    match scope {
        GraphInstanceScope::Root => Ok(root),
        GraphInstanceScope::Component(component) => dependency_by_digest(dependencies, component)
            .map(|dependency| dependency.document().workspace())
            .ok_or(GraphHierarchyError::UnknownInstanceScope(component)),
    }
}

fn validate_scope_placeholders(
    scope: GraphInstanceScope,
    workspace: &GraphWorkspaceDocument,
    bound_nodes: &BTreeSet<(GraphInstanceScope, GraphNodeId)>,
) -> Result<(), GraphHierarchyError> {
    for node in workspace
        .graph()
        .nodes()
        .iter()
        .filter(|node| has_component_instance_name(node))
    {
        if node.kind().version() != GRAPH_COMPONENT_INSTANCE_VERSION {
            return Err(GraphHierarchyError::UnsupportedInstanceVersion {
                scope,
                node: node.id(),
                version: node.kind().version(),
            });
        }
        if !bound_nodes.contains(&(scope, node.id())) {
            return Err(GraphHierarchyError::MissingInstanceBinding {
                scope,
                node: node.id(),
            });
        }
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
    scope: GraphInstanceScope,
    node: &super::NodeDefinition,
    component: &GraphComponentDocument,
) -> Result<(), GraphHierarchyError> {
    let prototype = graph_component_instance_prototype(component, node.label())?;
    if node.kind() != prototype.kind() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            scope,
            node: node.id(),
            aspect: "kind",
        });
    }
    if node.domain() != prototype.domain() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            scope,
            node: node.id(),
            aspect: "domain",
        });
    }
    if node.inputs() != prototype.inputs() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            scope,
            node: node.id(),
            aspect: "inputs",
        });
    }
    if node.outputs() != prototype.outputs() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            scope,
            node: node.id(),
            aspect: "outputs",
        });
    }
    if !node.parameters().is_empty() {
        return Err(GraphHierarchyError::InstanceShapeMismatch {
            scope,
            node: node.id(),
            aspect: "parameters",
        });
    }
    Ok(())
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

struct FlattenedComponentCopy {
    report: GraphFlattenedInstance,
    node_map: BTreeMap<GraphNodeId, GraphNodeId>,
}

fn flatten_instance(
    root: &mut GraphWorkspaceDocument,
    instance_node: GraphNodeId,
    component_digest: Digest,
    source_path: Vec<GraphNodeId>,
    component: &GraphComponentDocument,
    wire_origins: &mut BTreeMap<GraphWireId, GraphHierarchyWireOrigin>,
) -> Result<FlattenedComponentCopy, GraphHierarchyError> {
    let placement = root
        .placement(instance_node)
        .ok_or(GraphHierarchyError::NonCanonical)?;
    let incident_wires = root
        .graph()
        .wires()
        .iter()
        .copied()
        .filter(|wire| wire.source().node == instance_node || wire.target().node == instance_node)
        .collect::<Vec<_>>();
    let mut incident = Vec::with_capacity(incident_wires.len());
    for wire in incident_wires {
        let origin = wire_origins
            .remove(&wire.id())
            .ok_or(GraphHierarchyError::NonCanonical)?;
        incident.push((wire, origin));
    }
    root.delete_node(instance_node)?;

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
            instance_node,
            node.id(),
        )?;
        let y = translated_coordinate(
            placement.y(),
            component_placement.y(),
            origin_y,
            instance_node,
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
        if !has_component_instance_name(node) {
            nodes.push(GraphFlattenedNode {
                component_node: node.id(),
                flattened_node: flattened,
            });
        }
    }
    for wire in component_workspace.graph().wires() {
        let flattened_wire = root.connect(
            remap_component_endpoint(wire.source(), &node_map)?,
            remap_component_endpoint(wire.target(), &node_map)?,
        )?;
        if wire_origins
            .insert(
                flattened_wire,
                GraphHierarchyWireOrigin::Component {
                    source_path: source_path.clone(),
                    component: component_digest,
                    wire: wire.id(),
                },
            )
            .is_some()
        {
            return Err(GraphHierarchyError::NonCanonical);
        }
    }
    for (wire, origin) in incident {
        let source = if wire.source().node == instance_node {
            remap_public_output(component, wire.source(), &node_map)?
        } else {
            wire.source()
        };
        let target = if wire.target().node == instance_node {
            remap_public_input(component, wire.target(), &node_map)?
        } else {
            wire.target()
        };
        let flattened_wire = root.connect(source, target)?;
        if wire_origins.insert(flattened_wire, origin).is_some() {
            return Err(GraphHierarchyError::NonCanonical);
        }
    }
    Ok(FlattenedComponentCopy {
        report: GraphFlattenedInstance {
            instance_node,
            source_path,
            component: component_digest,
            nodes,
        },
        node_map,
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
        limits.maximum_instance_bindings,
        limits.maximum_nesting_depth,
        limits.maximum_expanded_instances,
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
        maximum_instance_bindings: values[2],
        maximum_nesting_depth: values[3],
        maximum_expanded_instances: values[4],
        maximum_flattened_nodes: values[5],
        maximum_flattened_wires: values[6],
    })
}

const fn limits_within(embedded: GraphHierarchyLimits, admission: GraphHierarchyLimits) -> bool {
    embedded.maximum_hierarchy_bytes <= admission.maximum_hierarchy_bytes
        && embedded.maximum_components <= admission.maximum_components
        && embedded.maximum_instance_bindings <= admission.maximum_instance_bindings
        && embedded.maximum_nesting_depth <= admission.maximum_nesting_depth
        && embedded.maximum_expanded_instances <= admission.maximum_expanded_instances
        && embedded.maximum_flattened_nodes <= admission.maximum_flattened_nodes
        && embedded.maximum_flattened_wires <= admission.maximum_flattened_wires
}

#[derive(Default)]
struct Encoder(Vec<u8>);

impl Encoder {
    fn bytes(&mut self, value: &[u8]) {
        self.0.extend_from_slice(value);
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
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

    fn u8(&mut self) -> Result<u8, GraphHierarchyError> {
        Ok(self.take(1)?[0])
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
            22,
            26,
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

    fn wrapper_component(child: &GraphComponentDocument, name: &str) -> GraphComponentDocument {
        let prototype = graph_component_instance_prototype(child, "Nested child").unwrap();
        let instance = NodeDefinition::new(
            GraphNodeId::new(1),
            prototype.kind().clone(),
            "Nested child",
            prototype.domain(),
            prototype.inputs().to_vec(),
            prototype.outputs().to_vec(),
            Vec::new(),
        );
        let graph = GraphDocument::try_new(
            1,
            child.workspace().graph().schema().clone(),
            child.workspace().graph().clocks().to_vec(),
            vec![instance],
            Vec::new(),
        )
        .unwrap();
        let workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            2,
            1,
            graph,
            vec![GraphNodePlacement::new(GraphNodeId::new(1), 20, 20)],
        )
        .unwrap();
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            name,
            2,
            2,
            1,
            workspace,
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "setpoint_samples",
                endpoint(1, 1),
            )],
            vec![GraphComponentOutput::new(
                GraphComponentOutputId::new(1),
                "permitted_output",
                endpoint(1, 2),
            )],
            Vec::new(),
        )
        .unwrap()
    }

    fn wired_wrapper_component(
        child: &GraphComponentDocument,
        name: &str,
    ) -> GraphComponentDocument {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let nested_id = GraphNodeId::new(1);
        let prototype = graph_component_instance_prototype(child, "Nested child").unwrap();
        let nested = NodeDefinition::new(
            nested_id,
            prototype.kind().clone(),
            "Nested child",
            prototype.domain(),
            prototype.inputs().to_vec(),
            prototype.outputs().to_vec(),
            Vec::new(),
        );
        let child_output = &child.outputs()[1];
        let witness = fixture
            .document()
            .wires()
            .iter()
            .find(|wire| wire.source() == child_output.source())
            .unwrap();
        let witness_target = fixture.document().node(witness.target().node).unwrap();
        let target_id = GraphNodeId::new(2);
        let target = NodeDefinition::new(
            target_id,
            witness_target.kind().clone(),
            "Wrapper-local sink",
            witness_target.domain(),
            witness_target.inputs().to_vec(),
            witness_target.outputs().to_vec(),
            witness_target.parameters().to_vec(),
        );
        let nested_output = graph_component_instance_output_port(child, child_output.id()).unwrap();
        let graph = GraphDocument::try_new(
            1,
            child.workspace().graph().schema().clone(),
            child.workspace().graph().clocks().to_vec(),
            vec![nested, target],
            vec![WireDefinition::new(
                GraphWireId::new(1),
                WireEndpoint {
                    node: nested_id,
                    port: nested_output,
                },
                WireEndpoint {
                    node: target_id,
                    port: witness.target().port,
                },
            )],
        )
        .unwrap();
        let workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            3,
            2,
            graph,
            vec![
                GraphNodePlacement::new(nested_id, 20, 20),
                GraphNodePlacement::new(target_id, 400, 20),
            ],
        )
        .unwrap();
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            name,
            2,
            2,
            1,
            workspace,
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "setpoint_samples",
                WireEndpoint {
                    node: nested_id,
                    port: GraphPortId::new(1),
                },
            )],
            vec![GraphComponentOutput::new(
                GraphComponentOutputId::new(1),
                "permitted_output",
                WireEndpoint {
                    node: nested_id,
                    port: graph_component_instance_output_port(
                        child,
                        GraphComponentOutputId::new(1),
                    )
                    .unwrap(),
                },
            )],
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
            vec![GraphComponentInstance::root(GraphNodeId::new(2), digest)],
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
        assert_eq!(hierarchy.flattened_node_count(), 22);
        assert_eq!(hierarchy.flattened_wire_count(), 26);
        assert_eq!(hierarchy.flattened_instance_count(), 1);
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
        assert_eq!(flattened.workspace().graph().nodes().len(), 22);
        assert_eq!(flattened.workspace().graph().wires().len(), 26);
        assert_eq!(flattened.instances().len(), 1);
        assert_eq!(flattened.node_provenance().len(), 22);
        assert_eq!(flattened.wire_provenance().len(), 26);
        assert_eq!(
            flattened.node_origin(GraphNodeId::new(1)),
            Some(&GraphHierarchyNodeOrigin::Root(GraphNodeId::new(1)))
        );
        assert_eq!(
            flattened.instances()[0].source_path(),
            [GraphNodeId::new(2)]
        );
        assert_eq!(flattened.instances()[0].nodes().len(), 20);
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
        let component_origin = GraphHierarchyNodeOrigin::Component {
            source_path: vec![GraphNodeId::new(2)],
            component: hierarchy.dependencies()[0].digest(),
            node: GraphNodeId::new(4),
        };
        assert_eq!(
            flattened.node_origin(remapped_four),
            Some(&component_origin)
        );
        assert_eq!(
            flattened.flattened_node(&component_origin),
            Some(remapped_four)
        );
        let root_wire_origin = GraphHierarchyWireOrigin::Root(GraphWireId::new(1));
        let flattened_root_wire = flattened.flattened_wire(&root_wire_origin).unwrap();
        assert_eq!(
            flattened.wire_origin(flattened_root_wire),
            Some(&root_wire_origin)
        );
        let component_wire_origin = GraphHierarchyWireOrigin::Component {
            source_path: vec![GraphNodeId::new(2)],
            component: hierarchy.dependencies()[0].digest(),
            wire: component.workspace().graph().wires()[0].id(),
        };
        let flattened_component_wire = flattened.flattened_wire(&component_wire_origin).unwrap();
        assert_eq!(
            flattened.wire_origin(flattened_component_wire),
            Some(&component_wire_origin)
        );
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
    fn root_instance_lifecycle_is_transactional_and_never_reuses_node_ids() {
        let mut hierarchy = hierarchy();
        let component = hierarchy.dependencies()[0].digest();
        let original_revision = hierarchy.revision();
        let added = hierarchy
            .add_root_instance(component, "Second PID leaf", 1_200, 100)
            .unwrap();
        assert_eq!(added, GraphNodeId::new(4));
        assert_eq!(hierarchy.revision(), original_revision + 1);
        assert_eq!(hierarchy.instances().len(), 2);
        assert_eq!(hierarchy.flattened_instance_count(), 2);
        assert_eq!(hierarchy.root().next_node_id(), 5);
        assert_eq!(
            hierarchy
                .instances()
                .iter()
                .find(|instance| instance.node() == added),
            Some(&GraphComponentInstance::root(added, component))
        );
        let encoding = encode_graph_hierarchy(&hierarchy).unwrap();
        assert_eq!(
            replay_graph_hierarchy(
                encoding.bytes(),
                GraphHierarchyLimits::interactive(),
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            )
            .unwrap()
            .document(),
            &hierarchy
        );

        assert_eq!(hierarchy.remove_root_instance(added).unwrap(), 0);
        assert_eq!(hierarchy.instances().len(), 1);
        assert_eq!(hierarchy.root().next_node_id(), 5);
        let next = hierarchy
            .add_root_instance(component, "Third PID leaf", 1_200, 100)
            .unwrap();
        assert_eq!(next, GraphNodeId::new(5));

        let retained = hierarchy.clone();
        assert_eq!(
            hierarchy.add_root_instance(Digest([0x91; 32]), "Unknown", 0, 0),
            Err(GraphHierarchyError::UnknownComponent(Digest([0x91; 32])))
        );
        assert_eq!(hierarchy, retained);
        assert_eq!(
            hierarchy.remove_root_instance(GraphNodeId::new(1)),
            Err(GraphHierarchyError::UnknownInstanceNode {
                scope: GraphInstanceScope::Root,
                node: GraphNodeId::new(1),
            })
        );
        assert_eq!(hierarchy, retained);
    }

    #[test]
    fn root_instance_placement_and_typed_wires_are_transactional_and_monotonic() {
        let mut hierarchy = hierarchy();
        let original_revision = hierarchy.revision();
        let original_graph_digest = hierarchy.root().graph_digest();
        let instance = GraphNodeId::new(2);

        hierarchy.move_root_instance(instance, 450, 175).unwrap();
        assert_eq!(hierarchy.revision(), original_revision + 1);
        assert_eq!(
            hierarchy.root().placement(instance),
            Some(GraphNodePlacement::new(instance, 450, 175))
        );
        assert_eq!(hierarchy.root().graph_digest(), original_graph_digest);
        let moved = hierarchy.clone();
        hierarchy.move_root_instance(instance, 450, 175).unwrap();
        assert_eq!(hierarchy, moved, "exact placement no-op advanced state");

        hierarchy.disconnect_root_wire(GraphWireId::new(1)).unwrap();
        assert_eq!(hierarchy.revision(), original_revision + 2);
        assert_eq!(hierarchy.root().next_wire_id(), 3);
        assert_eq!(hierarchy.flattened_wire_count(), 25);
        assert!(
            hierarchy
                .root()
                .graph()
                .wires()
                .iter()
                .all(|wire| wire.id() != GraphWireId::new(1))
        );

        let replacement = hierarchy
            .connect_root_wire(endpoint(1, 1), endpoint(2, 1))
            .unwrap();
        assert_eq!(replacement, GraphWireId::new(3));
        assert_eq!(hierarchy.revision(), original_revision + 3);
        assert_eq!(hierarchy.root().next_wire_id(), 4);
        assert_eq!(hierarchy.flattened_wire_count(), 26);
        assert!(hierarchy.root().graph().wires().iter().any(|wire| {
            wire.id() == replacement
                && wire.source() == endpoint(1, 1)
                && wire.target() == endpoint(2, 1)
        }));
        let encoding = encode_graph_hierarchy(&hierarchy).unwrap();
        assert_eq!(
            replay_graph_hierarchy(
                encoding.bytes(),
                GraphHierarchyLimits::interactive(),
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            )
            .unwrap()
            .document(),
            &hierarchy
        );

        let retained = hierarchy.clone();
        assert!(
            hierarchy
                .connect_root_wire(endpoint(1, 1), endpoint(2, 1))
                .is_err()
        );
        assert_eq!(hierarchy, retained);
        assert!(
            hierarchy
                .disconnect_root_wire(GraphWireId::new(99))
                .is_err()
        );
        assert_eq!(hierarchy, retained);
        assert_eq!(
            hierarchy.move_root_instance(GraphNodeId::new(1), 0, 0),
            Err(GraphHierarchyError::UnknownInstanceNode {
                scope: GraphInstanceScope::Root,
                node: GraphNodeId::new(1),
            })
        );
        assert_eq!(hierarchy, retained);
    }

    #[test]
    fn component_replacement_remaps_exact_bindings_and_preserves_root_authoring() {
        let mut hierarchy = hierarchy();
        let original = hierarchy.dependencies()[0].digest();
        let original_root = hierarchy.root().clone();
        let original_revision = hierarchy.revision();
        let mut replacement = hierarchy.dependencies()[0].document().clone();
        let mut workspace = replacement.workspace().clone();
        let node = workspace.graph().nodes()[0].id();
        let placement = workspace.placement(node).unwrap();
        workspace
            .move_node(node, placement.x() + 7, placement.y() - 3)
            .unwrap();
        replacement.replace_workspace(workspace).unwrap();
        let replacement_digest = encode_graph_component(&replacement).unwrap().digest();

        assert_eq!(
            hierarchy
                .replace_component(original, replacement.clone())
                .unwrap(),
            replacement_digest
        );
        assert_eq!(hierarchy.revision(), original_revision + 1);
        assert_eq!(hierarchy.root(), &original_root);
        assert!(hierarchy.dependency(original).is_none());
        assert_eq!(
            hierarchy.dependency(replacement_digest).unwrap().document(),
            &replacement
        );
        assert_eq!(hierarchy.instances()[0].component(), replacement_digest);
        let replaced = hierarchy.clone();
        assert_eq!(
            hierarchy
                .replace_component(replacement_digest, replacement)
                .unwrap(),
            replacement_digest
        );
        assert_eq!(
            hierarchy, replaced,
            "exact replacement no-op advanced state"
        );

        let retained = hierarchy.clone();
        assert_eq!(
            hierarchy.replace_component(Digest([0x72; 32]), component()),
            Err(GraphHierarchyError::UnknownComponent(Digest([0x72; 32])))
        );
        assert_eq!(hierarchy, retained);
    }

    #[test]
    fn unreferenced_component_library_lifecycle_is_transactional_and_exact() {
        let mut hierarchy = hierarchy();
        let original_root = hierarchy.root().clone();
        let original_counts = (
            hierarchy.flattened_instance_count(),
            hierarchy.flattened_node_count(),
            hierarchy.flattened_wire_count(),
        );
        let original_revision = hierarchy.revision();
        let mut extra = component();
        let mut workspace = extra.workspace().clone();
        let node = workspace.graph().nodes()[0].id();
        let placement = workspace.placement(node).unwrap();
        workspace
            .move_node(node, placement.x() + 31, placement.y() + 17)
            .unwrap();
        extra.replace_workspace(workspace).unwrap();
        let digest = encode_graph_component(&extra).unwrap().digest();

        assert_eq!(hierarchy.add_component(extra.clone()).unwrap(), digest);
        assert_eq!(hierarchy.revision(), original_revision + 1);
        assert_eq!(hierarchy.dependencies().len(), 2);
        assert_eq!(hierarchy.dependency(digest).unwrap().document(), &extra);
        assert_eq!(hierarchy.root(), &original_root);
        assert_eq!(
            (
                hierarchy.flattened_instance_count(),
                hierarchy.flattened_node_count(),
                hierarchy.flattened_wire_count(),
            ),
            original_counts
        );
        let encoding = encode_graph_hierarchy(&hierarchy).unwrap();
        assert_eq!(
            replay_graph_hierarchy(
                encoding.bytes(),
                GraphHierarchyLimits::interactive(),
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            )
            .unwrap()
            .document(),
            &hierarchy
        );

        let added = hierarchy.clone();
        assert_eq!(hierarchy.add_component(extra).unwrap(), digest);
        assert_eq!(hierarchy, added, "exact duplicate import advanced state");

        hierarchy.remove_component(digest).unwrap();
        assert_eq!(hierarchy.revision(), original_revision + 2);
        assert_eq!(hierarchy.dependencies().len(), 1);
        assert!(hierarchy.dependency(digest).is_none());
        assert_eq!(hierarchy.root(), &original_root);
        assert_eq!(
            (
                hierarchy.flattened_instance_count(),
                hierarchy.flattened_node_count(),
                hierarchy.flattened_wire_count(),
            ),
            original_counts
        );
    }

    #[test]
    fn referenced_and_unknown_component_removal_preserve_the_library() {
        let mut hierarchy = hierarchy();
        let referenced = hierarchy.dependencies()[0].digest();
        let retained = hierarchy.clone();
        assert_eq!(
            hierarchy.remove_component(referenced),
            Err(GraphHierarchyError::ComponentInUse(referenced))
        );
        assert_eq!(hierarchy, retained);
        assert_eq!(
            hierarchy.remove_component(Digest([0xb4; 32])),
            Err(GraphHierarchyError::UnknownComponent(Digest([0xb4; 32])))
        );
        assert_eq!(hierarchy, retained);
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
            Err(GraphHierarchyError::MissingInstanceBinding {
                scope: GraphInstanceScope::Root,
                node: GraphNodeId::new(2),
            })
        );
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                initial_root.clone(),
                vec![initial_component.clone()],
                vec![GraphComponentInstance::root(
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
                    GraphComponentInstance::root(GraphNodeId::new(2), digest),
                    GraphComponentInstance::root(GraphNodeId::new(2), digest),
                ],
            ),
            Err(GraphHierarchyError::DuplicateInstance {
                scope: GraphInstanceScope::Root,
                node: GraphNodeId::new(2),
            })
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
                vec![GraphComponentInstance::root(GraphNodeId::new(2), digest)],
            ),
            Err(GraphHierarchyError::InstanceShapeMismatch {
                scope: GraphInstanceScope::Root,
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
                vec![GraphComponentInstance::root(GraphNodeId::new(2), digest)],
            ),
            Err(GraphHierarchyError::UnsupportedInstanceVersion {
                scope: GraphInstanceScope::Root,
                node: GraphNodeId::new(2),
                version: 2,
            })
        );
    }

    #[test]
    fn dependency_and_binding_order_is_canonical_and_nested_dag_flattens() {
        let leaf = component();
        let leaf_digest = encode_graph_component(&leaf).unwrap().digest();
        let wrapper = wrapper_component(&leaf, "control.pid_wrapper");
        let wrapper_digest = encode_graph_component(&wrapper).unwrap().digest();
        let wrapper_root = root(&wrapper);
        let bindings = vec![
            GraphComponentInstance::root(GraphNodeId::new(2), wrapper_digest),
            GraphComponentInstance::nested(wrapper_digest, GraphNodeId::new(1), leaf_digest),
        ];
        let left = GraphHierarchyDocument::try_new(
            GraphHierarchyLimits::interactive(),
            1,
            wrapper_root.clone(),
            vec![leaf.clone(), wrapper.clone()],
            bindings.clone(),
        )
        .unwrap();
        let right = GraphHierarchyDocument::try_new(
            GraphHierarchyLimits::interactive(),
            1,
            wrapper_root,
            vec![wrapper, leaf],
            bindings.into_iter().rev().collect(),
        )
        .unwrap();
        assert_eq!(left, right);
        assert_eq!(left.flattened_instance_count(), 2);
        assert_eq!(left.flattened_node_count(), 22);
        assert_eq!(left.flattened_wire_count(), 26);
        assert_eq!(
            encode_graph_hierarchy(&left).unwrap(),
            encode_graph_hierarchy(&right).unwrap()
        );

        let flattened = flatten_graph_hierarchy(&left).unwrap();
        assert_eq!(flattened.workspace().graph().nodes().len(), 22);
        assert_eq!(flattened.workspace().graph().wires().len(), 26);
        assert_eq!(flattened.instances().len(), 2);
        assert_eq!(
            flattened.instances()[0].source_path(),
            [GraphNodeId::new(2)]
        );
        assert!(flattened.instances()[0].nodes().is_empty());
        assert_eq!(
            flattened.instances()[1].source_path(),
            [GraphNodeId::new(2), GraphNodeId::new(1)]
        );
        assert_eq!(flattened.instances()[1].nodes().len(), 20);
        assert_eq!(flattened.node_provenance().len(), 22);
        assert_eq!(flattened.wire_provenance().len(), 26);
        assert_eq!(
            flattened
                .node_provenance()
                .iter()
                .filter(|mapping| matches!(
                    mapping.origin(),
                    GraphHierarchyNodeOrigin::Component { source_path, .. }
                        if source_path == &[GraphNodeId::new(2), GraphNodeId::new(1)]
                ))
                .count(),
            20
        );
        assert_eq!(
            flattened
                .wire_provenance()
                .iter()
                .filter(|mapping| matches!(
                    mapping.origin(),
                    GraphHierarchyWireOrigin::Component { source_path, .. }
                        if source_path == &[GraphNodeId::new(2), GraphNodeId::new(1)]
                ))
                .count(),
            24
        );
        assert!(flattened.node_provenance().iter().all(|mapping| {
            !matches!(
                mapping.origin(),
                GraphHierarchyNodeOrigin::Component { source_path, .. }
                    if source_path == &[GraphNodeId::new(2)]
            )
        }));
        assert!(
            !flattened
                .workspace()
                .graph()
                .nodes()
                .iter()
                .any(has_component_instance_name)
        );
        let fixture = compile_representative_exact_control_graph().unwrap();
        assert!(
            analyze_graph(
                flattened.workspace().graph(),
                fixture.registry().semantic_registry()
            )
            .is_ok()
        );

        let encoding = encode_graph_hierarchy(&left).unwrap();
        assert_eq!(
            replay_graph_hierarchy(
                encoding.bytes(),
                GraphHierarchyLimits::interactive(),
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            )
            .unwrap()
            .document(),
            &left
        );
    }

    #[test]
    fn nested_reconnection_preserves_parent_component_wire_origin() {
        let leaf = chain_component();
        let leaf_digest = encode_graph_component(&leaf).unwrap().digest();
        let wrapper = wired_wrapper_component(&leaf, "control.wired_wrapper");
        let wrapper_digest = encode_graph_component(&wrapper).unwrap().digest();
        let hierarchy = GraphHierarchyDocument::try_new(
            GraphHierarchyLimits::interactive(),
            1,
            root(&wrapper),
            vec![leaf, wrapper],
            vec![
                GraphComponentInstance::root(GraphNodeId::new(2), wrapper_digest),
                GraphComponentInstance::nested(wrapper_digest, GraphNodeId::new(1), leaf_digest),
            ],
        )
        .unwrap();

        let flattened = flatten_graph_hierarchy(&hierarchy).unwrap();
        assert_eq!(flattened.node_provenance().len(), 23);
        assert_eq!(flattened.wire_provenance().len(), 27);
        let parent_origin = GraphHierarchyWireOrigin::Component {
            source_path: vec![GraphNodeId::new(2)],
            component: wrapper_digest,
            wire: GraphWireId::new(1),
        };
        let final_wire = flattened.flattened_wire(&parent_origin).unwrap();
        assert_eq!(flattened.wire_origin(final_wire), Some(&parent_origin));
        assert_eq!(
            flattened
                .wire_provenance()
                .iter()
                .filter(|mapping| matches!(
                    mapping.origin(),
                    GraphHierarchyWireOrigin::Component { source_path, .. }
                        if source_path == &[GraphNodeId::new(2)]
                ))
                .count(),
            1
        );
    }

    #[test]
    fn cycles_depth_and_expanded_instance_limits_fail_before_flattening() {
        let leaf = component();
        let leaf_digest = encode_graph_component(&leaf).unwrap().digest();
        let wrapper = wrapper_component(&leaf, "control.pid_wrapper");
        let wrapper_digest = encode_graph_component(&wrapper).unwrap().digest();
        let wrapper_root = root(&wrapper);
        let bindings = vec![
            GraphComponentInstance::root(GraphNodeId::new(2), wrapper_digest),
            GraphComponentInstance::nested(wrapper_digest, GraphNodeId::new(1), leaf_digest),
        ];
        let mut depth_limits = GraphHierarchyLimits::interactive();
        depth_limits.maximum_nesting_depth = 1;
        assert_eq!(
            GraphHierarchyDocument::try_new(
                depth_limits,
                1,
                wrapper_root.clone(),
                vec![leaf.clone(), wrapper.clone()],
                bindings.clone(),
            ),
            Err(GraphHierarchyError::LimitExceeded("nesting depth"))
        );
        let mut expansion_limits = GraphHierarchyLimits::interactive();
        expansion_limits.maximum_expanded_instances = 1;
        assert_eq!(
            GraphHierarchyDocument::try_new(
                expansion_limits,
                1,
                wrapper_root,
                vec![leaf, wrapper],
                bindings,
            ),
            Err(GraphHierarchyError::LimitExceeded(
                "expanded instance count"
            ))
        );

        let leaf = component();
        let first = wrapper_component(&leaf, "control.cycle_a");
        let second = wrapper_component(&leaf, "control.cycle_b");
        let first_digest = encode_graph_component(&first).unwrap().digest();
        let second_digest = encode_graph_component(&second).unwrap().digest();
        let cycle_at = first_digest.min(second_digest);
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                root(&first),
                vec![first, second],
                vec![
                    GraphComponentInstance::root(GraphNodeId::new(2), first_digest),
                    GraphComponentInstance::nested(
                        first_digest,
                        GraphNodeId::new(1),
                        second_digest,
                    ),
                    GraphComponentInstance::nested(
                        second_digest,
                        GraphNodeId::new(1),
                        first_digest,
                    ),
                ],
            ),
            Err(GraphHierarchyError::DependencyCycle(cycle_at))
        );
    }

    #[test]
    fn nested_bindings_are_total_unique_and_scoped_to_known_components() {
        let leaf = component();
        let leaf_digest = encode_graph_component(&leaf).unwrap().digest();
        let wrapper = wrapper_component(&leaf, "control.pid_wrapper");
        let wrapper_digest = encode_graph_component(&wrapper).unwrap().digest();
        let wrapper_root = root(&wrapper);
        let root_binding = GraphComponentInstance::root(GraphNodeId::new(2), wrapper_digest);

        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                wrapper_root.clone(),
                vec![leaf.clone(), wrapper.clone()],
                vec![root_binding],
            ),
            Err(GraphHierarchyError::MissingInstanceBinding {
                scope: GraphInstanceScope::Component(wrapper_digest),
                node: GraphNodeId::new(1),
            })
        );

        let nested =
            GraphComponentInstance::nested(wrapper_digest, GraphNodeId::new(1), leaf_digest);
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                wrapper_root.clone(),
                vec![leaf.clone(), wrapper.clone()],
                vec![root_binding, nested, nested],
            ),
            Err(GraphHierarchyError::DuplicateInstance {
                scope: GraphInstanceScope::Component(wrapper_digest),
                node: GraphNodeId::new(1),
            })
        );

        let unknown_parent = Digest([0xff; 32]);
        assert_eq!(
            GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                wrapper_root,
                vec![leaf, wrapper],
                vec![
                    root_binding,
                    nested,
                    GraphComponentInstance::nested(
                        unknown_parent,
                        GraphNodeId::new(1),
                        leaf_digest,
                    ),
                ],
            ),
            Err(GraphHierarchyError::UnknownInstanceScope(unknown_parent))
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
        let mut old_version = encoding.bytes().to_vec();
        old_version[4..6].copy_from_slice(&1_u16.to_le_bytes());
        assert_eq!(
            replay_graph_hierarchy(
                &old_version,
                GraphHierarchyLimits::interactive(),
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            ),
            Err(GraphHierarchyError::UnsupportedVersion(1))
        );
        let mut decoder = Decoder::new(encoding.bytes());
        decoder.take(4).unwrap();
        decoder.u16().unwrap();
        decoder.u16().unwrap();
        for _ in 0..HIERARCHY_LIMIT_FIELD_COUNT {
            decoder.u64().unwrap();
        }
        decoder.u64().unwrap();
        let root_length = usize::try_from(decoder.u32().unwrap()).unwrap();
        decoder.take(root_length).unwrap();
        let component_count = decoder.u32().unwrap();
        for _ in 0..component_count {
            let component_length = usize::try_from(decoder.u32().unwrap()).unwrap();
            decoder.take(component_length).unwrap();
        }
        assert_eq!(decoder.u32().unwrap(), 1);
        let scope_tag_offset = decoder.cursor;
        let mut bad_scope_tag = encoding.bytes().to_vec();
        bad_scope_tag[scope_tag_offset] = 2;
        assert_eq!(
            replay_graph_hierarchy(
                &bad_scope_tag,
                GraphHierarchyLimits::interactive(),
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            ),
            Err(GraphHierarchyError::NonCanonical)
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
                vec![GraphComponentInstance::root(GraphNodeId::new(2), digest)],
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
                GraphComponentInstance::root(GraphNodeId::new(1), digest),
                GraphComponentInstance::root(GraphNodeId::new(2), digest),
            ],
        )
        .unwrap();
        assert_eq!(hierarchy.flattened_node_count(), 40);
        assert_eq!(hierarchy.flattened_wire_count(), 49);
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
