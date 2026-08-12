//! Canonical, bounded editor workspace around one exact structural graph.
//!
//! `ALGR` remains the only executable graph authority. This module embeds those
//! exact bytes in an `ALGW` envelope with integer canvas coordinates and
//! monotonic editor identity cursors. Canvas values are presentation metadata;
//! no API converts them into graph literals, machine coordinates, or firmware
//! values.

use core::fmt;

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::{
    CanonicalGraphEncoding, ExecutionDomain, GraphDocument, GraphDocumentError, GraphLimits,
    GraphNodeId, GraphTypeId, GraphWireError, GraphWireId, NodeDefinition, NodeKind, NodeParameter,
    PortDefinition, TypedGraphValue, WireDefinition, WireEndpoint, encode_graph_document,
    replay_graph_document,
};

/// Magic bytes at the beginning of each canonical graph workspace.
pub const GRAPH_WORKSPACE_MAGIC: [u8; 4] = *b"ALGW";

/// Exact canonical graph-workspace format implemented by this source tree.
pub const GRAPH_WORKSPACE_VERSION: u16 = 1;

const GRAPH_WORKSPACE_FLAGS: u16 = 0;
const WORKSPACE_LIMIT_FIELD_COUNT: usize = 3;
const EXHAUSTED_U32_CURSOR: u64 = u32::MAX as u64 + 1;

/// Caller-owned and embedded bounds for one editor workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphWorkspaceLimits {
    /// Maximum complete canonical workspace byte length, including embedded
    /// `ALGR` bytes.
    pub maximum_workspace_bytes: usize,
    /// Maximum retained node placements.
    pub maximum_placements: usize,
    /// Maximum absolute integer canvas coordinate.
    pub maximum_coordinate_magnitude: u32,
}

impl GraphWorkspaceLimits {
    /// Bounded first editor policy. A future large-graph UI may admit a larger
    /// policy explicitly without changing existing workspace bytes.
    pub const fn interactive() -> Self {
        Self {
            maximum_workspace_bytes: 20 * 1024 * 1024,
            maximum_placements: 256,
            maximum_coordinate_magnitude: 1_000_000,
        }
    }

    fn validate(self) -> Result<(), GraphWorkspaceError> {
        if self.maximum_workspace_bytes == 0
            || self.maximum_placements == 0
            || self.maximum_coordinate_magnitude == 0
        {
            return Err(GraphWorkspaceError::ZeroLimit);
        }
        Ok(())
    }
}

impl Default for GraphWorkspaceLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// One stable integer canvas position. These units are presentation-only
/// logical points, not physical length or machine coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphNodePlacement {
    node: GraphNodeId,
    x: i32,
    y: i32,
}

impl GraphNodePlacement {
    /// Construct presentation-only placement metadata.
    pub const fn new(node: GraphNodeId, x: i32, y: i32) -> Self {
        Self { node, x, y }
    }

    /// Return the stable graph node identity.
    pub const fn node(self) -> GraphNodeId {
        self.node
    }

    /// Return the signed horizontal logical-point coordinate.
    pub const fn x(self) -> i32 {
        self.x
    }

    /// Return the signed vertical logical-point coordinate.
    pub const fn y(self) -> i32 {
        self.y
    }
}

/// Complete node shape supplied to the workspace before it assigns a stable
/// graph-local identity. A palette should derive these ports and parameter
/// contracts from an audited [`super::NodeSchema`]; structural workspace
/// admission deliberately remains independent of that semantic authority.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNodePrototype {
    kind: NodeKind,
    label: String,
    domain: ExecutionDomain,
    inputs: Vec<PortDefinition>,
    outputs: Vec<PortDefinition>,
    parameters: Vec<NodeParameter>,
}

