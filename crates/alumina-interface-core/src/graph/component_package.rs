//! Canonical, bounded exchange of one reusable component dependency closure.
//!
//! A standalone `ALGC` cannot carry the scoped bindings required by nested
//! authoring-only component placeholders. `ALCP` therefore carries one exact
//! root component, every transitively required `ALGC`, and only their scoped
//! bindings. It remains authoring data: replay grants no semantic, execution,
//! resource, timing, safety, or deployment authority.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::{
    CanonicalGraphComponentEncoding, GraphComponentDocument, GraphComponentError,
    GraphComponentInstance, GraphComponentLimits, GraphDocument, GraphDocumentError,
    GraphHierarchyDocument, GraphHierarchyError, GraphHierarchyLimits, GraphInstanceScope,
    GraphLimits, GraphNodeId, GraphNodePlacement, GraphWorkspaceDocument, GraphWorkspaceError,
    GraphWorkspaceLimits, NodeDefinition, encode_graph_component,
    graph_component_instance_prototype, replay_graph_component,
};

/// Magic bytes at the beginning of each canonical component package.
pub const GRAPH_COMPONENT_PACKAGE_MAGIC: [u8; 4] = *b"ALCP";

/// Exact canonical component-package format implemented by this source tree.
pub const GRAPH_COMPONENT_PACKAGE_VERSION: u16 = 1;

const GRAPH_COMPONENT_PACKAGE_FLAGS: u16 = 0;
const COMPONENT_PACKAGE_LIMIT_FIELD_COUNT: usize = 7;
const PACKAGE_ROOT_LABEL: &str = "Canonical package root";

/// Caller-owned and embedded bounds for one reusable component closure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphComponentPackageLimits {
    /// Maximum complete canonical `ALCP` bytes.
    pub maximum_package_bytes: usize,
    /// Maximum distinct canonical component definitions.
    pub maximum_components: usize,
    /// Maximum component-scoped instance bindings.
    pub maximum_instance_bindings: usize,
    /// Maximum dependency depth, including the package root.
    pub maximum_nesting_depth: usize,
    /// Maximum expanded component occurrences, including the package root.
    pub maximum_expanded_instances: usize,
    /// Maximum ordinary nodes after recursively expanding the package root.
    pub maximum_flattened_nodes: usize,
    /// Maximum ordinary wires after recursively expanding the package root.
    pub maximum_flattened_wires: usize,
}

impl GraphComponentPackageLimits {
    /// Bounded first interactive exchange policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_package_bytes: 32 * 1024 * 1024,
            maximum_components: 64,
            maximum_instance_bindings: 4_096,
            maximum_nesting_depth: 32,
            maximum_expanded_instances: 4_096,
            maximum_flattened_nodes: 4_096,
            maximum_flattened_wires: 8_192,
        }
    }

    fn validate(self) -> Result<(), GraphComponentPackageError> {
        if self.maximum_package_bytes == 0
            || self.maximum_components == 0
            || self.maximum_instance_bindings == 0
            || self.maximum_nesting_depth == 0
            || self.maximum_expanded_instances == 0
            || self.maximum_flattened_nodes == 0
            || self.maximum_flattened_wires == 0
        {
            Err(GraphComponentPackageError::ZeroLimit)
        } else {
            Ok(())
        }
    }

    const fn hierarchy_limits(self) -> GraphHierarchyLimits {
        GraphHierarchyLimits {
            maximum_hierarchy_bytes: self.maximum_package_bytes,
            maximum_components: self.maximum_components,
            maximum_instance_bindings: self.maximum_instance_bindings.saturating_add(1),
            maximum_nesting_depth: self.maximum_nesting_depth,
            maximum_expanded_instances: self.maximum_expanded_instances,
            maximum_flattened_nodes: self.maximum_flattened_nodes,
            maximum_flattened_wires: self.maximum_flattened_wires,
        }
    }
}

impl Default for GraphComponentPackageLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// One canonical dependency retained by an `ALCP` closure.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphComponentPackageDependency {
    document: GraphComponentDocument,
    encoding: CanonicalGraphComponentEncoding,
}

