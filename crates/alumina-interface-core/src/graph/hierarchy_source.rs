//! Canonical, bounded provenance for one deterministic hierarchy flattening.
//!
//! `ALGM` binds every final node and wire in a flattened `ALGW` to either its
//! exact root identity or a stable component occurrence path and local identity.
//! Replay regenerates the map from the complete source `ALGH`; callers never
//! trust imported provenance as an independent authority.

use core::fmt;
use std::collections::BTreeSet;

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::{
    GraphHierarchyDocument, GraphHierarchyError, GraphHierarchyFlattening,
    GraphHierarchyNodeOrigin, GraphHierarchyWireOrigin, GraphNodeId, GraphWireId,
    encode_graph_hierarchy, flatten_graph_hierarchy,
};

/// Magic bytes at the beginning of each canonical hierarchy source map.
pub const GRAPH_HIERARCHY_SOURCE_MAP_MAGIC: [u8; 4] = *b"ALGM";

/// Exact canonical hierarchy source-map version implemented by this tree.
pub const GRAPH_HIERARCHY_SOURCE_MAP_VERSION: u16 = 1;

/// Hard first-release UI/source-map byte ceiling.
pub const MAX_GRAPH_HIERARCHY_SOURCE_MAP_BYTES: usize = 4 * 1024 * 1024;

const GRAPH_HIERARCHY_SOURCE_MAP_FLAGS: u16 = 0;
const SOURCE_MAP_LIMIT_FIELD_COUNT: usize = 4;
const SOURCE_MAP_FIXED_BYTES: usize =
    4 + 2 + 2 + SOURCE_MAP_LIMIT_FIELD_COUNT * 8 + 32 + 32 + 4 + 4;

/// Caller-owned and canonically embedded source-map bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphHierarchySourceMapLimits {
    /// Maximum complete canonical `ALGM` bytes.
    pub maximum_source_map_bytes: usize,
    /// Maximum total final-node provenance records.
    pub maximum_node_records: usize,
    /// Maximum total final-wire provenance records.
    pub maximum_wire_records: usize,
    /// Maximum component occurrence path depth.
    pub maximum_source_path_depth: usize,
}

impl GraphHierarchySourceMapLimits {
    /// Bounded first source-map policy matching interactive hierarchy limits.
    pub const fn interactive() -> Self {
        Self {
            maximum_source_map_bytes: MAX_GRAPH_HIERARCHY_SOURCE_MAP_BYTES,
            maximum_node_records: 4_096,
            maximum_wire_records: 8_192,
            maximum_source_path_depth: 32,
        }
    }