impl GraphNodePrototype {
    /// Construct a node prototype. The complete candidate graph validates its
    /// names, domain, ports, parameter values, and bounds transactionally.
    pub fn new(
        kind: NodeKind,
        label: impl Into<String>,
        domain: ExecutionDomain,
        inputs: Vec<PortDefinition>,
        outputs: Vec<PortDefinition>,
        parameters: Vec<NodeParameter>,
    ) -> Self {
        Self {
            kind,
            label: label.into(),
            domain,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Return the opaque versioned behavior identity.
    pub const fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Return the proposed user-facing label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Return requested execution ownership.
    pub const fn domain(&self) -> ExecutionDomain {
        self.domain
    }

    /// Borrow exact proposed input ports.
    pub fn inputs(&self) -> &[PortDefinition] {
        &self.inputs
    }

    /// Borrow exact proposed output ports.
    pub fn outputs(&self) -> &[PortDefinition] {
        &self.outputs
    }

    /// Borrow exact proposed parameters.
    pub fn parameters(&self) -> &[NodeParameter] {
        &self.parameters
    }

    fn into_node(self, id: GraphNodeId) -> NodeDefinition {
        NodeDefinition::new(
            id,
            self.kind,
            self.label,
            self.domain,
            self.inputs,
            self.outputs,
            self.parameters,
        )
    }
}

/// Saved editor state containing one exact graph and presentation-only canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphWorkspaceDocument {
    limits: GraphWorkspaceLimits,
    revision: u64,
    next_node_id: u64,
    next_wire_id: u64,
    graph: GraphDocument,
    graph_digest: Digest,
    placements: Vec<GraphNodePlacement>,
}

impl GraphWorkspaceDocument {
    /// Validate and canonicalize one complete workspace.
    ///
    /// Every graph node must have exactly one placement. Identity cursors must
    /// be strictly beyond all retained IDs and are allowed to equal
    /// `u32::MAX + 1` only as an explicit exhausted sentinel.
    #[allow(
        clippy::too_many_arguments,
        reason = "workspace policy, identity allocation, graph, and presentation state remain explicit"
    )]
    pub fn try_new(
        limits: GraphWorkspaceLimits,
        revision: u64,
        next_node_id: u64,
        next_wire_id: u64,
        graph: GraphDocument,
        mut placements: Vec<GraphNodePlacement>,
    ) -> Result<Self, GraphWorkspaceError> {
        limits.validate()?;
        if graph.nodes().len() > limits.maximum_placements
            || placements.len() > limits.maximum_placements
        {
            return Err(GraphWorkspaceError::LimitExceeded("placement count"));
        }
        placements.sort_unstable_by_key(|placement| placement.node);
        validate_placements(&graph, &placements, limits)?;
        validate_identity_cursor(
            next_node_id,
            graph.nodes().iter().map(|node| u64::from(node.id().get())),
            "node",
        )?;
        validate_identity_cursor(
            next_wire_id,
            graph.wires().iter().map(|wire| u64::from(wire.id().get())),
            "wire",
        )?;
        let graph_digest = encode_graph_document(&graph)?.digest();
        Ok(Self {
            limits,
            revision,
            next_node_id,
            next_wire_id,
            graph,
            graph_digest,
            placements,
        })
    }

    /// Return embedded workspace limits.
    pub const fn limits(&self) -> GraphWorkspaceLimits {
        self.limits
    }

    /// Return the monotonic workspace edit revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Return the next stable node-ID cursor or `u32::MAX + 1` when exhausted.
    pub const fn next_node_id(&self) -> u64 {
        self.next_node_id
    }

    /// Return the next stable wire-ID cursor or `u32::MAX + 1` when exhausted.
    pub const fn next_wire_id(&self) -> u64 {
        self.next_wire_id
    }

    /// Borrow the exact executable structural graph authority.
    pub const fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    /// Return the SHA-256 identity of the embedded canonical `ALGR` bytes.
    pub const fn graph_digest(&self) -> Digest {
        self.graph_digest
    }

    /// Borrow placements in canonical node-ID order.
    pub fn placements(&self) -> &[GraphNodePlacement] {
        &self.placements
    }

    /// Resolve one presentation-only node position.
    pub fn placement(&self, node: GraphNodeId) -> Option<GraphNodePlacement> {
        self.placements
            .binary_search_by_key(&node, |placement| placement.node)
            .ok()
            .map(|index| self.placements[index])
    }

    /// Transactionally move one node and advance only the workspace revision.
    pub fn move_node(
        &mut self,
        node: GraphNodeId,
        x: i32,
        y: i32,
    ) -> Result<(), GraphWorkspaceError> {
        validate_coordinate(x, self.limits)?;
        validate_coordinate(y, self.limits)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("workspace"))?;
        let mut placements = self.placements.clone();
        let index = placements
            .binary_search_by_key(&node, |placement| placement.node)
            .map_err(|_| GraphWorkspaceError::UnknownNode(node))?;
        placements[index] = GraphNodePlacement::new(node, x, y);
        let candidate = Self::try_new(
            self.limits,
            revision,
            self.next_node_id,
            self.next_wire_id,
            self.graph.clone(),
            placements,
        )?;
        *self = candidate;
        Ok(())
    }

    /// Transactionally create one node at an integer canvas position. The
    /// workspace, not the caller, assigns the monotonic stable node identity.
    pub fn create_node(
        &mut self,
        prototype: GraphNodePrototype,
        x: i32,
        y: i32,
    ) -> Result<GraphNodeId, GraphWorkspaceError> {
        validate_coordinate(x, self.limits)?;
        validate_coordinate(y, self.limits)?;
        let node_value = u32::try_from(self.next_node_id)
            .map_err(|_| GraphWorkspaceError::IdentifierExhausted("node"))?;
        let following_node_id = self
            .next_node_id
            .checked_add(1)
            .filter(|value| *value <= EXHAUSTED_U32_CURSOR)
            .ok_or(GraphWorkspaceError::IdentifierExhausted("node"))?;
        let graph_revision = self
            .graph
            .revision()
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("graph"))?;
        let workspace_revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("workspace"))?;
        let id = GraphNodeId::new(node_value);
        let mut nodes = self.graph.nodes().to_vec();
        nodes.push(prototype.into_node(id));
        let graph = GraphDocument::try_new(
            graph_revision,
            self.graph.schema().clone(),
            self.graph.clocks().to_vec(),
            nodes,
            self.graph.wires().to_vec(),
        )?;
        let mut placements = self.placements.clone();
        placements.push(GraphNodePlacement::new(id, x, y));
        let candidate = Self::try_new(
            self.limits,
            workspace_revision,
            following_node_id,
            self.next_wire_id,
            graph,
            placements,
        )?;
        *self = candidate;
        Ok(id)
    }

    /// Transactionally remove one node, its placement, and all incident wires.
    /// Neither the node nor wire allocation cursor is rewound.
    pub fn delete_node(&mut self, id: GraphNodeId) -> Result<usize, GraphWorkspaceError> {
        if self.graph.node(id).is_none() {
            return Err(GraphWorkspaceError::UnknownNode(id));
        }
        let graph_revision = self
            .graph
            .revision()
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("graph"))?;
        let workspace_revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("workspace"))?;
        let nodes = self
            .graph
            .nodes()
            .iter()
            .filter(|node| node.id() != id)
            .cloned()
            .collect();
        let wires: Vec<_> = self
            .graph
            .wires()
            .iter()
            .copied()
            .filter(|wire| wire.source().node != id && wire.target().node != id)
            .collect();
        let removed_wires = self.graph.wires().len() - wires.len();
        let graph = GraphDocument::try_new(
            graph_revision,
            self.graph.schema().clone(),
            self.graph.clocks().to_vec(),
            nodes,
            wires,
        )?;
        let placements = self
            .placements
            .iter()
            .copied()
            .filter(|placement| placement.node != id)
            .collect();
        let candidate = Self::try_new(
            self.limits,
            workspace_revision,
            self.next_node_id,
            self.next_wire_id,
            graph,
            placements,
        )?;
        *self = candidate;
        Ok(removed_wires)
    }

    /// Transactionally replace one exact parameter value while preserving its
    /// stable ID, name, and registered root type.
    pub fn set_parameter(
        &mut self,
        node_id: GraphNodeId,
        parameter_id: u32,
        value: TypedGraphValue,
    ) -> Result<(), GraphWorkspaceError> {
        let node = self
            .graph
            .node(node_id)
            .ok_or(GraphWorkspaceError::UnknownNode(node_id))?;
        let mut parameters = node.parameters().to_vec();
        let parameter = parameters
            .iter_mut()
            .find(|parameter| parameter.id() == parameter_id)
            .ok_or(GraphWorkspaceError::UnknownParameter {
                node: node_id,
                parameter: parameter_id,
            })?;
        let expected = parameter.value().value_type();
        let received = value.value_type();
        if expected != received {
            return Err(GraphWorkspaceError::ParameterTypeMismatch { expected, received });
        }
        *parameter = NodeParameter::new(parameter.id(), parameter.name(), value);
        let replacement = NodeDefinition::new(
            node.id(),
            node.kind().clone(),
            node.label(),
            node.domain(),
            node.inputs().to_vec(),
            node.outputs().to_vec(),
            parameters,
        );
        let graph_revision = self
            .graph
            .revision()
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("graph"))?;
        let workspace_revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("workspace"))?;
        let nodes = self
            .graph
            .nodes()
            .iter()
            .map(|node| {
                if node.id() == node_id {
                    replacement.clone()
                } else {
                    node.clone()
                }
            })
            .collect();
        let graph = GraphDocument::try_new(
            graph_revision,
            self.graph.schema().clone(),
            self.graph.clocks().to_vec(),
            nodes,
            self.graph.wires().to_vec(),
        )?;
        let candidate = Self::try_new(
            self.limits,
            workspace_revision,
            self.next_node_id,
            self.next_wire_id,
            graph,
            self.placements.clone(),
        )?;
        *self = candidate;
        Ok(())
    }

    /// Transactionally add one typed output-to-input wire using the monotonic
    /// workspace cursor. Existing target ownership is never replaced silently.
    pub fn connect(
        &mut self,
        source: WireEndpoint,
        target: WireEndpoint,
    ) -> Result<GraphWireId, GraphWorkspaceError> {
        let wire_value = u32::try_from(self.next_wire_id)
            .map_err(|_| GraphWorkspaceError::IdentifierExhausted("wire"))?;
        let following_wire_id = self
            .next_wire_id
            .checked_add(1)
            .filter(|value| *value <= EXHAUSTED_U32_CURSOR)
            .ok_or(GraphWorkspaceError::IdentifierExhausted("wire"))?;
        let graph_revision = self
            .graph
            .revision()
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("graph"))?;
        let workspace_revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("workspace"))?;
        let id = GraphWireId::new(wire_value);
        let mut wires = self.graph.wires().to_vec();
        wires.push(WireDefinition::new(id, source, target));
        let graph = GraphDocument::try_new(
            graph_revision,
            self.graph.schema().clone(),
            self.graph.clocks().to_vec(),
            self.graph.nodes().to_vec(),
            wires,
        )?;
        let candidate = Self::try_new(
            self.limits,
            workspace_revision,
            self.next_node_id,
            following_wire_id,
            graph,
            self.placements.clone(),
        )?;
        *self = candidate;
        Ok(id)
    }

    /// Transactionally remove one wire without reusing its stable identity.
    pub fn disconnect(&mut self, id: GraphWireId) -> Result<(), GraphWorkspaceError> {
        if !self.graph.wires().iter().any(|wire| wire.id() == id) {
            return Err(GraphWorkspaceError::UnknownWire(id));
        }
        let graph_revision = self
            .graph
            .revision()
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("graph"))?;
        let workspace_revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphWorkspaceError::RevisionOverflow("workspace"))?;
        let wires = self
            .graph
            .wires()
            .iter()
            .copied()
            .filter(|wire| wire.id() != id)
            .collect();
        let graph = GraphDocument::try_new(
            graph_revision,
            self.graph.schema().clone(),
            self.graph.clocks().to_vec(),
            self.graph.nodes().to_vec(),
            wires,
        )?;
        let candidate = Self::try_new(
            self.limits,
            workspace_revision,
            self.next_node_id,
            self.next_wire_id,
            graph,
            self.placements.clone(),
        )?;
        *self = candidate;
        Ok(())
    }
}