impl GraphComponentPackageDependency {
    /// Borrow the validated component definition.
    pub const fn document(&self) -> &GraphComponentDocument {
        &self.document
    }

    /// Borrow the exact canonical `ALGC` bytes.
    pub const fn encoding(&self) -> &CanonicalGraphComponentEncoding {
        &self.encoding
    }

    /// Return the exact canonical component identity.
    pub const fn digest(&self) -> Digest {
        self.encoding.digest()
    }
}

/// One exact reusable component and its complete transitive dependency closure.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphComponentPackageDocument {
    limits: GraphComponentPackageLimits,
    root: Digest,
    dependencies: Vec<GraphComponentPackageDependency>,
    instances: Vec<GraphComponentInstance>,
    expanded_instances: usize,
    flattened_nodes: usize,
    flattened_wires: usize,
}

impl GraphComponentPackageDocument {
    /// Validate and canonicalize a complete component closure.
    pub fn try_new(
        limits: GraphComponentPackageLimits,
        root: Digest,
        components: Vec<GraphComponentDocument>,
        mut instances: Vec<GraphComponentInstance>,
    ) -> Result<Self, GraphComponentPackageError> {
        limits.validate()?;
        if components.len() > limits.maximum_components {
            return Err(GraphComponentPackageError::LimitExceeded("component count"));
        }
        if instances.len() > limits.maximum_instance_bindings {
            return Err(GraphComponentPackageError::LimitExceeded(
                "instance binding count",
            ));
        }
        if instances
            .iter()
            .any(|instance| instance.scope() == GraphInstanceScope::Root)
        {
            return Err(GraphComponentPackageError::RootBinding);
        }

        let mut dependencies = Vec::with_capacity(components.len());
        for document in components {
            let encoding = encode_graph_component(&document)?;
            dependencies.push(GraphComponentPackageDependency { document, encoding });
        }
        dependencies.sort_unstable_by_key(GraphComponentPackageDependency::digest);
        for pair in dependencies.windows(2) {
            if pair[0].digest() == pair[1].digest() {
                return Err(GraphComponentPackageError::DuplicateComponent(
                    pair[0].digest(),
                ));
            }
        }
        let root_dependency = dependency_by_digest(&dependencies, root)
            .ok_or(GraphComponentPackageError::UnknownRoot(root))?;
        validate_stable_names(&dependencies)?;
        instances.sort_unstable();

        let root_workspace = synthetic_package_root(root_dependency.document(), limits)?;
        let mut hierarchy_instances = instances.clone();
        hierarchy_instances.push(GraphComponentInstance::root(GraphNodeId::new(1), root));
        let hierarchy = GraphHierarchyDocument::try_new(
            limits.hierarchy_limits(),
            1,
            root_workspace,
            dependencies
                .iter()
                .map(|dependency| dependency.document.clone())
                .collect(),
            hierarchy_instances,
        )?;

        let reachable = reachable_components(root, &instances)?;
        if reachable.len() != dependencies.len() {
            let unreachable = dependencies
                .iter()
                .map(GraphComponentPackageDependency::digest)
                .find(|digest| !reachable.contains(digest))
                .ok_or(GraphComponentPackageError::NonCanonical)?;
            return Err(GraphComponentPackageError::UnreachableComponent(
                unreachable,
            ));
        }

        Ok(Self {
            limits,
            root,
            dependencies,
            instances,
            expanded_instances: hierarchy.flattened_instance_count(),
            flattened_nodes: hierarchy.flattened_node_count(),
            flattened_wires: hierarchy.flattened_wire_count(),
        })
    }

    /// Return embedded package limits.
    pub const fn limits(&self) -> GraphComponentPackageLimits {
        self.limits
    }

    /// Return the exact root component identity.
    pub const fn root(&self) -> Digest {
        self.root
    }

    /// Borrow the complete dependency closure in digest order.
    pub fn dependencies(&self) -> &[GraphComponentPackageDependency] {
        &self.dependencies
    }