    fn validate(self) -> Result<(), GraphHierarchySourceMapError> {
        if self.maximum_source_map_bytes == 0
            || self.maximum_node_records == 0
            || self.maximum_wire_records == 0
            || self.maximum_source_path_depth == 0
        {
            Err(GraphHierarchySourceMapError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphHierarchySourceMapLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Canonical hierarchy source-map bytes and their exact SHA-256 identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphHierarchySourceMapEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphHierarchySourceMapEncoding {
    /// Borrow complete canonical `ALGM` bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return SHA-256 over exactly [`Self::bytes`].
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// Consume the carrier and return canonical bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Independently regenerated source map and flattening accepted from bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphHierarchySourceMapReplay {
    flattening: GraphHierarchyFlattening,
    encoding: CanonicalGraphHierarchySourceMapEncoding,
}

impl GraphHierarchySourceMapReplay {
    /// Borrow the freshly regenerated flattening and total provenance.
    pub const fn flattening(&self) -> &GraphHierarchyFlattening {
        &self.flattening
    }

    /// Borrow the exact imported and byte-for-byte regenerated encoding.
    pub const fn encoding(&self) -> &CanonicalGraphHierarchySourceMapEncoding {
        &self.encoding
    }

    /// Consume the replay result and return the regenerated flattening.
    pub fn into_flattening(self) -> GraphHierarchyFlattening {
        self.flattening
    }
}

/// Rejection while encoding or independently replaying hierarchy provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphHierarchySourceMapError {
    /// A caller or embedded policy contained zero.
    ZeroLimit,
    /// A bounded byte length, count, or source path exceeded policy.
    LimitExceeded(&'static str),
    /// Input did not begin with [`GRAPH_HIERARCHY_SOURCE_MAP_MAGIC`].
    InvalidMagic,
    /// Source-map version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved source-map flags were nonzero.
    UnsupportedFlags(u16),
    /// A fixed-width field ran past input.
    Truncated,
    /// A canonical field could not represent an in-memory value.
    IntegerOverflow(&'static str),
    /// An origin tag was neither root nor component.
    InvalidOriginTag(u8),
    /// A node, wire, or path identity was zero.
    ZeroIdentity(&'static str),
    /// Imported bytes name a different source hierarchy.
    SourceHierarchyMismatch {
        /// Digest regenerated from the caller's complete hierarchy.
        expected: Digest,
        /// Digest carried by imported source-map bytes.
        actual: Digest,
    },
    /// Imported bytes name a different flattened workspace.
    FlattenedWorkspaceMismatch {
        /// Digest regenerated by deterministic flattening.
        expected: Digest,
        /// Digest carried by imported source-map bytes.
        actual: Digest,
    },
    /// Valid fields remained after the source map.
    TrailingBytes,
    /// Decoding and fresh deterministic regeneration changed at least one byte.
    NonCanonical,
    /// The source hierarchy could not encode or flatten.
    Hierarchy(GraphHierarchyError),
}

impl From<GraphHierarchyError> for GraphHierarchySourceMapError {
    fn from(value: GraphHierarchyError) -> Self {
        Self::Hierarchy(value)
    }
}

impl fmt::Display for GraphHierarchySourceMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => {
                formatter.write_str("graph hierarchy source-map policy contains zero")
            }
            Self::LimitExceeded(name) => {
                write!(
                    formatter,
                    "graph hierarchy source-map {name} exceeds policy"
                )
            }
            Self::InvalidMagic => {
                formatter.write_str("graph hierarchy source-map magic is invalid")
            }
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "graph hierarchy source-map version {version} is unsupported"
            ),
            Self::UnsupportedFlags(flags) => write!(
                formatter,
                "graph hierarchy source-map flags {flags:#06x} are unsupported"
            ),
            Self::Truncated => formatter.write_str("graph hierarchy source map is truncated"),
            Self::IntegerOverflow(name) => write!(
                formatter,
                "graph hierarchy source-map {name} exceeds its integer width"
            ),
            Self::InvalidOriginTag(tag) => {
                write!(
                    formatter,
                    "graph hierarchy source-map origin tag {tag} is invalid"
                )
            }
            Self::ZeroIdentity(name) => {
                write!(
                    formatter,
                    "graph hierarchy source-map {name} identity is zero"
                )
            }
            Self::SourceHierarchyMismatch { expected, actual } => write!(
                formatter,
                "graph hierarchy source map names hierarchy {actual:?}, expected {expected:?}"
            ),
            Self::FlattenedWorkspaceMismatch { expected, actual } => write!(
                formatter,
                "graph hierarchy source map names workspace {actual:?}, expected {expected:?}"
            ),
            Self::TrailingBytes => {
                formatter.write_str("graph hierarchy source map has trailing bytes")
            }
            Self::NonCanonical => {
                formatter.write_str("graph hierarchy source-map bytes are noncanonical")
            }
            Self::Hierarchy(error) => write!(formatter, "graph hierarchy source failed: {error}"),
        }
    }
}

impl std::error::Error for GraphHierarchySourceMapError {}

/// Encode total deterministic node/wire provenance for one exact flattening.
pub fn encode_graph_hierarchy_source_map(
    flattening: &GraphHierarchyFlattening,
    limits: GraphHierarchySourceMapLimits,
) -> Result<CanonicalGraphHierarchySourceMapEncoding, GraphHierarchySourceMapError> {
    limits.validate()?;
    validate_flattening_provenance(flattening, limits)?;
    let node_count = to_u32(flattening.node_provenance().len(), "node record count")?;
    let wire_count = to_u32(flattening.wire_provenance().len(), "wire record count")?;
    let encoded_length = source_map_encoded_length(flattening)?;
    if encoded_length > limits.maximum_source_map_bytes {
        return Err(GraphHierarchySourceMapError::LimitExceeded(
            "document byte length",
        ));
    }
    let mut encoder = Encoder::with_capacity(encoded_length);
    encoder.bytes(&GRAPH_HIERARCHY_SOURCE_MAP_MAGIC);
    encoder.u16(GRAPH_HIERARCHY_SOURCE_MAP_VERSION);
    encoder.u16(GRAPH_HIERARCHY_SOURCE_MAP_FLAGS);
    encode_limits(&mut encoder, limits)?;
    encoder.digest(flattening.source_digest());
    encoder.digest(flattening.encoding().digest());
    encoder.u32(node_count);
    for mapping in flattening.node_provenance() {
        encode_node_origin(&mut encoder, mapping.origin(), limits)?;
        encoder.u32(mapping.flattened_node().get());
    }
    encoder.u32(wire_count);
    for mapping in flattening.wire_provenance() {
        encode_wire_origin(&mut encoder, mapping.origin(), limits)?;
        encoder.u32(mapping.flattened_wire().get());
    }
    if encoder.0.len() != encoded_length {
        return Err(GraphHierarchySourceMapError::NonCanonical);
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphHierarchySourceMapEncoding {
        bytes: encoder.0,
        digest,
    })
}

fn source_map_encoded_length(
    flattening: &GraphHierarchyFlattening,
) -> Result<usize, GraphHierarchySourceMapError> {
    let mut length = SOURCE_MAP_FIXED_BYTES;
    for mapping in flattening.node_provenance() {
        add_record_length(&mut length, mapping.origin().source_path())?;
    }
    for mapping in flattening.wire_provenance() {
        add_record_length(&mut length, mapping.origin().source_path())?;
    }
    Ok(length)
}

fn add_record_length(
    length: &mut usize,
    source_path: Option<&[GraphNodeId]>,
) -> Result<(), GraphHierarchySourceMapError> {
    let origin_length = if let Some(source_path) = source_path {
        source_path
            .len()
            .checked_mul(4)
            .and_then(|path_bytes| path_bytes.checked_add(1 + 4 + 32 + 4))
    } else {
        Some(1 + 4)
    }
    .ok_or(GraphHierarchySourceMapError::IntegerOverflow(
        "document byte length",
    ))?;
    *length = length
        .checked_add(origin_length)
        .and_then(|value| value.checked_add(4))
        .ok_or(GraphHierarchySourceMapError::IntegerOverflow(
            "document byte length",
        ))?;
    Ok(())
}

/// Bound, decode, and accept source-map bytes only when a fresh flattening of
/// `hierarchy` regenerates every byte.
pub fn replay_graph_hierarchy_source_map(
    bytes: &[u8],
    hierarchy: &GraphHierarchyDocument,
    admission: GraphHierarchySourceMapLimits,
) -> Result<GraphHierarchySourceMapReplay, GraphHierarchySourceMapError> {
    admission.validate()?;
    if bytes.len() > admission.maximum_source_map_bytes {
        return Err(GraphHierarchySourceMapError::LimitExceeded(
            "admitted document byte length",
        ));
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_HIERARCHY_SOURCE_MAP_MAGIC.len())? != GRAPH_HIERARCHY_SOURCE_MAP_MAGIC {
        return Err(GraphHierarchySourceMapError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_HIERARCHY_SOURCE_MAP_VERSION {
        return Err(GraphHierarchySourceMapError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_HIERARCHY_SOURCE_MAP_FLAGS {
        return Err(GraphHierarchySourceMapError::UnsupportedFlags(flags));
    }
    let limits = decode_limits(&mut decoder)?;
    limits.validate()?;
    if !limits_within(limits, admission) {
        return Err(GraphHierarchySourceMapError::LimitExceeded(
            "embedded admission limit",
        ));
    }
    if bytes.len() > limits.maximum_source_map_bytes {
        return Err(GraphHierarchySourceMapError::LimitExceeded(
            "embedded document byte length",
        ));
    }
    let expected_source = encode_graph_hierarchy(hierarchy)?.digest();
    let actual_source = decoder.digest()?;
    if actual_source != expected_source {
        return Err(GraphHierarchySourceMapError::SourceHierarchyMismatch {
            expected: expected_source,
            actual: actual_source,
        });
    }
    let flattening = flatten_graph_hierarchy(hierarchy)?;
    if flattening.source_digest() != expected_source {
        return Err(GraphHierarchySourceMapError::NonCanonical);
    }
    let expected_workspace = flattening.encoding().digest();
    let actual_workspace = decoder.digest()?;
    if actual_workspace != expected_workspace {
        return Err(GraphHierarchySourceMapError::FlattenedWorkspaceMismatch {
            expected: expected_workspace,
            actual: actual_workspace,
        });
    }

    let node_count = decoder.count(limits.maximum_node_records, "node record count")?;
    if node_count != flattening.node_provenance().len() {
        return Err(GraphHierarchySourceMapError::NonCanonical);
    }
    let mut node_origins = BTreeSet::new();
    let mut prior_node = None;
    for _ in 0..node_count {
        let origin = decode_node_origin(&mut decoder, limits)?;
        if !node_origins.insert(origin) {
            return Err(GraphHierarchySourceMapError::NonCanonical);
        }
        let flattened = nonzero_node(decoder.u32()?, "flattened node")?;
        if prior_node.is_some_and(|prior| prior >= flattened) {
            return Err(GraphHierarchySourceMapError::NonCanonical);
        }
        prior_node = Some(flattened);
    }

    let wire_count = decoder.count(limits.maximum_wire_records, "wire record count")?;
    if wire_count != flattening.wire_provenance().len() {
        return Err(GraphHierarchySourceMapError::NonCanonical);
    }
    let mut wire_origins = BTreeSet::new();
    let mut prior_wire = None;
    for _ in 0..wire_count {
        let origin = decode_wire_origin(&mut decoder, limits)?;
        if !wire_origins.insert(origin) {
            return Err(GraphHierarchySourceMapError::NonCanonical);
        }
        let flattened = nonzero_wire(decoder.u32()?, "flattened wire")?;
        if prior_wire.is_some_and(|prior| prior >= flattened) {
            return Err(GraphHierarchySourceMapError::NonCanonical);
        }
        prior_wire = Some(flattened);
    }
    if !decoder.is_empty() {
        return Err(GraphHierarchySourceMapError::TrailingBytes);
    }

    let encoding = encode_graph_hierarchy_source_map(&flattening, limits)?;
    if encoding.bytes() != bytes {
        return Err(GraphHierarchySourceMapError::NonCanonical);
    }
    Ok(GraphHierarchySourceMapReplay {
        flattening,
        encoding,
    })
}

fn validate_flattening_provenance(
    flattening: &GraphHierarchyFlattening,
    limits: GraphHierarchySourceMapLimits,
) -> Result<(), GraphHierarchySourceMapError> {
    let nodes = flattening.node_provenance();
    let wires = flattening.wire_provenance();
    if nodes.len() > limits.maximum_node_records {
        return Err(GraphHierarchySourceMapError::LimitExceeded(
            "node record count",
        ));
    }
    if wires.len() > limits.maximum_wire_records {
        return Err(GraphHierarchySourceMapError::LimitExceeded(
            "wire record count",
        ));
    }
    if nodes.len() != flattening.workspace().graph().nodes().len()
        || wires.len() != flattening.workspace().graph().wires().len()
    {
        return Err(GraphHierarchySourceMapError::NonCanonical);
    }
    let mut node_origins = BTreeSet::new();
    let mut prior_node = None;
    for mapping in nodes {
        validate_node_origin(mapping.origin(), limits)?;
        if !node_origins.insert(mapping.origin()) {
            return Err(GraphHierarchySourceMapError::NonCanonical);
        }
        let flattened = mapping.flattened_node();
        if flattened.get() == 0 || prior_node.is_some_and(|prior| prior >= flattened) {
            return Err(GraphHierarchySourceMapError::NonCanonical);
        }
        prior_node = Some(flattened);
    }
    let mut wire_origins = BTreeSet::new();
    let mut prior_wire = None;
    for mapping in wires {
        validate_wire_origin(mapping.origin(), limits)?;
        if !wire_origins.insert(mapping.origin()) {
            return Err(GraphHierarchySourceMapError::NonCanonical);
        }
        let flattened = mapping.flattened_wire();
        if flattened.get() == 0 || prior_wire.is_some_and(|prior| prior >= flattened) {
            return Err(GraphHierarchySourceMapError::NonCanonical);
        }
        prior_wire = Some(flattened);
    }
    Ok(())
}

fn encode_node_origin(
    encoder: &mut Encoder,
    origin: &GraphHierarchyNodeOrigin,
    limits: GraphHierarchySourceMapLimits,
) -> Result<(), GraphHierarchySourceMapError> {
    validate_node_origin(origin, limits)?;
    match origin {
        GraphHierarchyNodeOrigin::Root(node) => {
            encoder.u8(0);
            encoder.u32(node.get());
        }
        GraphHierarchyNodeOrigin::Component {
            source_path,
            component,
            node,
        } => {
            encoder.u8(1);
            encode_source_path(encoder, source_path)?;
            encoder.digest(*component);
            encoder.u32(node.get());
        }
    }
    Ok(())
}

fn encode_wire_origin(
    encoder: &mut Encoder,
    origin: &GraphHierarchyWireOrigin,
    limits: GraphHierarchySourceMapLimits,
) -> Result<(), GraphHierarchySourceMapError> {
    validate_wire_origin(origin, limits)?;
    match origin {
        GraphHierarchyWireOrigin::Root(wire) => {
            encoder.u8(0);
            encoder.u32(wire.get());
        }
        GraphHierarchyWireOrigin::Component {
            source_path,
            component,
            wire,
        } => {
            encoder.u8(1);
            encode_source_path(encoder, source_path)?;
            encoder.digest(*component);
            encoder.u32(wire.get());
        }
    }
    Ok(())
}

fn decode_node_origin(
    decoder: &mut Decoder<'_>,
    limits: GraphHierarchySourceMapLimits,
) -> Result<GraphHierarchyNodeOrigin, GraphHierarchySourceMapError> {
    match decoder.u8()? {
        0 => Ok(GraphHierarchyNodeOrigin::Root(nonzero_node(
            decoder.u32()?,
            "root node",
        )?)),
        1 => Ok(GraphHierarchyNodeOrigin::Component {
            source_path: decode_source_path(decoder, limits)?,
            component: decoder.digest()?,
            node: nonzero_node(decoder.u32()?, "component node")?,
        }),
        tag => Err(GraphHierarchySourceMapError::InvalidOriginTag(tag)),
    }
}

fn decode_wire_origin(
    decoder: &mut Decoder<'_>,
    limits: GraphHierarchySourceMapLimits,
) -> Result<GraphHierarchyWireOrigin, GraphHierarchySourceMapError> {
    match decoder.u8()? {
        0 => Ok(GraphHierarchyWireOrigin::Root(nonzero_wire(
            decoder.u32()?,
            "root wire",
        )?)),
        1 => Ok(GraphHierarchyWireOrigin::Component {
            source_path: decode_source_path(decoder, limits)?,
            component: decoder.digest()?,
            wire: nonzero_wire(decoder.u32()?, "component wire")?,
        }),
        tag => Err(GraphHierarchySourceMapError::InvalidOriginTag(tag)),
    }
}

fn validate_node_origin(
    origin: &GraphHierarchyNodeOrigin,
    limits: GraphHierarchySourceMapLimits,
) -> Result<(), GraphHierarchySourceMapError> {
    match origin {
        GraphHierarchyNodeOrigin::Root(node) => {
            nonzero_node(node.get(), "root node")?;
        }
        GraphHierarchyNodeOrigin::Component {
            source_path, node, ..
        } => {
            validate_source_path(source_path, limits)?;
            nonzero_node(node.get(), "component node")?;
        }
    }
    Ok(())
}

fn validate_wire_origin(
    origin: &GraphHierarchyWireOrigin,
    limits: GraphHierarchySourceMapLimits,
) -> Result<(), GraphHierarchySourceMapError> {
    match origin {
        GraphHierarchyWireOrigin::Root(wire) => {
            nonzero_wire(wire.get(), "root wire")?;
        }
        GraphHierarchyWireOrigin::Component {
            source_path, wire, ..
        } => {
            validate_source_path(source_path, limits)?;
            nonzero_wire(wire.get(), "component wire")?;
        }
    }
    Ok(())
}

fn validate_source_path(
    source_path: &[GraphNodeId],
    limits: GraphHierarchySourceMapLimits,
) -> Result<(), GraphHierarchySourceMapError> {
    if source_path.is_empty() {
        return Err(GraphHierarchySourceMapError::ZeroIdentity("source path"));
    }
    if source_path.len() > limits.maximum_source_path_depth {
        return Err(GraphHierarchySourceMapError::LimitExceeded(
            "source path depth",
        ));
    }
    to_u32(source_path.len(), "source path depth")?;
    if source_path.iter().any(|node| node.get() == 0) {
        return Err(GraphHierarchySourceMapError::ZeroIdentity(
            "source path node",
        ));
    }
    Ok(())
}

fn encode_source_path(
    encoder: &mut Encoder,
    source_path: &[GraphNodeId],
) -> Result<(), GraphHierarchySourceMapError> {
    encoder.u32(to_u32(source_path.len(), "source path depth")?);
    for node in source_path {
        encoder.u32(node.get());
    }
    Ok(())
}

fn decode_source_path(
    decoder: &mut Decoder<'_>,
    limits: GraphHierarchySourceMapLimits,
) -> Result<Vec<GraphNodeId>, GraphHierarchySourceMapError> {
    let count = decoder.count(limits.maximum_source_path_depth, "source path depth")?;
    if count == 0 {
        return Err(GraphHierarchySourceMapError::ZeroIdentity("source path"));
    }
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(nonzero_node(decoder.u32()?, "source path node")?);
    }
    Ok(result)
}

fn nonzero_node(
    value: u32,
    name: &'static str,
) -> Result<GraphNodeId, GraphHierarchySourceMapError> {
    if value == 0 {
        Err(GraphHierarchySourceMapError::ZeroIdentity(name))
    } else {
        Ok(GraphNodeId::new(value))
    }
}

fn nonzero_wire(
    value: u32,
    name: &'static str,
) -> Result<GraphWireId, GraphHierarchySourceMapError> {
    if value == 0 {
        Err(GraphHierarchySourceMapError::ZeroIdentity(name))
    } else {
        Ok(GraphWireId::new(value))
    }
}

fn encode_limits(
    encoder: &mut Encoder,
    limits: GraphHierarchySourceMapLimits,
) -> Result<(), GraphHierarchySourceMapError> {
    for value in [
        limits.maximum_source_map_bytes,
        limits.maximum_node_records,
        limits.maximum_wire_records,
        limits.maximum_source_path_depth,
    ] {
        encoder.u64(
            u64::try_from(value)
                .map_err(|_| GraphHierarchySourceMapError::IntegerOverflow("limit value"))?,
        );
    }
    Ok(())
}

fn decode_limits(
    decoder: &mut Decoder<'_>,
) -> Result<GraphHierarchySourceMapLimits, GraphHierarchySourceMapError> {
    let mut values = [0_usize; SOURCE_MAP_LIMIT_FIELD_COUNT];
    for value in &mut values {
        *value = usize::try_from(decoder.u64()?)
            .map_err(|_| GraphHierarchySourceMapError::IntegerOverflow("limit value"))?;
    }
    Ok(GraphHierarchySourceMapLimits {
        maximum_source_map_bytes: values[0],
        maximum_node_records: values[1],
        maximum_wire_records: values[2],
        maximum_source_path_depth: values[3],
    })
}

const fn limits_within(
    embedded: GraphHierarchySourceMapLimits,
    admission: GraphHierarchySourceMapLimits,
) -> bool {
    embedded.maximum_source_map_bytes <= admission.maximum_source_map_bytes
        && embedded.maximum_node_records <= admission.maximum_node_records
        && embedded.maximum_wire_records <= admission.maximum_wire_records
        && embedded.maximum_source_path_depth <= admission.maximum_source_path_depth
}

fn to_u32(value: usize, name: &'static str) -> Result<u32, GraphHierarchySourceMapError> {
    u32::try_from(value).map_err(|_| GraphHierarchySourceMapError::IntegerOverflow(name))
}

struct Encoder(Vec<u8>);

impl Encoder {
    fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    fn bytes(&mut self, value: &[u8]) {
        self.0.extend_from_slice(value);
    }

    fn digest(&mut self, value: Digest) {
        self.bytes(&value.0);
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
}

struct Decoder<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphHierarchySourceMapError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(GraphHierarchySourceMapError::Truncated)?;
        let result = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GraphHierarchySourceMapError::Truncated)?;
        self.cursor = end;
        Ok(result)
    }

    fn digest(&mut self) -> Result<Digest, GraphHierarchySourceMapError> {
        let bytes: [u8; 32] = self
            .take(32)?
            .try_into()
            .map_err(|_| GraphHierarchySourceMapError::Truncated)?;
        Ok(Digest(bytes))
    }

    fn u8(&mut self) -> Result<u8, GraphHierarchySourceMapError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, GraphHierarchySourceMapError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| GraphHierarchySourceMapError::Truncated)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, GraphHierarchySourceMapError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| GraphHierarchySourceMapError::Truncated)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, GraphHierarchySourceMapError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| GraphHierarchySourceMapError::Truncated)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn count(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<usize, GraphHierarchySourceMapError> {
        let count = usize::try_from(self.u32()?)
            .map_err(|_| GraphHierarchySourceMapError::IntegerOverflow(name))?;
        if count > maximum {
            Err(GraphHierarchySourceMapError::LimitExceeded(name))
        } else {
            Ok(count)
        }
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        GraphComponentDocument, GraphComponentInstance, GraphComponentLimits, GraphDocument,
        GraphHierarchyLimits, GraphNodePlacement, GraphWorkspaceDocument, GraphWorkspaceLimits,
        NodeDefinition, WireDefinition, WireEndpoint, compile_representative_exact_control_graph,
        encode_graph_component, graph_component_instance_prototype,
    };

    fn nested_hierarchy() -> GraphHierarchyDocument {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let graph = fixture.document().clone();
        let next_node_id = u64::from(graph.nodes().last().unwrap().id().get()) + 1;
        let next_wire_id = u64::from(graph.wires().last().unwrap().id().get()) + 1;
        let placements = graph
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let offset = i32::try_from(index).unwrap() * 20;
                GraphNodePlacement::new(node.id(), offset, offset)
            })
            .collect();
        let leaf_workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            next_node_id,
            next_wire_id,
            graph,
            placements,
        )
        .unwrap();
        let leaf = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.source_map_leaf",
            1,
            1,
            1,
            leaf_workspace,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let leaf_digest = encode_graph_component(&leaf).unwrap().digest();

        let nested_id = GraphNodeId::new(1);
        let nested_prototype = graph_component_instance_prototype(&leaf, "Nested leaf").unwrap();
        let nested_node = NodeDefinition::new(
            nested_id,
            nested_prototype.kind().clone(),
            "Nested leaf",
            nested_prototype.domain(),
            nested_prototype.inputs().to_vec(),
            nested_prototype.outputs().to_vec(),
            nested_prototype.parameters().to_vec(),
        );
        let wrapper_graph = GraphDocument::try_new(
            1,
            leaf.workspace().graph().schema().clone(),
            leaf.workspace().graph().clocks().to_vec(),
            vec![nested_node],
            Vec::new(),
        )
        .unwrap();
        let wrapper_workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            2,
            1,
            wrapper_graph,
            vec![GraphNodePlacement::new(nested_id, 10, 10)],
        )
        .unwrap();
        let wrapper = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.source_map_wrapper",
            1,
            1,
            1,
            wrapper_workspace,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let wrapper_digest = encode_graph_component(&wrapper).unwrap().digest();

        let root_id = GraphNodeId::new(1);
        let root_prototype = graph_component_instance_prototype(&wrapper, "Root wrapper").unwrap();
        let root_node = NodeDefinition::new(
            root_id,
            root_prototype.kind().clone(),
            "Root wrapper",
            root_prototype.domain(),
            root_prototype.inputs().to_vec(),
            root_prototype.outputs().to_vec(),
            root_prototype.parameters().to_vec(),
        );
        let witness_wire = leaf
            .workspace()
            .graph()
            .wires()
            .iter()
            .find(|wire| wire.source().node != wire.target().node)
            .unwrap();
        let source_definition = leaf
            .workspace()
            .graph()
            .node(witness_wire.source().node)
            .unwrap();
        let target_definition = leaf
            .workspace()
            .graph()
            .node(witness_wire.target().node)
            .unwrap();
        let root_source_id = GraphNodeId::new(2);
        let root_target_id = GraphNodeId::new(3);
        let root_source = NodeDefinition::new(
            root_source_id,
            source_definition.kind().clone(),
            "Root provenance source",
            source_definition.domain(),
            source_definition.inputs().to_vec(),
            source_definition.outputs().to_vec(),
            source_definition.parameters().to_vec(),
        );
        let root_target = NodeDefinition::new(
            root_target_id,
            target_definition.kind().clone(),
            "Root provenance target",
            target_definition.domain(),
            target_definition.inputs().to_vec(),
            target_definition.outputs().to_vec(),
            target_definition.parameters().to_vec(),
        );
        let root_graph = GraphDocument::try_new(
            1,
            leaf.workspace().graph().schema().clone(),
            leaf.workspace().graph().clocks().to_vec(),
            vec![root_node, root_source, root_target],
            vec![WireDefinition::new(
                GraphWireId::new(1),
                WireEndpoint {
                    node: root_source_id,
                    port: witness_wire.source().port,
                },
                WireEndpoint {
                    node: root_target_id,
                    port: witness_wire.target().port,
                },
            )],
        )
        .unwrap();
        let root = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            4,
            2,
            root_graph,
            vec![
                GraphNodePlacement::new(root_id, 30, 30),
                GraphNodePlacement::new(root_source_id, 0, 0),
                GraphNodePlacement::new(root_target_id, 600, 0),
            ],
        )
        .unwrap();
        GraphHierarchyDocument::try_new(
            GraphHierarchyLimits::interactive(),
            1,
            root,
            vec![leaf, wrapper],
            vec![
                GraphComponentInstance::root(root_id, wrapper_digest),
                GraphComponentInstance::nested(wrapper_digest, nested_id, leaf_digest),
            ],
        )
        .unwrap()
    }

    #[test]
    fn canonical_total_source_map_round_trips_by_fresh_flattening() {
        let hierarchy = nested_hierarchy();
        let flattening = flatten_graph_hierarchy(&hierarchy).unwrap();
        assert_eq!(flattening.node_provenance().len(), 23);
        assert_eq!(flattening.wire_provenance().len(), 26);
        assert_eq!(
            flattening
                .node_provenance()
                .iter()
                .filter(|mapping| mapping.origin().source_path().is_none())
                .count(),
            2
        );
        assert_eq!(
            flattening
                .node_provenance()
                .iter()
                .filter(|mapping| {
                    mapping.origin().source_path()
                        == Some([GraphNodeId::new(1), GraphNodeId::new(1)].as_slice())
                })
                .count(),
            21
        );
        assert_eq!(
            flattening
                .wire_provenance()
                .iter()
                .filter(|mapping| mapping.origin().source_path().is_none())
                .count(),
            1
        );
        assert_eq!(
            flattening
                .wire_provenance()
                .iter()
                .filter(|mapping| {
                    mapping.origin().source_path()
                        == Some([GraphNodeId::new(1), GraphNodeId::new(1)].as_slice())
                })
                .count(),
            25
        );

        let limits = GraphHierarchySourceMapLimits::interactive();
        let encoding = encode_graph_hierarchy_source_map(&flattening, limits).unwrap();
        assert_eq!(encoding.bytes().len(), 2_577);
        assert_eq!(
            encoding.digest().0,
            [
                0xdf, 0xfc, 0x80, 0x37, 0x79, 0xd9, 0x0c, 0x51, 0x70, 0xb0, 0xbe, 0x15, 0x78, 0xa8,
                0x66, 0x6e, 0x84, 0x87, 0x8c, 0x84, 0x7e, 0x80, 0xb0, 0xed, 0x47, 0x3b, 0xbb, 0x54,
                0x51, 0x26, 0x73, 0x59,
            ]
        );
        let replay =
            replay_graph_hierarchy_source_map(encoding.bytes(), &hierarchy, limits).unwrap();
        assert_eq!(replay.encoding(), &encoding);
        assert_eq!(replay.flattening(), &flattening);
    }

    #[test]
    fn corruption_identity_and_admission_fail_before_provenance_is_accepted() {
        let hierarchy = nested_hierarchy();
        let limits = GraphHierarchySourceMapLimits::interactive();
        let flattening = flatten_graph_hierarchy(&hierarchy).unwrap();
        let encoding = encode_graph_hierarchy_source_map(&flattening, limits).unwrap();

        let mut bad_magic = encoding.bytes().to_vec();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            replay_graph_hierarchy_source_map(&bad_magic, &hierarchy, limits),
            Err(GraphHierarchySourceMapError::InvalidMagic)
        );
        let mut bad_version = encoding.bytes().to_vec();
        bad_version[4..6].copy_from_slice(&0_u16.to_le_bytes());
        assert_eq!(
            replay_graph_hierarchy_source_map(&bad_version, &hierarchy, limits),
            Err(GraphHierarchySourceMapError::UnsupportedVersion(0))
        );
        let mut wrong_source = encoding.bytes().to_vec();
        let source_offset = 8 + SOURCE_MAP_LIMIT_FIELD_COUNT * 8;
        wrong_source[source_offset] ^= 1;
        assert!(matches!(
            replay_graph_hierarchy_source_map(&wrong_source, &hierarchy, limits),
            Err(GraphHierarchySourceMapError::SourceHierarchyMismatch { .. })
        ));
        let mut wrong_workspace = encoding.bytes().to_vec();
        wrong_workspace[source_offset + 32] ^= 1;
        assert!(matches!(
            replay_graph_hierarchy_source_map(&wrong_workspace, &hierarchy, limits),
            Err(GraphHierarchySourceMapError::FlattenedWorkspaceMismatch { .. })
        ));

        let first_origin_tag = source_offset + 64 + 4;
        let mut bad_origin = encoding.bytes().to_vec();
        bad_origin[first_origin_tag] = 2;
        assert_eq!(
            replay_graph_hierarchy_source_map(&bad_origin, &hierarchy, limits),
            Err(GraphHierarchySourceMapError::InvalidOriginTag(2))
        );
        let mut trailing = encoding.bytes().to_vec();
        trailing.push(0);
        assert_eq!(
            replay_graph_hierarchy_source_map(&trailing, &hierarchy, limits),
            Err(GraphHierarchySourceMapError::TrailingBytes)
        );
        let mut tighter = limits;
        tighter.maximum_source_map_bytes = encoding.bytes().len() - 1;
        assert_eq!(
            encode_graph_hierarchy_source_map(&flattening, tighter),
            Err(GraphHierarchySourceMapError::LimitExceeded(
                "document byte length"
            ))
        );
        assert_eq!(
            replay_graph_hierarchy_source_map(encoding.bytes(), &hierarchy, tighter),
            Err(GraphHierarchySourceMapError::LimitExceeded(
                "admitted document byte length"
            ))
        );
        let mut shallow = limits;
        shallow.maximum_source_path_depth = 1;
        assert_eq!(
            encode_graph_hierarchy_source_map(&flattening, shallow),
            Err(GraphHierarchySourceMapError::LimitExceeded(
                "source path depth"
            ))
        );
    }
}