/// Canonical workspace bytes paired with their SHA-256 content identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphWorkspaceEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphWorkspaceEncoding {
    /// Borrow the complete canonical bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the SHA-256 identity of exactly [`Self::bytes`].
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// Consume the carrier and return canonical bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Successfully replayed workspace and its verified canonical identity.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphWorkspaceReplay {
    document: GraphWorkspaceDocument,
    encoding: CanonicalGraphWorkspaceEncoding,
}

impl GraphWorkspaceReplay {
    /// Borrow the reconstructed workspace.
    pub const fn document(&self) -> &GraphWorkspaceDocument {
        &self.document
    }

    /// Borrow the byte-for-byte verified canonical encoding.
    pub const fn encoding(&self) -> &CanonicalGraphWorkspaceEncoding {
        &self.encoding
    }

    /// Consume the replay and return the workspace document.
    pub fn into_document(self) -> GraphWorkspaceDocument {
        self.document
    }
}

/// Rejection at the canonical workspace or transactional-edit boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphWorkspaceError {
    /// A workspace policy contained zero.
    ZeroLimit,
    /// A count, length, embedded policy, or coordinate exceeded policy.
    LimitExceeded(&'static str),
    /// Input did not begin with [`GRAPH_WORKSPACE_MAGIC`].
    InvalidMagic,
    /// The workspace format version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved workspace flags were nonzero.
    UnsupportedFlags(u16),
    /// A fixed-width or declared-length field ran past the input.
    Truncated,
    /// A canonical field could not represent an in-memory value.
    IntegerOverflow(&'static str),
    /// Valid fields remained after the workspace.
    TrailingBytes,
    /// Decoding and canonical reconstruction changed at least one byte.
    NonCanonical,
    /// A placement used node identity zero or duplicated another placement.
    InvalidPlacementIdentity,
    /// A graph node had no corresponding placement.
    MissingPlacement(GraphNodeId),
    /// A placement or edit referenced no graph node.
    UnknownNode(GraphNodeId),
    /// A monotonic allocation cursor was behind retained IDs or out of range.
    InvalidIdentifierCursor(&'static str),
    /// A stable identifier has no remaining `u32` value.
    IdentifierExhausted(&'static str),
    /// A requested wire did not exist.
    UnknownWire(GraphWireId),
    /// A requested node-local parameter did not exist.
    UnknownParameter {
        /// Owning node identity.
        node: GraphNodeId,
        /// Node-local parameter identity.
        parameter: u32,
    },
    /// A parameter edit attempted to change its registered root type.
    ParameterTypeMismatch {
        /// Retained parameter type.
        expected: GraphTypeId,
        /// Proposed parameter type.
        received: GraphTypeId,
    },
    /// A graph or workspace revision could not advance.
    RevisionOverflow(&'static str),
    /// Embedded `ALGR` encoding/replay failed.
    GraphWire(GraphWireError),
    /// A structural wire edit failed exact graph validation.
    GraphDocument(GraphDocumentError),
}

impl From<GraphWireError> for GraphWorkspaceError {
    fn from(value: GraphWireError) -> Self {
        Self::GraphWire(value)
    }
}

impl From<GraphDocumentError> for GraphWorkspaceError {
    fn from(value: GraphDocumentError) -> Self {
        Self::GraphDocument(value)
    }
}

impl fmt::Display for GraphWorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph workspace policy contains zero"),
            Self::LimitExceeded(name) => {
                write!(formatter, "graph workspace {name} exceeds policy")
            }
            Self::InvalidMagic => formatter.write_str("graph workspace magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "graph workspace version {version} is unsupported"
                )
            }
            Self::UnsupportedFlags(flags) => {
                write!(
                    formatter,
                    "graph workspace flags {flags:#06x} are unsupported"
                )
            }
            Self::Truncated => formatter.write_str("graph workspace is truncated"),
            Self::IntegerOverflow(name) => {
                write!(
                    formatter,
                    "graph workspace {name} exceeds its integer width"
                )
            }
            Self::TrailingBytes => formatter.write_str("graph workspace has trailing bytes"),
            Self::NonCanonical => formatter.write_str("graph workspace bytes are noncanonical"),
            Self::InvalidPlacementIdentity => {
                formatter.write_str("graph workspace placement identity is invalid")
            }
            Self::MissingPlacement(node) => {
                write!(formatter, "graph workspace node {node:?} has no placement")
            }
            Self::UnknownNode(node) => {
                write!(formatter, "graph workspace node {node:?} is unknown")
            }
            Self::InvalidIdentifierCursor(kind) => {
                write!(formatter, "graph workspace next {kind} identity is invalid")
            }
            Self::IdentifierExhausted(kind) => {
                write!(formatter, "graph workspace {kind} identities are exhausted")
            }
            Self::UnknownWire(wire) => {
                write!(formatter, "graph workspace wire {wire:?} is unknown")
            }
            Self::UnknownParameter { node, parameter } => {
                write!(
                    formatter,
                    "graph workspace node {node:?} parameter {parameter} is unknown"
                )
            }
            Self::ParameterTypeMismatch { expected, received } => {
                write!(
                    formatter,
                    "graph workspace parameter type {received:?} does not match {expected:?}"
                )
            }
            Self::RevisionOverflow(kind) => {
                write!(formatter, "graph workspace {kind} revision is exhausted")
            }
            Self::GraphWire(error) => write!(formatter, "embedded graph failed: {error}"),
            Self::GraphDocument(error) => write!(formatter, "graph edit failed: {error}"),
        }
    }
}