    /// Borrow component-scoped bindings in canonical parent/node/child order.
    pub fn instances(&self) -> &[GraphComponentInstance] {
        &self.instances
    }

    /// Return expanded component occurrences, including the package root.
    pub const fn expanded_instance_count(&self) -> usize {
        self.expanded_instances
    }

    /// Return ordinary nodes after complete recursive expansion.
    pub const fn flattened_node_count(&self) -> usize {
        self.flattened_nodes
    }

    /// Return ordinary wires after complete recursive expansion.
    pub const fn flattened_wire_count(&self) -> usize {
        self.flattened_wires
    }
}

/// Canonical component-package bytes paired with their SHA-256 identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphComponentPackageEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphComponentPackageEncoding {
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

/// Successfully replayed component package and its exact canonical encoding.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphComponentPackageReplay {
    document: GraphComponentPackageDocument,
    encoding: CanonicalGraphComponentPackageEncoding,
}

impl GraphComponentPackageReplay {
    /// Borrow the reconstructed package.
    pub const fn document(&self) -> &GraphComponentPackageDocument {
        &self.document
    }

    /// Borrow byte-for-byte verified canonical encoding.
    pub const fn encoding(&self) -> &CanonicalGraphComponentPackageEncoding {
        &self.encoding
    }

    /// Consume replay and return the validated package document.
    pub fn into_document(self) -> GraphComponentPackageDocument {
        self.document
    }
}

/// Exact additions from one successful package merge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphComponentPackageImportReport {
    root: Digest,
    added_components: Vec<Digest>,
    added_instance_bindings: usize,
}

impl GraphComponentPackageImportReport {
    /// Return the imported package's exact root identity.
    pub const fn root(&self) -> Digest {
        self.root
    }

    /// Borrow newly added component identities in digest order.
    pub fn added_components(&self) -> &[Digest] {
        &self.added_components
    }

    /// Return the number of newly added scoped bindings.
    pub const fn added_instance_bindings(&self) -> usize {
        self.added_instance_bindings
    }

    /// Return whether the merge changed the destination hierarchy.
    pub fn changed(&self) -> bool {
        !self.added_components.is_empty() || self.added_instance_bindings != 0
    }
}

impl GraphHierarchyDocument {
    /// Extract one dependency and exactly its transitive child closure.
    pub fn export_component_package(
        &self,
        root: Digest,
        limits: GraphComponentPackageLimits,
    ) -> Result<GraphComponentPackageDocument, GraphComponentPackageError> {
        if self.dependency(root).is_none() {
            return Err(GraphComponentPackageError::UnknownRoot(root));
        }
        let mut reachable = BTreeSet::new();
        let mut pending = vec![root];
        let mut instances = Vec::new();
        while let Some(parent) = pending.pop() {
            if !reachable.insert(parent) {
                continue;
            }
            let mut children = self
                .instances()
                .iter()
                .copied()
                .filter(|instance| instance.scope() == GraphInstanceScope::Component(parent))
                .collect::<Vec<_>>();
            children.sort_unstable();
            for instance in children.iter().rev() {
                pending.push(instance.component());
            }
            instances.extend(children);
        }
        let components = self
            .dependencies()
            .iter()
            .filter(|dependency| reachable.contains(&dependency.digest()))
            .map(|dependency| dependency.document().clone())
            .collect();
        GraphComponentPackageDocument::try_new(limits, root, components, instances)
    }

    /// Merge one independently replayed closure without replacing any existing
    /// component or scoped binding. Exact duplicates are a no-op; a stable-name
    /// collision or different binding for an existing parent/node rejects the
    /// complete transaction.
    pub fn import_component_package(
        &mut self,
        package: &GraphComponentPackageDocument,
    ) -> Result<GraphComponentPackageImportReport, GraphComponentPackageError> {
        let mut components = self
            .dependencies()
            .iter()
            .map(|dependency| dependency.document().clone())
            .collect::<Vec<_>>();
        let mut added_components = Vec::new();
        for incoming in package.dependencies() {
            if let Some(existing) = self.dependency(incoming.digest()) {
                if existing.encoding().bytes() != incoming.encoding().bytes() {
                    return Err(GraphComponentPackageError::NonCanonical);
                }
                continue;
            }
            if let Some(existing) = self.dependencies().iter().find(|existing| {
                existing.document().name() == incoming.document().name()
                    && existing.digest() != incoming.digest()
            }) {
                return Err(GraphComponentPackageError::StableNameConflict {
                    name: incoming.document().name().to_owned(),
                    existing: existing.digest(),
                    incoming: incoming.digest(),
                });
            }
            components.push(incoming.document().clone());
            added_components.push(incoming.digest());
        }
        added_components.sort_unstable();

        let mut instances = self.instances().to_vec();
        let mut added_instance_bindings = 0_usize;
        for incoming in package.instances() {
            let key = (incoming.scope(), incoming.node());
            if let Some(existing) = instances
                .iter()
                .find(|existing| (existing.scope(), existing.node()) == key)
            {
                if existing.component() != incoming.component() {
                    let GraphInstanceScope::Component(parent) = incoming.scope() else {
                        return Err(GraphComponentPackageError::RootBinding);
                    };
                    return Err(GraphComponentPackageError::BindingConflict {
                        parent,
                        node: incoming.node(),
                        existing: existing.component(),
                        incoming: incoming.component(),
                    });
                }
                continue;
            }
            instances.push(*incoming);
            added_instance_bindings = added_instance_bindings.checked_add(1).ok_or(
                GraphComponentPackageError::IntegerOverflow("added binding count"),
            )?;
        }

        let report = GraphComponentPackageImportReport {
            root: package.root(),
            added_components,
            added_instance_bindings,
        };
        if !report.changed() {
            return Ok(report);
        }
        let revision =
            self.revision()
                .checked_add(1)
                .ok_or(GraphComponentPackageError::IntegerOverflow(
                    "hierarchy revision",
                ))?;
        let candidate = GraphHierarchyDocument::try_new(
            self.limits(),
            revision,
            self.root().clone(),
            components,
            instances,
        )?;
        *self = candidate;
        Ok(report)
    }
}

/// Encode one validated package and compute its exact content identity.
pub fn encode_graph_component_package(
    document: &GraphComponentPackageDocument,
) -> Result<CanonicalGraphComponentPackageEncoding, GraphComponentPackageError> {
    let mut encoder = Encoder::default();
    encoder.bytes(&GRAPH_COMPONENT_PACKAGE_MAGIC);
    encoder.u16(GRAPH_COMPONENT_PACKAGE_VERSION);
    encoder.u16(GRAPH_COMPONENT_PACKAGE_FLAGS);
    encode_limits(&mut encoder, document.limits)?;
    encoder.bytes(&document.root.0);
    encoder.u32(
        u32::try_from(document.dependencies.len())
            .map_err(|_| GraphComponentPackageError::IntegerOverflow("component count"))?,
    );
    for dependency in &document.dependencies {
        let encoding = encode_graph_component(dependency.document())?;
        if encoding != *dependency.encoding() {
            return Err(GraphComponentPackageError::NonCanonical);
        }
        encoder.length_prefixed(encoding.bytes(), "component length")?;
    }
    encoder.u32(
        u32::try_from(document.instances.len())
            .map_err(|_| GraphComponentPackageError::IntegerOverflow("instance count"))?,
    );
    for instance in &document.instances {
        let GraphInstanceScope::Component(parent) = instance.scope() else {
            return Err(GraphComponentPackageError::RootBinding);
        };
        encoder.bytes(&parent.0);
        encoder.u32(instance.node().get());
        encoder.bytes(&instance.component().0);
    }
    if encoder.0.len() > document.limits.maximum_package_bytes {
        return Err(GraphComponentPackageError::LimitExceeded(
            "document byte length",
        ));
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphComponentPackageEncoding {
        bytes: encoder.0,
        digest,
    })
}