impl std::error::Error for GraphWorkspaceError {}

/// Encode one validated workspace and compute its content identity.
pub fn encode_graph_workspace(
    document: &GraphWorkspaceDocument,
) -> Result<CanonicalGraphWorkspaceEncoding, GraphWorkspaceError> {
    let graph = encode_graph_document(document.graph())?;
    if graph.digest() != document.graph_digest {
        return Err(GraphWorkspaceError::NonCanonical);
    }
    let mut encoder = Encoder::default();
    encoder.bytes(&GRAPH_WORKSPACE_MAGIC);
    encoder.u16(GRAPH_WORKSPACE_VERSION);
    encoder.u16(GRAPH_WORKSPACE_FLAGS);
    encode_limits(&mut encoder, document.limits)?;
    encoder.u64(document.revision);
    encoder.u64(document.next_node_id);
    encoder.u64(document.next_wire_id);
    encode_embedded_graph(&mut encoder, &graph)?;
    encoder.u32(
        u32::try_from(document.placements.len())
            .map_err(|_| GraphWorkspaceError::IntegerOverflow("placement count"))?,
    );
    for placement in &document.placements {
        encoder.u32(placement.node.get());
        encoder.i32(placement.x);
        encoder.i32(placement.y);
    }
    if encoder.0.len() > document.limits.maximum_workspace_bytes {
        return Err(GraphWorkspaceError::LimitExceeded("document byte length"));
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphWorkspaceEncoding {
        bytes: encoder.0,
        digest,
    })
}