/// Decode, validate, canonically re-encode, and identify untrusted `ALCP` bytes.
#[allow(
    clippy::too_many_arguments,
    reason = "each nested canonical envelope retains an independent caller-owned admission policy"
)]
pub fn replay_graph_component_package(
    bytes: &[u8],
    package_admission: GraphComponentPackageLimits,
    component_admission: GraphComponentLimits,
    workspace_admission: GraphWorkspaceLimits,
    graph_admission: GraphLimits,
) -> Result<GraphComponentPackageReplay, GraphComponentPackageError> {
    package_admission.validate()?;
    if bytes.len() > package_admission.maximum_package_bytes {
        return Err(GraphComponentPackageError::LimitExceeded(
            "admitted document byte length",
        ));
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_COMPONENT_PACKAGE_MAGIC.len())? != GRAPH_COMPONENT_PACKAGE_MAGIC {
        return Err(GraphComponentPackageError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_COMPONENT_PACKAGE_VERSION {
        return Err(GraphComponentPackageError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_COMPONENT_PACKAGE_FLAGS {
        return Err(GraphComponentPackageError::UnsupportedFlags(flags));
    }
    let limits = decode_limits(&mut decoder)?;
    if !limits_within(limits, package_admission) {
        return Err(GraphComponentPackageError::LimitExceeded(
            "embedded admission limit",
        ));
    }
    limits.validate()?;
    if bytes.len() > limits.maximum_package_bytes {
        return Err(GraphComponentPackageError::LimitExceeded(
            "embedded document byte length",
        ));
    }
    let root = Digest(decoder.digest()?);
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
        let parent = Digest(decoder.digest()?);
        let node = GraphNodeId::new(decoder.u32()?);
        let component = Digest(decoder.digest()?);
        instances.push(GraphComponentInstance::nested(parent, node, component));
    }
    if !decoder.is_empty() {
        return Err(GraphComponentPackageError::TrailingBytes);
    }
    let document = GraphComponentPackageDocument::try_new(limits, root, components, instances)?;
    let encoding = encode_graph_component_package(&document)?;
    if encoding.bytes() != bytes {
        return Err(GraphComponentPackageError::NonCanonical);
    }
    Ok(GraphComponentPackageReplay { document, encoding })
}

/// Component-package construction, replay, or transactional import failure.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphComponentPackageError {
    /// A package policy contained zero.
    ZeroLimit,
    /// A byte length, count, depth, or expansion exceeded policy.
    LimitExceeded(&'static str),
    /// Input did not begin with [`GRAPH_COMPONENT_PACKAGE_MAGIC`].
    InvalidMagic,
    /// Component-package format version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved package flags were nonzero.
    UnsupportedFlags(u16),
    /// A fixed-width or length-delimited field ran past input.
    Truncated,
    /// A canonical field could not represent an in-memory value.
    IntegerOverflow(&'static str),
    /// Valid fields remained after the package.
    TrailingBytes,
    /// Decoding or exact identity comparison found a noncanonical value.
    NonCanonical,
    /// Two embedded dependencies had the same exact identity.
    DuplicateComponent(Digest),
    /// The selected root component was absent from the embedded closure.
    UnknownRoot(Digest),
    /// A package retained a root-workspace binding instead of component scope.
    RootBinding,
    /// A component was embedded but not reachable from the selected root.
    UnreachableComponent(Digest),
    /// Two exact definitions claimed one stable component name.
    StableNameConflict {
        /// Colliding stable name.
        name: String,
        /// Destination or earlier package identity.
        existing: Digest,
        /// Incoming identity.
        incoming: Digest,
    },
    /// Destination and package assign different children to one exact placeholder.
    BindingConflict {
        /// Exact containing component.
        parent: Digest,
        /// Parent-local placeholder.
        node: GraphNodeId,
        /// Existing child identity.
        existing: Digest,
        /// Incoming child identity.
        incoming: Digest,
    },
    /// Embedded component construction or replay failed.
    Component(GraphComponentError),
    /// Synthetic structural graph construction failed.
    Graph(GraphDocumentError),
    /// Synthetic root-workspace construction failed.
    Workspace(GraphWorkspaceError),
    /// Complete hierarchy validation or destination admission failed.
    Hierarchy(GraphHierarchyError),
}

impl From<GraphComponentError> for GraphComponentPackageError {
    fn from(value: GraphComponentError) -> Self {
        Self::Component(value)
    }
}

impl From<GraphDocumentError> for GraphComponentPackageError {
    fn from(value: GraphDocumentError) -> Self {
        Self::Graph(value)
    }
}

impl From<GraphWorkspaceError> for GraphComponentPackageError {
    fn from(value: GraphWorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl From<GraphHierarchyError> for GraphComponentPackageError {
    fn from(value: GraphHierarchyError) -> Self {
        Self::Hierarchy(value)
    }
}

impl fmt::Display for GraphComponentPackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("component-package policy contains zero"),
            Self::LimitExceeded(name) => {
                write!(formatter, "component-package {name} exceeds policy")
            }
            Self::InvalidMagic => formatter.write_str("component-package magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "component-package version {version} is unsupported"
                )
            }
            Self::UnsupportedFlags(flags) => write!(
                formatter,
                "component-package flags {flags:#06x} are unsupported"
            ),
            Self::Truncated => formatter.write_str("component package is truncated"),
            Self::IntegerOverflow(name) => {
                write!(
                    formatter,
                    "component-package {name} exceeds its integer width"
                )
            }
            Self::TrailingBytes => formatter.write_str("component package has trailing bytes"),
            Self::NonCanonical => formatter.write_str("component package is noncanonical"),
            Self::DuplicateComponent(digest) => {
                write!(formatter, "component package duplicates {digest:?}")
            }
            Self::UnknownRoot(digest) => {
                write!(formatter, "component-package root {digest:?} is unknown")
            }
            Self::RootBinding => {
                formatter.write_str("component package contains a root-workspace binding")
            }
            Self::UnreachableComponent(digest) => write!(
                formatter,
                "component-package dependency {digest:?} is outside the root closure"
            ),
            Self::StableNameConflict {
                name,
                existing,
                incoming,
            } => write!(
                formatter,
                "component stable name {name:?} conflicts between {existing:?} and {incoming:?}"
            ),
            Self::BindingConflict {
                parent,
                node,
                existing,
                incoming,
            } => write!(
                formatter,
                "component {parent:?} node {node:?} is already bound to {existing:?}, not incoming {incoming:?}"
            ),
            Self::Component(error) => write!(formatter, "package component failed: {error}"),
            Self::Graph(error) => write!(formatter, "package root graph failed: {error}"),
            Self::Workspace(error) => write!(formatter, "package root workspace failed: {error}"),
            Self::Hierarchy(error) => write!(formatter, "package hierarchy failed: {error}"),
        }
    }
}

impl std::error::Error for GraphComponentPackageError {}

fn synthetic_package_root(
    component: &GraphComponentDocument,
    limits: GraphComponentPackageLimits,
) -> Result<GraphWorkspaceDocument, GraphComponentPackageError> {
    let prototype = graph_component_instance_prototype(component, PACKAGE_ROOT_LABEL)?;
    let node = NodeDefinition::new(
        GraphNodeId::new(1),
        prototype.kind().clone(),
        PACKAGE_ROOT_LABEL,
        prototype.domain(),
        prototype.inputs().to_vec(),
        prototype.outputs().to_vec(),
        prototype.parameters().to_vec(),
    );
    let graph = GraphDocument::try_new(
        1,
        component.workspace().graph().schema().clone(),
        component.workspace().graph().clocks().to_vec(),
        vec![node],
        Vec::new(),
    )?;
    GraphWorkspaceDocument::try_new(
        GraphWorkspaceLimits {
            maximum_workspace_bytes: limits.maximum_package_bytes,
            maximum_placements: limits.maximum_flattened_nodes,
            maximum_coordinate_magnitude: 1,
        },
        1,
        2,
        1,
        graph,
        vec![GraphNodePlacement::new(GraphNodeId::new(1), 0, 0)],
    )
    .map_err(Into::into)
}

fn reachable_components(
    root: Digest,
    instances: &[GraphComponentInstance],
) -> Result<BTreeSet<Digest>, GraphComponentPackageError> {
    let mut reachable = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(parent) = pending.pop() {
        if !reachable.insert(parent) {
            continue;
        }
        for instance in instances
            .iter()
            .filter(|instance| instance.scope() == GraphInstanceScope::Component(parent))
        {
            pending.push(instance.component());
        }
    }
    Ok(reachable)
}

fn validate_stable_names(
    dependencies: &[GraphComponentPackageDependency],
) -> Result<(), GraphComponentPackageError> {
    let mut names = BTreeMap::new();
    for dependency in dependencies {
        if let Some(existing) = names.insert(dependency.document().name(), dependency.digest()) {
            return Err(GraphComponentPackageError::StableNameConflict {
                name: dependency.document().name().to_owned(),
                existing,
                incoming: dependency.digest(),
            });
        }
    }
    Ok(())
}

fn dependency_by_digest(
    dependencies: &[GraphComponentPackageDependency],
    digest: Digest,
) -> Option<&GraphComponentPackageDependency> {
    dependencies
        .binary_search_by_key(&digest, GraphComponentPackageDependency::digest)
        .ok()
        .map(|index| &dependencies[index])
}

fn encode_limits(
    encoder: &mut Encoder,
    limits: GraphComponentPackageLimits,
) -> Result<(), GraphComponentPackageError> {
    for value in [
        limits.maximum_package_bytes,
        limits.maximum_components,
        limits.maximum_instance_bindings,
        limits.maximum_nesting_depth,
        limits.maximum_expanded_instances,
        limits.maximum_flattened_nodes,
        limits.maximum_flattened_wires,
    ] {
        encoder.u64(
            u64::try_from(value)
                .map_err(|_| GraphComponentPackageError::IntegerOverflow("limit value"))?,
        );
    }
    Ok(())
}

fn decode_limits(
    decoder: &mut Decoder<'_>,
) -> Result<GraphComponentPackageLimits, GraphComponentPackageError> {
    let mut values = [0_usize; COMPONENT_PACKAGE_LIMIT_FIELD_COUNT];
    for value in &mut values {
        *value = usize::try_from(decoder.u64()?)
            .map_err(|_| GraphComponentPackageError::IntegerOverflow("limit value"))?;
    }
    Ok(GraphComponentPackageLimits {
        maximum_package_bytes: values[0],
        maximum_components: values[1],
        maximum_instance_bindings: values[2],
        maximum_nesting_depth: values[3],
        maximum_expanded_instances: values[4],
        maximum_flattened_nodes: values[5],
        maximum_flattened_wires: values[6],
    })
}

const fn limits_within(
    embedded: GraphComponentPackageLimits,
    admission: GraphComponentPackageLimits,
) -> bool {
    embedded.maximum_package_bytes <= admission.maximum_package_bytes
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
    ) -> Result<(), GraphComponentPackageError> {
        self.u32(
            u32::try_from(value.len())
                .map_err(|_| GraphComponentPackageError::IntegerOverflow(name))?,
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

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphComponentPackageError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(GraphComponentPackageError::Truncated)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GraphComponentPackageError::Truncated)?;
        self.cursor = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, GraphComponentPackageError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| GraphComponentPackageError::Truncated)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, GraphComponentPackageError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| GraphComponentPackageError::Truncated)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, GraphComponentPackageError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| GraphComponentPackageError::Truncated)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn digest(&mut self) -> Result<[u8; 32], GraphComponentPackageError> {
        self.take(32)?
            .try_into()
            .map_err(|_| GraphComponentPackageError::Truncated)
    }

    fn count(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<usize, GraphComponentPackageError> {
        let count = usize::try_from(self.u32()?)
            .map_err(|_| GraphComponentPackageError::IntegerOverflow(name))?;
        if count > maximum {
            Err(GraphComponentPackageError::LimitExceeded(name))
        } else {
            Ok(count)
        }
    }

    fn length_prefixed(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<&'a [u8], GraphComponentPackageError> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| GraphComponentPackageError::IntegerOverflow(name))?;
        if length > maximum {
            return Err(GraphComponentPackageError::LimitExceeded(name));
        }
        self.take(length)
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}