/// Decode, validate, canonically re-encode, and identify an untrusted workspace.
pub fn replay_graph_workspace(
    bytes: &[u8],
    workspace_admission: GraphWorkspaceLimits,
    graph_admission: GraphLimits,
) -> Result<GraphWorkspaceReplay, GraphWorkspaceError> {
    workspace_admission.validate()?;
    if bytes.len() > workspace_admission.maximum_workspace_bytes {
        return Err(GraphWorkspaceError::LimitExceeded(
            "admitted document byte length",
        ));
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_WORKSPACE_MAGIC.len())? != GRAPH_WORKSPACE_MAGIC {
        return Err(GraphWorkspaceError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_WORKSPACE_VERSION {
        return Err(GraphWorkspaceError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_WORKSPACE_FLAGS {
        return Err(GraphWorkspaceError::UnsupportedFlags(flags));
    }
    let limits = decode_limits(&mut decoder)?;
    if !limits_within(limits, workspace_admission) {
        return Err(GraphWorkspaceError::LimitExceeded(
            "embedded admission limit",
        ));
    }
    limits.validate()?;
    if bytes.len() > limits.maximum_workspace_bytes {
        return Err(GraphWorkspaceError::LimitExceeded(
            "embedded document byte length",
        ));
    }
    let revision = decoder.u64()?;
    let next_node_id = decoder.u64()?;
    let next_wire_id = decoder.u64()?;
    let graph_length = usize::try_from(decoder.u32()?)
        .map_err(|_| GraphWorkspaceError::IntegerOverflow("embedded graph length"))?;
    let graph_bytes = decoder.take(graph_length)?;
    let graph = replay_graph_document(graph_bytes, graph_admission)?.into_document();
    let placement_count = usize::try_from(decoder.u32()?)
        .map_err(|_| GraphWorkspaceError::IntegerOverflow("placement count"))?;
    if placement_count > limits.maximum_placements {
        return Err(GraphWorkspaceError::LimitExceeded("placement count"));
    }
    let mut placements = Vec::with_capacity(placement_count);
    for _ in 0..placement_count {
        placements.push(GraphNodePlacement::new(
            GraphNodeId::new(decoder.u32()?),
            decoder.i32()?,
            decoder.i32()?,
        ));
    }
    if !decoder.is_empty() {
        return Err(GraphWorkspaceError::TrailingBytes);
    }
    let document = GraphWorkspaceDocument::try_new(
        limits,
        revision,
        next_node_id,
        next_wire_id,
        graph,
        placements,
    )?;
    let encoding = encode_graph_workspace(&document)?;
    if encoding.bytes() != bytes {
        return Err(GraphWorkspaceError::NonCanonical);
    }
    Ok(GraphWorkspaceReplay { document, encoding })
}

fn validate_placements(
    graph: &GraphDocument,
    placements: &[GraphNodePlacement],
    limits: GraphWorkspaceLimits,
) -> Result<(), GraphWorkspaceError> {
    let mut previous = None;
    for placement in placements {
        if placement.node.get() == 0 || previous == Some(placement.node) {
            return Err(GraphWorkspaceError::InvalidPlacementIdentity);
        }
        previous = Some(placement.node);
        if graph.node(placement.node).is_none() {
            return Err(GraphWorkspaceError::UnknownNode(placement.node));
        }
        validate_coordinate(placement.x, limits)?;
        validate_coordinate(placement.y, limits)?;
    }
    for node in graph.nodes() {
        if placements
            .binary_search_by_key(&node.id(), |placement| placement.node)
            .is_err()
        {
            return Err(GraphWorkspaceError::MissingPlacement(node.id()));
        }
    }
    Ok(())
}

fn validate_coordinate(
    value: i32,
    limits: GraphWorkspaceLimits,
) -> Result<(), GraphWorkspaceError> {
    if value.unsigned_abs() > limits.maximum_coordinate_magnitude {
        Err(GraphWorkspaceError::LimitExceeded("canvas coordinate"))
    } else {
        Ok(())
    }
}

fn validate_identity_cursor(
    cursor: u64,
    retained: impl Iterator<Item = u64>,
    kind: &'static str,
) -> Result<(), GraphWorkspaceError> {
    let minimum = retained
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(GraphWorkspaceError::InvalidIdentifierCursor(kind))?;
    if cursor < minimum || cursor == 0 || cursor > EXHAUSTED_U32_CURSOR {
        Err(GraphWorkspaceError::InvalidIdentifierCursor(kind))
    } else {
        Ok(())
    }
}

fn encode_limits(
    encoder: &mut Encoder,
    limits: GraphWorkspaceLimits,
) -> Result<(), GraphWorkspaceError> {
    let values = [
        limits.maximum_workspace_bytes,
        limits.maximum_placements,
        usize::try_from(limits.maximum_coordinate_magnitude)
            .map_err(|_| GraphWorkspaceError::IntegerOverflow("coordinate limit"))?,
    ];
    for value in values {
        encoder.u64(
            u64::try_from(value)
                .map_err(|_| GraphWorkspaceError::IntegerOverflow("limit value"))?,
        );
    }
    Ok(())
}

fn decode_limits(decoder: &mut Decoder<'_>) -> Result<GraphWorkspaceLimits, GraphWorkspaceError> {
    let mut values = [0_usize; WORKSPACE_LIMIT_FIELD_COUNT];
    for value in &mut values {
        *value = usize::try_from(decoder.u64()?)
            .map_err(|_| GraphWorkspaceError::IntegerOverflow("limit value"))?;
    }
    Ok(GraphWorkspaceLimits {
        maximum_workspace_bytes: values[0],
        maximum_placements: values[1],
        maximum_coordinate_magnitude: u32::try_from(values[2])
            .map_err(|_| GraphWorkspaceError::IntegerOverflow("coordinate limit"))?,
    })
}

const fn limits_within(embedded: GraphWorkspaceLimits, admission: GraphWorkspaceLimits) -> bool {
    embedded.maximum_workspace_bytes <= admission.maximum_workspace_bytes
        && embedded.maximum_placements <= admission.maximum_placements
        && embedded.maximum_coordinate_magnitude <= admission.maximum_coordinate_magnitude
}

fn encode_embedded_graph(
    encoder: &mut Encoder,
    graph: &CanonicalGraphEncoding,
) -> Result<(), GraphWorkspaceError> {
    encoder.u32(
        u32::try_from(graph.bytes().len())
            .map_err(|_| GraphWorkspaceError::IntegerOverflow("embedded graph length"))?,
    );
    encoder.bytes(graph.bytes());
    Ok(())
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

    fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
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

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphWorkspaceError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(GraphWorkspaceError::Truncated)?;
        let result = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GraphWorkspaceError::Truncated)?;
        self.cursor = end;
        Ok(result)
    }

    fn u16(&mut self) -> Result<u16, GraphWorkspaceError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| GraphWorkspaceError::Truncated)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, GraphWorkspaceError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| GraphWorkspaceError::Truncated)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn i32(&mut self) -> Result<i32, GraphWorkspaceError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| GraphWorkspaceError::Truncated)?;
        Ok(i32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, GraphWorkspaceError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| GraphWorkspaceError::Truncated)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        GraphPortId, GraphValue, RepresentativeControlSignal, analyze_graph,
        compile_representative_exact_control_graph,
    };
    use hyperreal::Rational;

    fn placements(graph: &GraphDocument) -> Vec<GraphNodePlacement> {
        graph
            .nodes()
            .iter()
            .rev()
            .map(|node| {
                let coordinate = i32::try_from(node.id().get()).unwrap() * 10;
                GraphNodePlacement::new(node.id(), coordinate, -coordinate)
            })
            .collect()
    }

    fn workspace() -> GraphWorkspaceDocument {
        let fixture = compile_representative_exact_control_graph().unwrap();
        GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            7,
            20,
            23,
            fixture.document().clone(),
            placements(fixture.document()),
        )
        .unwrap()
    }

    fn prototype(node: &NodeDefinition, label: &str) -> GraphNodePrototype {
        GraphNodePrototype::new(
            node.kind().clone(),
            label,
            node.domain(),
            node.inputs().to_vec(),
            node.outputs().to_vec(),
            node.parameters().to_vec(),
        )
    }

    #[test]
    fn canonical_workspace_replays_embedded_graph_and_sorted_integer_canvas() {
        let workspace = workspace();
        assert!(
            workspace
                .placements()
                .windows(2)
                .all(|pair| pair[0].node() < pair[1].node())
        );
        let encoding = encode_graph_workspace(&workspace).unwrap();
        let replay = replay_graph_workspace(
            encoding.bytes(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.document(), &workspace);
        assert_eq!(replay.encoding(), &encoding);
        assert_eq!(replay.document().graph_digest(), workspace.graph_digest());

        for length in 0..encoding.bytes().len() {
            assert!(
                replay_graph_workspace(
                    &encoding.bytes()[..length],
                    GraphWorkspaceLimits::interactive(),
                    GraphLimits::interactive(),
                )
                .is_err()
            );
        }
        let mut trailing = encoding.bytes().to_vec();
        trailing.push(0);
        assert_eq!(
            replay_graph_workspace(
                &trailing,
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            )
            .unwrap_err(),
            GraphWorkspaceError::TrailingBytes
        );
    }

    #[test]
    fn canvas_moves_change_only_workspace_identity_and_fail_transactionally() {
        let mut workspace = workspace();
        let graph_digest = workspace.graph_digest();
        let graph_revision = workspace.graph().revision();
        let before = encode_graph_workspace(&workspace).unwrap().digest();
        workspace.move_node(GraphNodeId::new(1), 321, 654).unwrap();
        assert_eq!(workspace.graph_digest(), graph_digest);
        assert_eq!(workspace.graph().revision(), graph_revision);
        assert_eq!(workspace.revision(), 8);
        assert_eq!(
            workspace.placement(GraphNodeId::new(1)),
            Some(GraphNodePlacement::new(GraphNodeId::new(1), 321, 654))
        );
        assert_ne!(encode_graph_workspace(&workspace).unwrap().digest(), before);

        let retained = workspace.clone();
        assert_eq!(
            workspace
                .move_node(GraphNodeId::new(1), i32::MAX, 0)
                .unwrap_err(),
            GraphWorkspaceError::LimitExceeded("canvas coordinate")
        );
        assert_eq!(workspace, retained);
    }

    #[test]
    fn wire_edits_use_monotonic_ids_and_revalidate_complete_graph() {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let mut workspace = workspace();
        let retained = workspace.clone();
        assert!(
            workspace
                .connect(
                    RepresentativeControlSignal::ClampedController.endpoint(),
                    WireEndpoint {
                        node: GraphNodeId::new(19),
                        port: GraphPortId::new(1),
                    },
                )
                .is_err()
        );
        assert_eq!(workspace, retained);

        workspace.disconnect(GraphWireId::new(22)).unwrap();
        let id = workspace
            .connect(
                RepresentativeControlSignal::PermittedOutput.endpoint(),
                WireEndpoint {
                    node: GraphNodeId::new(19),
                    port: GraphPortId::new(1),
                },
            )
            .unwrap();
        assert_eq!(id, GraphWireId::new(23));
        assert_eq!(workspace.next_wire_id(), 24);
        assert_eq!(workspace.graph().revision(), 3);
        assert_eq!(workspace.revision(), 9);
        assert_ne!(
            workspace.graph_digest(),
            fixture.simulation().graph_digest()
        );
        analyze_graph(workspace.graph(), fixture.registry().semantic_registry()).unwrap();
    }

    #[test]
    fn node_creation_and_deletion_are_atomic_and_never_reuse_identities() {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let mut workspace = workspace();
        let retained = workspace.clone();
        assert_eq!(
            workspace
                .create_node(
                    prototype(
                        fixture.document().node(GraphNodeId::new(8)).unwrap(),
                        "New scale"
                    ),
                    i32::MAX,
                    0,
                )
                .unwrap_err(),
            GraphWorkspaceError::LimitExceeded("canvas coordinate")
        );
        assert_eq!(workspace, retained);

        let created = workspace
            .create_node(
                prototype(
                    fixture.document().node(GraphNodeId::new(8)).unwrap(),
                    "New scale",
                ),
                6_000,
                40,
            )
            .unwrap();
        assert_eq!(created, GraphNodeId::new(20));
        assert_eq!(workspace.next_node_id(), 21);
        assert_eq!(workspace.next_wire_id(), 23);
        assert_eq!(workspace.graph().revision(), 2);
        assert_eq!(workspace.revision(), 8);
        assert_eq!(
            workspace.placement(created),
            Some(GraphNodePlacement::new(created, 6_000, 40))
        );

        assert_eq!(workspace.delete_node(GraphNodeId::new(18)).unwrap(), 3);
        assert!(workspace.graph().node(GraphNodeId::new(18)).is_none());
        assert_eq!(workspace.graph().wires().len(), 19);
        assert_eq!(workspace.next_node_id(), 21);
        assert_eq!(workspace.next_wire_id(), 23);
        assert_eq!(workspace.graph().revision(), 3);
        assert_eq!(workspace.revision(), 9);

        let retained = workspace.clone();
        assert_eq!(
            workspace.delete_node(GraphNodeId::new(99)).unwrap_err(),
            GraphWorkspaceError::UnknownNode(GraphNodeId::new(99))
        );
        assert_eq!(workspace, retained);
        let encoding = encode_graph_workspace(&workspace).unwrap();
        assert_eq!(
            replay_graph_workspace(
                encoding.bytes(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            )
            .unwrap()
            .document(),
            &workspace
        );
    }

    #[test]
    fn exact_parameter_edits_preserve_contract_type_and_fail_transactionally() {
        let mut workspace = workspace();
        let factor_type = workspace
            .graph()
            .node(GraphNodeId::new(8))
            .unwrap()
            .parameters()[0]
            .value()
            .value_type();
        let exact = TypedGraphValue::try_new(
            workspace.graph().schema(),
            factor_type,
            GraphValue::ExactRational(Rational::fraction(3, 2).unwrap()),
        )
        .unwrap();
        workspace
            .set_parameter(GraphNodeId::new(8), 1, exact)
            .unwrap();
        assert_eq!(workspace.graph().revision(), 2);
        assert_eq!(workspace.revision(), 8);
        assert_eq!(
            workspace
                .graph()
                .node(GraphNodeId::new(8))
                .unwrap()
                .parameters()[0]
                .value()
                .value(),
            &GraphValue::ExactRational(Rational::fraction(3, 2).unwrap())
        );

        let retained = workspace.clone();
        let other_type = workspace
            .graph()
            .node(GraphNodeId::new(9))
            .unwrap()
            .parameters()[0]
            .value()
            .value_type();
        let wrong_type = TypedGraphValue::try_new(
            workspace.graph().schema(),
            other_type,
            GraphValue::ExactRational(Rational::from(4)),
        )
        .unwrap();
        assert_eq!(
            workspace
                .set_parameter(GraphNodeId::new(8), 1, wrong_type)
                .unwrap_err(),
            GraphWorkspaceError::ParameterTypeMismatch {
                expected: factor_type,
                received: other_type,
            }
        );
        assert_eq!(workspace, retained);
        assert_eq!(
            workspace
                .set_parameter(
                    GraphNodeId::new(8),
                    99,
                    workspace
                        .graph()
                        .node(GraphNodeId::new(8))
                        .unwrap()
                        .parameters()[0]
                        .value()
                        .clone(),
                )
                .unwrap_err(),
            GraphWorkspaceError::UnknownParameter {
                node: GraphNodeId::new(8),
                parameter: 99,
            }
        );
        assert_eq!(workspace, retained);

        let encoding = encode_graph_workspace(&workspace).unwrap();
        assert_eq!(
            replay_graph_workspace(
                encoding.bytes(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            )
            .unwrap()
            .document(),
            &workspace
        );
    }

    #[test]
    fn workspace_admission_rejects_missing_nodes_bad_cursors_and_larger_policy() {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let mut missing = placements(fixture.document());
        missing.pop();
        assert!(matches!(
            GraphWorkspaceDocument::try_new(
                GraphWorkspaceLimits::interactive(),
                1,
                20,
                23,
                fixture.document().clone(),
                missing,
            ),
            Err(GraphWorkspaceError::MissingPlacement(_))
        ));
        assert_eq!(
            GraphWorkspaceDocument::try_new(
                GraphWorkspaceLimits::interactive(),
                1,
                19,
                23,
                fixture.document().clone(),
                placements(fixture.document()),
            )
            .unwrap_err(),
            GraphWorkspaceError::InvalidIdentifierCursor("node")
        );

        let encoding = encode_graph_workspace(&workspace()).unwrap();
        let mut restrictive = GraphWorkspaceLimits::interactive();
        restrictive.maximum_placements = 18;
        assert_eq!(
            replay_graph_workspace(encoding.bytes(), restrictive, GraphLimits::interactive())
                .unwrap_err(),
            GraphWorkspaceError::LimitExceeded("embedded admission limit")
        );
    }
}
