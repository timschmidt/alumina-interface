//! Canonical, bounded graph-document encoding and content-identity replay.
//!
//! The format is deliberately independent of serde and JavaScript numbers.
//! Every integer is fixed-width little-endian, exact rationals retain signed
//! reduced decimal magnitudes, collections carry bounded `u32` lengths, and a
//! decoder must reconstruct and re-encode the document byte-for-byte before
//! accepting it.

use core::fmt;
use std::str;

use alumina_protocol::{DeviceId, Digest};
use alumina_storage::sha256;
use hyperreal::Rational;

use super::{
    BaseDimensions, ClockDefinition, ClockKind, ExecutionDomain, GraphClockId, GraphDocument,
    GraphDocumentError, GraphLimits, GraphNodeId, GraphPortId, GraphSchema, GraphSchemaError,
    GraphTypeId, GraphValue, GraphWireId, JobGraphHandle, NodeDefinition, NodeKind, NodeParameter,
    PortDefinition, RecordField, RecordFieldId, RecordValueField, ResourceClassId,
    ResourceGraphHandle, TypeDefinition, TypeKind, TypedGraphValue, UnitDefinition, UnitId,
    WireDefinition, WireEndpoint,
};

/// Magic bytes at the beginning of each canonical graph document.
pub const GRAPH_DOCUMENT_MAGIC: [u8; 4] = *b"ALGR";

/// Exact canonical graph-document format implemented by this source tree.
pub const GRAPH_DOCUMENT_VERSION: u16 = 1;

const GRAPH_DOCUMENT_FLAGS: u16 = 0;
const LIMIT_FIELD_COUNT: usize = 17;
const STABLE_NAME_BYTES: usize = 64;
const UNIT_SYMBOL_BYTES: usize = 32;

/// Canonical graph bytes paired with their SHA-256 content identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphEncoding {
    /// Borrow the complete canonical bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the SHA-256 identity of exactly [`Self::bytes`].
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// Consume the carrier and return its canonical bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Successfully replayed document and its verified canonical identity.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphReplay {
    document: GraphDocument,
    encoding: CanonicalGraphEncoding,
}

impl GraphReplay {
    /// Borrow the reconstructed and fully validated document.
    pub const fn document(&self) -> &GraphDocument {
        &self.document
    }

    /// Borrow the byte-for-byte verified canonical encoding.
    pub const fn encoding(&self) -> &CanonicalGraphEncoding {
        &self.encoding
    }

    /// Consume the replay result and return the document.
    pub fn into_document(self) -> GraphDocument {
        self.document
    }
}

/// Rejection at the canonical graph encoding/replay boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphWireError {
    /// Input did not begin with [`GRAPH_DOCUMENT_MAGIC`].
    InvalidMagic,
    /// The format version is not implemented here.
    UnsupportedVersion(u16),
    /// Reserved format flags were nonzero.
    UnsupportedFlags(u16),
    /// A fixed-width or declared-length field ran past the input.
    Truncated,
    /// A count, length, or embedded policy exceeded decoder admission policy.
    LimitExceeded(&'static str),
    /// An integer could not be represented by the canonical field width.
    IntegerOverflow(&'static str),
    /// A stable string or literal was not UTF-8.
    InvalidUtf8(&'static str),
    /// A discriminant had no V1 meaning.
    InvalidTag(&'static str, u8),
    /// An exact rational was malformed, unreduced, or noncanonical.
    InvalidRational,
    /// A type-directed literal could not be represented.
    InvalidValue,
    /// Valid fields remained after the decoded graph document.
    TrailingBytes,
    /// Decoding succeeded structurally but re-encoding changed any byte.
    NonCanonical,
    /// Value/type registry validation rejected the decoded content.
    Schema(GraphSchemaError),
    /// Structural document validation rejected the decoded content.
    Document(GraphDocumentError),
}

impl From<GraphSchemaError> for GraphWireError {
    fn from(value: GraphSchemaError) -> Self {
        Self::Schema(value)
    }
}

impl From<GraphDocumentError> for GraphWireError {
    fn from(value: GraphDocumentError) -> Self {
        Self::Document(value)
    }
}

impl fmt::Display for GraphWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => formatter.write_str("graph document magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "graph document version {version} is unsupported")
            }
            Self::UnsupportedFlags(flags) => {
                write!(
                    formatter,
                    "graph document flags {flags:#06x} are unsupported"
                )
            }
            Self::Truncated => formatter.write_str("graph document is truncated"),
            Self::LimitExceeded(name) => write!(formatter, "graph wire {name} exceeds policy"),
            Self::IntegerOverflow(name) => {
                write!(formatter, "graph wire {name} exceeds its integer width")
            }
            Self::InvalidUtf8(name) => write!(formatter, "graph wire {name} is not UTF-8"),
            Self::InvalidTag(name, tag) => {
                write!(formatter, "graph wire {name} tag {tag} is invalid")
            }
            Self::InvalidRational => formatter.write_str("graph wire rational is noncanonical"),
            Self::InvalidValue => formatter.write_str("graph wire value contradicts its type"),
            Self::TrailingBytes => formatter.write_str("graph document has trailing bytes"),
            Self::NonCanonical => formatter.write_str("graph document bytes are not canonical"),
            Self::Schema(error) => write!(formatter, "graph wire schema rejected: {error}"),
            Self::Document(error) => write!(formatter, "graph wire document rejected: {error}"),
        }
    }
}

impl std::error::Error for GraphWireError {}

/// Encode one already validated graph document and compute its content identity.
pub fn encode_graph_document(
    document: &GraphDocument,
) -> Result<CanonicalGraphEncoding, GraphWireError> {
    let mut encoder = Encoder::default();
    encoder.bytes(&GRAPH_DOCUMENT_MAGIC);
    encoder.u16(GRAPH_DOCUMENT_VERSION);
    encoder.u16(GRAPH_DOCUMENT_FLAGS);
    encode_limits(&mut encoder, document.schema().limits())?;
    encoder.u64(document.revision());
    encode_schema(&mut encoder, document.schema())?;
    encode_clocks(&mut encoder, document.clocks())?;
    encode_nodes(&mut encoder, document.schema(), document.nodes())?;
    encode_wires(&mut encoder, document.wires())?;
    if encoder.0.len() > document.schema().limits().maximum_document_bytes {
        return Err(GraphWireError::LimitExceeded("document byte length"));
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphEncoding {
        bytes: encoder.0,
        digest,
    })
}

/// Decode, validate, canonically re-encode, and identify an untrusted graph.
///
/// Embedded graph limits are retained as document authority but every one must
/// be no greater than `admission`. This prevents a document from granting its
/// own decoder a larger allocation or rational-magnitude budget.
pub fn replay_graph_document(
    bytes: &[u8],
    admission: GraphLimits,
) -> Result<GraphReplay, GraphWireError> {
    admission.validate()?;
    if bytes.len() > admission.maximum_document_bytes {
        return Err(GraphWireError::LimitExceeded(
            "admitted document byte length",
        ));
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_DOCUMENT_MAGIC.len())? != GRAPH_DOCUMENT_MAGIC {
        return Err(GraphWireError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_DOCUMENT_VERSION {
        return Err(GraphWireError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_DOCUMENT_FLAGS {
        return Err(GraphWireError::UnsupportedFlags(flags));
    }
    let limits = decode_limits(&mut decoder)?;
    if !limits_within(limits, admission) {
        return Err(GraphWireError::LimitExceeded("embedded admission limit"));
    }
    limits.validate()?;
    if bytes.len() > limits.maximum_document_bytes {
        return Err(GraphWireError::LimitExceeded(
            "embedded document byte length",
        ));
    }
    let revision = decoder.u64()?;
    let schema = decode_schema(&mut decoder, limits)?;
    let clocks = decode_clocks(&mut decoder, limits)?;
    let nodes = decode_nodes(&mut decoder, &schema)?;
    let wires = decode_wires(&mut decoder, limits)?;
    if !decoder.is_empty() {
        return Err(GraphWireError::TrailingBytes);
    }
    let document = GraphDocument::try_new(revision, schema, clocks, nodes, wires)?;
    let encoding = encode_graph_document(&document)?;
    if encoding.bytes() != bytes {
        return Err(GraphWireError::NonCanonical);
    }
    Ok(GraphReplay { document, encoding })
}

pub(super) fn encode_typed_value_bytes(
    schema: &GraphSchema,
    value: &TypedGraphValue,
) -> Result<Vec<u8>, GraphWireError> {
    let mut encoder = Encoder::default();
    encode_typed_value(&mut encoder, schema, value)?;
    Ok(encoder.0)
}

pub(super) fn decode_typed_value_bytes(
    schema: &GraphSchema,
    bytes: &[u8],
) -> Result<TypedGraphValue, GraphWireError> {
    let mut decoder = Decoder::new(bytes);
    let value = decode_typed_value(&mut decoder, schema)?;
    if !decoder.is_empty() {
        return Err(GraphWireError::TrailingBytes);
    }
    let canonical = encode_typed_value_bytes(schema, &value)?;
    if canonical != bytes {
        return Err(GraphWireError::NonCanonical);
    }
    Ok(value)
}

fn encode_limits(encoder: &mut Encoder, limits: GraphLimits) -> Result<(), GraphWireError> {
    for value in limit_values(limits) {
        encoder
            .u64(u64::try_from(value).map_err(|_| GraphWireError::IntegerOverflow("limit value"))?);
    }
    Ok(())
}

fn decode_limits(decoder: &mut Decoder<'_>) -> Result<GraphLimits, GraphWireError> {
    let mut values = [0_usize; LIMIT_FIELD_COUNT];
    for value in &mut values {
        *value = usize::try_from(decoder.u64()?)
            .map_err(|_| GraphWireError::IntegerOverflow("limit value"))?;
    }
    Ok(GraphLimits {
        maximum_document_bytes: values[0],
        maximum_rational_digits: values[1],
        maximum_units: values[2],
        maximum_types: values[3],
        maximum_record_fields: values[4],
        maximum_value_depth: values[5],
        maximum_value_nodes: values[6],
        maximum_array_items: values[7],
        maximum_text_bytes: values[8],
        maximum_blob_bytes: values[9],
        maximum_stream_capacity: values[10],
        maximum_clocks: values[11],
        maximum_nodes: values[12],
        maximum_wires: values[13],
        maximum_ports_per_node: values[14],
        maximum_parameters_per_node: values[15],
        maximum_label_bytes: values[16],
    })
}

const fn limit_values(limits: GraphLimits) -> [usize; LIMIT_FIELD_COUNT] {
    [
        limits.maximum_document_bytes,
        limits.maximum_rational_digits,
        limits.maximum_units,
        limits.maximum_types,
        limits.maximum_record_fields,
        limits.maximum_value_depth,
        limits.maximum_value_nodes,
        limits.maximum_array_items,
        limits.maximum_text_bytes,
        limits.maximum_blob_bytes,
        limits.maximum_stream_capacity,
        limits.maximum_clocks,
        limits.maximum_nodes,
        limits.maximum_wires,
        limits.maximum_ports_per_node,
        limits.maximum_parameters_per_node,
        limits.maximum_label_bytes,
    ]
}

fn limits_within(received: GraphLimits, admission: GraphLimits) -> bool {
    limit_values(received)
        .into_iter()
        .zip(limit_values(admission))
        .all(|(received, admitted)| received <= admitted)
}

fn encode_schema(encoder: &mut Encoder, schema: &GraphSchema) -> Result<(), GraphWireError> {
    encoder.count(schema.units().len(), "unit count")?;
    for unit in schema.units() {
        encoder.u32(unit.id().get());
        encoder.text(unit.symbol())?;
        for exponent in unit.dimensions().exponents() {
            encoder.u8(exponent.cast_unsigned());
        }
        encoder.rational(unit.scale(), schema.limits().maximum_rational_digits)?;
    }
    encoder.count(schema.types().len(), "type count")?;
    for definition in schema.types() {
        encoder.u32(definition.id().get());
        encoder.text(definition.name())?;
        encode_type_kind(encoder, definition.kind(), schema.limits())?;
    }
    Ok(())
}

fn decode_schema(
    decoder: &mut Decoder<'_>,
    limits: GraphLimits,
) -> Result<GraphSchema, GraphWireError> {
    let unit_count = decoder.count(limits.maximum_units, "unit count")?;
    let mut units = Vec::with_capacity(unit_count);
    for _ in 0..unit_count {
        let id = UnitId::new(decoder.u32()?);
        let symbol = decoder.text(UNIT_SYMBOL_BYTES, "unit symbol")?;
        let mut exponents = [0_i8; 7];
        for exponent in &mut exponents {
            *exponent = decoder.u8()?.cast_signed();
        }
        let scale = decoder.rational(limits.maximum_rational_digits)?;
        units.push(UnitDefinition::new(
            id,
            symbol,
            BaseDimensions::new(exponents),
            scale,
        ));
    }
    let type_count = decoder.count(limits.maximum_types, "type count")?;
    let mut types = Vec::with_capacity(type_count);
    for _ in 0..type_count {
        let id = GraphTypeId::new(decoder.u32()?);
        let name = decoder.text(STABLE_NAME_BYTES, "type name")?;
        let kind = decode_type_kind(decoder, limits)?;
        types.push(TypeDefinition::new(id, name, kind));
    }
    GraphSchema::try_new(limits, units, types).map_err(Into::into)
}

fn encode_type_kind(
    encoder: &mut Encoder,
    kind: &TypeKind,
    limits: GraphLimits,
) -> Result<(), GraphWireError> {
    match kind {
        TypeKind::Boolean => encoder.u8(0),
        TypeKind::ExactRational { unit } => {
            encoder.u8(1);
            encoder.u32(unit.get());
        }
        TypeKind::MeasurementInterval { unit } => {
            encoder.u8(2);
            encoder.u32(unit.get());
        }
        TypeKind::CanonicalI64 { unit, quantum } => {
            encoder.u8(3);
            encoder.u32(unit.get());
            encoder.rational(quantum, limits.maximum_rational_digits)?;
        }
        TypeKind::CanonicalU64 { unit, quantum } => {
            encoder.u8(4);
            encoder.u32(unit.get());
            encoder.rational(quantum, limits.maximum_rational_digits)?;
        }
        TypeKind::Text { maximum_bytes } => {
            encoder.u8(5);
            encoder.u32(*maximum_bytes);
        }
        TypeKind::Bytes { maximum_bytes } => {
            encoder.u8(6);
            encoder.u32(*maximum_bytes);
        }
        TypeKind::Array {
            element,
            maximum_items,
        } => {
            encoder.u8(7);
            encoder.u32(element.get());
            encoder.u32(*maximum_items);
        }
        TypeKind::Record { fields } => {
            encoder.u8(8);
            encoder.count(fields.len(), "record field count")?;
            for field in fields {
                encoder.u32(field.id().get());
                encoder.text(field.name())?;
                encoder.u32(field.value_type().get());
            }
        }
        TypeKind::Option { value } => {
            encoder.u8(9);
            encoder.u32(value.get());
        }
        TypeKind::Result { ok, error } => {
            encoder.u8(10);
            encoder.u32(ok.get());
            encoder.u32(error.get());
        }
        TypeKind::Event { payload, clock } => {
            encoder.u8(11);
            encoder.u32(payload.get());
            encoder.u32(clock.get());
        }
        TypeKind::Stream {
            sample,
            clock,
            capacity,
        } => {
            encoder.u8(12);
            encoder.u32(sample.get());
            encoder.u32(clock.get());
            encoder.u32(*capacity);
        }
        TypeKind::ResourceHandle { class } => {
            encoder.u8(13);
            encoder.u32(class.get());
        }
        TypeKind::JobHandle => encoder.u8(14),
    }
    Ok(())
}

fn decode_type_kind(
    decoder: &mut Decoder<'_>,
    limits: GraphLimits,
) -> Result<TypeKind, GraphWireError> {
    Ok(match decoder.u8()? {
        0 => TypeKind::Boolean,
        1 => TypeKind::ExactRational {
            unit: UnitId::new(decoder.u32()?),
        },
        2 => TypeKind::MeasurementInterval {
            unit: UnitId::new(decoder.u32()?),
        },
        3 => TypeKind::CanonicalI64 {
            unit: UnitId::new(decoder.u32()?),
            quantum: decoder.rational(limits.maximum_rational_digits)?,
        },
        4 => TypeKind::CanonicalU64 {
            unit: UnitId::new(decoder.u32()?),
            quantum: decoder.rational(limits.maximum_rational_digits)?,
        },
        5 => TypeKind::Text {
            maximum_bytes: decoder.u32()?,
        },
        6 => TypeKind::Bytes {
            maximum_bytes: decoder.u32()?,
        },
        7 => TypeKind::Array {
            element: GraphTypeId::new(decoder.u32()?),
            maximum_items: decoder.u32()?,
        },
        8 => {
            let count = decoder.count(limits.maximum_record_fields, "record field count")?;
            let mut fields = Vec::with_capacity(count);
            for _ in 0..count {
                fields.push(RecordField::new(
                    RecordFieldId::new(decoder.u32()?),
                    decoder.text(STABLE_NAME_BYTES, "record field name")?,
                    GraphTypeId::new(decoder.u32()?),
                ));
            }
            TypeKind::Record { fields }
        }
        9 => TypeKind::Option {
            value: GraphTypeId::new(decoder.u32()?),
        },
        10 => TypeKind::Result {
            ok: GraphTypeId::new(decoder.u32()?),
            error: GraphTypeId::new(decoder.u32()?),
        },
        11 => TypeKind::Event {
            payload: GraphTypeId::new(decoder.u32()?),
            clock: GraphClockId::new(decoder.u32()?),
        },
        12 => TypeKind::Stream {
            sample: GraphTypeId::new(decoder.u32()?),
            clock: GraphClockId::new(decoder.u32()?),
            capacity: decoder.u32()?,
        },
        13 => TypeKind::ResourceHandle {
            class: ResourceClassId::new(decoder.u32()?),
        },
        14 => TypeKind::JobHandle,
        tag => return Err(GraphWireError::InvalidTag("type", tag)),
    })
}

fn encode_clocks(encoder: &mut Encoder, clocks: &[ClockDefinition]) -> Result<(), GraphWireError> {
    encoder.count(clocks.len(), "clock count")?;
    for clock in clocks {
        encoder.u32(clock.id().get());
        encoder.text(clock.name())?;
        match clock.kind() {
            ClockKind::HostMonotonic { ticks_per_second } => {
                encoder.u8(0);
                encoder.u64(ticks_per_second);
            }
            ClockKind::DeviceCycle {
                device_id,
                ticks_per_second,
            } => {
                encoder.u8(1);
                encoder.device_id(device_id);
                encoder.u64(ticks_per_second);
            }
            ClockKind::Derived {
                source,
                numerator,
                denominator,
            } => {
                encoder.u8(2);
                encoder.u32(source.get());
                encoder.u32(numerator);
                encoder.u32(denominator);
            }
        }
    }
    Ok(())
}

fn decode_clocks(
    decoder: &mut Decoder<'_>,
    limits: GraphLimits,
) -> Result<Vec<ClockDefinition>, GraphWireError> {
    let count = decoder.count(limits.maximum_clocks, "clock count")?;
    let mut clocks = Vec::with_capacity(count);
    for _ in 0..count {
        let id = GraphClockId::new(decoder.u32()?);
        let name = decoder.text(STABLE_NAME_BYTES, "clock name")?;
        let kind = match decoder.u8()? {
            0 => ClockKind::HostMonotonic {
                ticks_per_second: decoder.u64()?,
            },
            1 => ClockKind::DeviceCycle {
                device_id: decoder.device_id()?,
                ticks_per_second: decoder.u64()?,
            },
            2 => ClockKind::Derived {
                source: GraphClockId::new(decoder.u32()?),
                numerator: decoder.u32()?,
                denominator: decoder.u32()?,
            },
            tag => return Err(GraphWireError::InvalidTag("clock", tag)),
        };
        clocks.push(ClockDefinition::new(id, name, kind));
    }
    Ok(clocks)
}

fn encode_nodes(
    encoder: &mut Encoder,
    schema: &GraphSchema,
    nodes: &[NodeDefinition],
) -> Result<(), GraphWireError> {
    encoder.count(nodes.len(), "node count")?;
    for node in nodes {
        encoder.u32(node.id().get());
        encoder.text(node.kind().name())?;
        encoder.u16(node.kind().version());
        encoder.text(node.label())?;
        match node.domain() {
            ExecutionDomain::HostExact => encoder.u8(0),
            ExecutionDomain::Service { device_id } => {
                encoder.u8(1);
                encoder.device_id(device_id);
            }
            ExecutionDomain::Realtime { device_id } => {
                encoder.u8(2);
                encoder.device_id(device_id);
            }
        }
        encode_ports(encoder, node.inputs())?;
        encode_ports(encoder, node.outputs())?;
        encoder.count(node.parameters().len(), "parameter count")?;
        for parameter in node.parameters() {
            encoder.u32(parameter.id());
            encoder.text(parameter.name())?;
            encode_typed_value(encoder, schema, parameter.value())?;
        }
    }
    Ok(())
}

fn decode_nodes(
    decoder: &mut Decoder<'_>,
    schema: &GraphSchema,
) -> Result<Vec<NodeDefinition>, GraphWireError> {
    let limits = schema.limits();
    let count = decoder.count(limits.maximum_nodes, "node count")?;
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        let id = GraphNodeId::new(decoder.u32()?);
        let kind_name = decoder.text(STABLE_NAME_BYTES, "node kind")?;
        let kind_version = decoder.u16()?;
        let label = decoder.text(limits.maximum_label_bytes, "node label")?;
        let domain = match decoder.u8()? {
            0 => ExecutionDomain::HostExact,
            1 => ExecutionDomain::Service {
                device_id: decoder.device_id()?,
            },
            2 => ExecutionDomain::Realtime {
                device_id: decoder.device_id()?,
            },
            tag => return Err(GraphWireError::InvalidTag("execution domain", tag)),
        };
        let inputs = decode_ports(decoder, limits.maximum_ports_per_node)?;
        let outputs = decode_ports(decoder, limits.maximum_ports_per_node)?;
        let parameter_count =
            decoder.count(limits.maximum_parameters_per_node, "node parameter count")?;
        let mut parameters = Vec::with_capacity(parameter_count);
        for _ in 0..parameter_count {
            let parameter_id = decoder.u32()?;
            let name = decoder.text(STABLE_NAME_BYTES, "parameter name")?;
            let value = decode_typed_value(decoder, schema)?;
            parameters.push(NodeParameter::new(parameter_id, name, value));
        }
        nodes.push(NodeDefinition::new(
            id,
            NodeKind::new(kind_name, kind_version),
            label,
            domain,
            inputs,
            outputs,
            parameters,
        ));
    }
    Ok(nodes)
}

fn encode_ports(encoder: &mut Encoder, ports: &[PortDefinition]) -> Result<(), GraphWireError> {
    encoder.count(ports.len(), "port count")?;
    for port in ports {
        encoder.u32(port.id().get());
        encoder.text(port.name())?;
        encoder.u32(port.value_type().get());
    }
    Ok(())
}

fn decode_ports(
    decoder: &mut Decoder<'_>,
    maximum: usize,
) -> Result<Vec<PortDefinition>, GraphWireError> {
    let count = decoder.count(maximum, "port count")?;
    let mut ports = Vec::with_capacity(count);
    for _ in 0..count {
        ports.push(PortDefinition::new(
            GraphPortId::new(decoder.u32()?),
            decoder.text(STABLE_NAME_BYTES, "port name")?,
            GraphTypeId::new(decoder.u32()?),
        ));
    }
    Ok(ports)
}

fn encode_wires(encoder: &mut Encoder, wires: &[WireDefinition]) -> Result<(), GraphWireError> {
    encoder.count(wires.len(), "wire count")?;
    for wire in wires {
        encoder.u32(wire.id().get());
        encode_endpoint(encoder, wire.source());
        encode_endpoint(encoder, wire.target());
    }
    Ok(())
}

fn decode_wires(
    decoder: &mut Decoder<'_>,
    limits: GraphLimits,
) -> Result<Vec<WireDefinition>, GraphWireError> {
    let count = decoder.count(limits.maximum_wires, "wire count")?;
    let mut wires = Vec::with_capacity(count);
    for _ in 0..count {
        wires.push(WireDefinition::new(
            GraphWireId::new(decoder.u32()?),
            decode_endpoint(decoder)?,
            decode_endpoint(decoder)?,
        ));
    }
    Ok(wires)
}

fn encode_endpoint(encoder: &mut Encoder, endpoint: WireEndpoint) {
    encoder.u32(endpoint.node.get());
    encoder.u32(endpoint.port.get());
}

fn decode_endpoint(decoder: &mut Decoder<'_>) -> Result<WireEndpoint, GraphWireError> {
    Ok(WireEndpoint {
        node: GraphNodeId::new(decoder.u32()?),
        port: GraphPortId::new(decoder.u32()?),
    })
}

fn encode_typed_value(
    encoder: &mut Encoder,
    schema: &GraphSchema,
    typed: &TypedGraphValue,
) -> Result<(), GraphWireError> {
    schema.validate_typed_value(typed)?;
    encoder.u32(typed.value_type().get());
    encode_value(encoder, schema, typed.value_type(), typed.value())
}

fn encode_value(
    encoder: &mut Encoder,
    schema: &GraphSchema,
    value_type: GraphTypeId,
    value: &GraphValue,
) -> Result<(), GraphWireError> {
    let kind = schema
        .value_type(value_type)
        .ok_or(GraphSchemaError::UnknownType(value_type))?
        .kind()
        .clone();
    match (kind, value) {
        (TypeKind::Boolean, GraphValue::Boolean(value)) => encoder.u8(u8::from(*value)),
        (TypeKind::ExactRational { .. }, GraphValue::ExactRational(value)) => {
            encoder.rational(value, schema.limits().maximum_rational_digits)?;
        }
        (
            TypeKind::MeasurementInterval { .. },
            GraphValue::MeasurementInterval { lower, upper },
        ) => {
            encoder.rational(lower, schema.limits().maximum_rational_digits)?;
            encoder.rational(upper, schema.limits().maximum_rational_digits)?;
        }
        (TypeKind::CanonicalI64 { .. }, GraphValue::CanonicalI64(value)) => encoder.i64(*value),
        (TypeKind::CanonicalU64 { .. }, GraphValue::CanonicalU64(value)) => encoder.u64(*value),
        (TypeKind::Text { .. }, GraphValue::Text(value)) => encoder.text(value)?,
        (TypeKind::Bytes { .. }, GraphValue::Bytes(value)) => {
            encoder.count(value.len(), "byte literal length")?;
            encoder.bytes(value);
        }
        (TypeKind::Array { element, .. }, GraphValue::Array(values)) => {
            encoder.count(values.len(), "array literal count")?;
            for value in values {
                encode_value(encoder, schema, element, value)?;
            }
        }
        (TypeKind::Record { fields }, GraphValue::Record(values)) => {
            encoder.count(values.len(), "record literal count")?;
            for (field, value) in fields.iter().zip(values) {
                encoder.u32(value.field.get());
                encode_value(encoder, schema, field.value_type(), &value.value)?;
            }
        }
        (TypeKind::Option { .. }, GraphValue::OptionNone) => encoder.u8(0),
        (TypeKind::Option { value: inner }, GraphValue::OptionSome(value)) => {
            encoder.u8(1);
            encode_value(encoder, schema, inner, value)?;
        }
        (TypeKind::Result { ok, .. }, GraphValue::ResultOk(value)) => {
            encoder.u8(0);
            encode_value(encoder, schema, ok, value)?;
        }
        (TypeKind::Result { error, .. }, GraphValue::ResultError(value)) => {
            encoder.u8(1);
            encode_value(encoder, schema, error, value)?;
        }
        (TypeKind::ResourceHandle { .. }, GraphValue::ResourceHandle(handle)) => {
            encoder.device_id(handle.device_id);
            encoder.digest(handle.board_package_digest);
            encoder.u32(handle.class.get());
            encoder.u32(handle.resource_selector);
        }
        (TypeKind::JobHandle, GraphValue::JobHandle(handle)) => {
            encoder.device_id(handle.device_id);
            encoder.digest(handle.global_job_digest);
            encoder.digest(handle.partition_digest);
        }
        _ => return Err(GraphWireError::InvalidValue),
    }
    Ok(())
}

fn decode_typed_value(
    decoder: &mut Decoder<'_>,
    schema: &GraphSchema,
) -> Result<TypedGraphValue, GraphWireError> {
    let value_type = GraphTypeId::new(decoder.u32()?);
    let mut budget = ValueDecodeBudget::default();
    let value = decode_value(decoder, schema, value_type, 1, &mut budget)?;
    TypedGraphValue::try_new(schema, value_type, value).map_err(Into::into)
}

fn decode_value(
    decoder: &mut Decoder<'_>,
    schema: &GraphSchema,
    value_type: GraphTypeId,
    depth: usize,
    budget: &mut ValueDecodeBudget,
) -> Result<GraphValue, GraphWireError> {
    let limits = schema.limits();
    if depth > limits.maximum_value_depth {
        return Err(GraphSchemaError::ValueDepthExceeded.into());
    }
    budget.nodes = budget
        .nodes
        .checked_add(1)
        .ok_or(GraphSchemaError::ValueNodeLimitExceeded)?;
    if budget.nodes > limits.maximum_value_nodes {
        return Err(GraphSchemaError::ValueNodeLimitExceeded.into());
    }
    let kind = schema
        .value_type(value_type)
        .ok_or(GraphSchemaError::UnknownType(value_type))?
        .kind()
        .clone();
    Ok(match kind {
        TypeKind::Boolean => match decoder.u8()? {
            0 => GraphValue::Boolean(false),
            1 => GraphValue::Boolean(true),
            tag => return Err(GraphWireError::InvalidTag("boolean", tag)),
        },
        TypeKind::ExactRational { .. } => {
            GraphValue::ExactRational(decoder.rational(limits.maximum_rational_digits)?)
        }
        TypeKind::MeasurementInterval { .. } => GraphValue::MeasurementInterval {
            lower: decoder.rational(limits.maximum_rational_digits)?,
            upper: decoder.rational(limits.maximum_rational_digits)?,
        },
        TypeKind::CanonicalI64 { .. } => GraphValue::CanonicalI64(decoder.i64()?),
        TypeKind::CanonicalU64 { .. } => GraphValue::CanonicalU64(decoder.u64()?),
        TypeKind::Text { maximum_bytes } => GraphValue::Text(
            decoder.text(
                usize::try_from(maximum_bytes)
                    .map_err(|_| GraphWireError::IntegerOverflow("text literal bound"))?
                    .min(limits.maximum_text_bytes),
                "text literal",
            )?,
        ),
        TypeKind::Bytes { maximum_bytes } => {
            let maximum = usize::try_from(maximum_bytes)
                .map_err(|_| GraphWireError::IntegerOverflow("byte literal bound"))?
                .min(limits.maximum_blob_bytes);
            GraphValue::Bytes(decoder.owned_bytes(maximum, "byte literal length")?)
        }
        TypeKind::Array {
            element,
            maximum_items,
        } => {
            let maximum = usize::try_from(maximum_items)
                .map_err(|_| GraphWireError::IntegerOverflow("array literal bound"))?
                .min(limits.maximum_array_items);
            let count = decoder.count(maximum, "array literal count")?;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(decode_value(decoder, schema, element, depth + 1, budget)?);
            }
            GraphValue::Array(values)
        }
        TypeKind::Record { fields } => {
            let count = decoder.count(limits.maximum_record_fields, "record literal count")?;
            if count != fields.len() {
                return Err(GraphSchemaError::RecordShape.into());
            }
            let mut values = Vec::with_capacity(count);
            for field in fields {
                values.push(RecordValueField {
                    field: RecordFieldId::new(decoder.u32()?),
                    value: decode_value(decoder, schema, field.value_type(), depth + 1, budget)?,
                });
            }
            GraphValue::Record(values)
        }
        TypeKind::Option { value } => match decoder.u8()? {
            0 => GraphValue::OptionNone,
            1 => GraphValue::OptionSome(Box::new(decode_value(
                decoder,
                schema,
                value,
                depth + 1,
                budget,
            )?)),
            tag => return Err(GraphWireError::InvalidTag("option value", tag)),
        },
        TypeKind::Result { ok, error } => match decoder.u8()? {
            0 => GraphValue::ResultOk(Box::new(decode_value(
                decoder,
                schema,
                ok,
                depth + 1,
                budget,
            )?)),
            1 => GraphValue::ResultError(Box::new(decode_value(
                decoder,
                schema,
                error,
                depth + 1,
                budget,
            )?)),
            tag => return Err(GraphWireError::InvalidTag("result value", tag)),
        },
        TypeKind::ResourceHandle { .. } => GraphValue::ResourceHandle(ResourceGraphHandle {
            device_id: decoder.device_id()?,
            board_package_digest: decoder.digest()?,
            class: ResourceClassId::new(decoder.u32()?),
            resource_selector: decoder.u32()?,
        }),
        TypeKind::JobHandle => GraphValue::JobHandle(JobGraphHandle {
            device_id: decoder.device_id()?,
            global_job_digest: decoder.digest()?,
            partition_digest: decoder.digest()?,
        }),
        TypeKind::Event { .. } | TypeKind::Stream { .. } => {
            return Err(GraphSchemaError::RuntimeOnlyType(value_type).into());
        }
    })
}

#[derive(Default)]
struct ValueDecodeBudget {
    nodes: usize,
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

    fn i64(&mut self, value: i64) {
        self.bytes(&value.to_le_bytes());
    }

    fn count(&mut self, value: usize, name: &'static str) -> Result<(), GraphWireError> {
        self.u32(u32::try_from(value).map_err(|_| GraphWireError::IntegerOverflow(name))?);
        Ok(())
    }

    fn text(&mut self, value: &str) -> Result<(), GraphWireError> {
        self.count(value.len(), "string byte length")?;
        self.bytes(value.as_bytes());
        Ok(())
    }

    fn rational(&mut self, value: &Rational, maximum_digits: usize) -> Result<(), GraphWireError> {
        let sign = if value.is_zero() {
            0
        } else if value.is_negative() {
            2
        } else {
            1
        };
        let numerator = value.numerator().to_string();
        let denominator = value.denominator().to_string();
        if numerator.len() > maximum_digits || denominator.len() > maximum_digits {
            return Err(GraphWireError::LimitExceeded("rational magnitude"));
        }
        self.u8(sign);
        self.text(&numerator)?;
        self.text(&denominator)
    }

    fn device_id(&mut self, value: DeviceId) {
        self.bytes(&value.0);
    }

    fn digest(&mut self, value: Digest) {
        self.bytes(&value.0);
    }
}

struct Decoder<'a> {
    remaining: &'a [u8],
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    const fn is_empty(&self) -> bool {
        self.remaining.is_empty()
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphWireError> {
        if length > self.remaining.len() {
            return Err(GraphWireError::Truncated);
        }
        let (value, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], GraphWireError> {
        self.take(N)?
            .try_into()
            .map_err(|_| GraphWireError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, GraphWireError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, GraphWireError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, GraphWireError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, GraphWireError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn i64(&mut self) -> Result<i64, GraphWireError> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    fn count(&mut self, maximum: usize, name: &'static str) -> Result<usize, GraphWireError> {
        let value =
            usize::try_from(self.u32()?).map_err(|_| GraphWireError::IntegerOverflow(name))?;
        if value > maximum {
            Err(GraphWireError::LimitExceeded(name))
        } else {
            Ok(value)
        }
    }

    fn text(&mut self, maximum: usize, name: &'static str) -> Result<String, GraphWireError> {
        let bytes = self.bounded_bytes(maximum, name)?;
        str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| GraphWireError::InvalidUtf8(name))
    }

    fn bounded_bytes(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<&'a [u8], GraphWireError> {
        let length = self.count(maximum, name)?;
        self.take(length)
    }

    fn owned_bytes(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<Vec<u8>, GraphWireError> {
        Ok(self.bounded_bytes(maximum, name)?.to_vec())
    }

    fn rational(&mut self, maximum_digits: usize) -> Result<Rational, GraphWireError> {
        let sign = self.u8()?;
        let numerator = self.text(maximum_digits, "rational numerator")?;
        let denominator = self.text(maximum_digits, "rational denominator")?;
        if !canonical_digits(&numerator) || !canonical_digits(&denominator) || denominator == "0" {
            return Err(GraphWireError::InvalidRational);
        }
        let numerator_is_zero = numerator == "0";
        if !matches!(sign, 0..=2)
            || (sign == 0) != numerator_is_zero
            || (sign == 2 && numerator_is_zero)
        {
            return Err(GraphWireError::InvalidRational);
        }
        let mut source = String::with_capacity(
            numerator
                .len()
                .checked_add(denominator.len())
                .and_then(|length| length.checked_add(2))
                .ok_or(GraphWireError::IntegerOverflow("rational text"))?,
        );
        if sign == 2 {
            source.push('-');
        }
        source.push_str(&numerator);
        source.push('/');
        source.push_str(&denominator);
        let value = source
            .parse::<Rational>()
            .map_err(|_| GraphWireError::InvalidRational)?;
        if value.numerator().to_string() != numerator
            || value.denominator().to_string() != denominator
            || value.is_zero() != (sign == 0)
            || value.is_negative() != (sign == 2)
        {
            return Err(GraphWireError::InvalidRational);
        }
        Ok(value)
    }

    fn device_id(&mut self) -> Result<DeviceId, GraphWireError> {
        Ok(DeviceId(self.array()?))
    }

    fn digest(&mut self) -> Result<Digest, GraphWireError> {
        Ok(Digest(self.array()?))
    }
}

fn canonical_digits(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value.len() == 1 || !value.starts_with('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MM: UnitId = UnitId::new(1);
    const SECOND: UnitId = UnitId::new(2);
    const BOOL: GraphTypeId = GraphTypeId::new(1);
    const EXACT: GraphTypeId = GraphTypeId::new(2);
    const INTERVAL: GraphTypeId = GraphTypeId::new(3);
    const SIGNED: GraphTypeId = GraphTypeId::new(4);
    const UNSIGNED: GraphTypeId = GraphTypeId::new(5);
    const TEXT: GraphTypeId = GraphTypeId::new(6);
    const BYTES: GraphTypeId = GraphTypeId::new(7);
    const ARRAY: GraphTypeId = GraphTypeId::new(8);
    const RECORD: GraphTypeId = GraphTypeId::new(9);
    const OPTION: GraphTypeId = GraphTypeId::new(10);
    const RESULT: GraphTypeId = GraphTypeId::new(11);
    const EVENT: GraphTypeId = GraphTypeId::new(12);
    const STREAM: GraphTypeId = GraphTypeId::new(13);
    const RESOURCE: GraphTypeId = GraphTypeId::new(14);
    const JOB: GraphTypeId = GraphTypeId::new(15);
    const RESOURCE_CLASS: ResourceClassId = ResourceClassId::new(5);

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
                    RESOURCE,
                    "core.resource",
                    TypeKind::ResourceHandle {
                        class: RESOURCE_CLASS,
                    },
                ),
                TypeDefinition::new(
                    STREAM,
                    "stream.interval",
                    TypeKind::Stream {
                        sample: INTERVAL,
                        clock: GraphClockId::new(3),
                        capacity: 64,
                    },
                ),
                TypeDefinition::new(
                    EVENT,
                    "event.bool",
                    TypeKind::Event {
                        payload: BOOL,
                        clock: GraphClockId::new(1),
                    },
                ),
                TypeDefinition::new(
                    RESULT,
                    "core.result",
                    TypeKind::Result {
                        ok: EXACT,
                        error: TEXT,
                    },
                ),
                TypeDefinition::new(OPTION, "core.option", TypeKind::Option { value: TEXT }),
                TypeDefinition::new(
                    RECORD,
                    "core.record",
                    TypeKind::Record {
                        fields: vec![
                            RecordField::new(RecordFieldId::new(2), "position", EXACT),
                            RecordField::new(RecordFieldId::new(1), "enabled", BOOL),
                        ],
                    },
                ),
                TypeDefinition::new(
                    ARRAY,
                    "core.array",
                    TypeKind::Array {
                        element: EXACT,
                        maximum_items: 8,
                    },
                ),
                TypeDefinition::new(BYTES, "core.bytes", TypeKind::Bytes { maximum_bytes: 32 }),
                TypeDefinition::new(TEXT, "core.text", TypeKind::Text { maximum_bytes: 64 }),
                TypeDefinition::new(
                    UNSIGNED,
                    "lattice.unsigned",
                    TypeKind::CanonicalU64 {
                        unit: SECOND,
                        quantum: Rational::fraction(1, 1_000).unwrap(),
                    },
                ),
                TypeDefinition::new(
                    SIGNED,
                    "lattice.signed",
                    TypeKind::CanonicalI64 {
                        unit: MM,
                        quantum: Rational::fraction(1, 16).unwrap(),
                    },
                ),
                TypeDefinition::new(
                    INTERVAL,
                    "measured.mm",
                    TypeKind::MeasurementInterval { unit: MM },
                ),
                TypeDefinition::new(EXACT, "exact.mm", TypeKind::ExactRational { unit: MM }),
                TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
            ],
        )
        .unwrap()
    }

    fn parameter(
        schema: &GraphSchema,
        id: u32,
        name: &str,
        value_type: GraphTypeId,
        value: GraphValue,
    ) -> NodeParameter {
        NodeParameter::new(
            id,
            name,
            TypedGraphValue::try_new(schema, value_type, value).unwrap(),
        )
    }

    fn fixture(revision: u64) -> GraphDocument {
        let schema = schema();
        let parameters = vec![
            parameter(
                &schema,
                15,
                "job",
                JOB,
                GraphValue::JobHandle(JobGraphHandle {
                    device_id: DeviceId([1; 16]),
                    global_job_digest: Digest([2; 32]),
                    partition_digest: Digest([3; 32]),
                }),
            ),
            parameter(
                &schema,
                14,
                "resource",
                RESOURCE,
                GraphValue::ResourceHandle(ResourceGraphHandle {
                    device_id: DeviceId([1; 16]),
                    board_package_digest: Digest([4; 32]),
                    class: RESOURCE_CLASS,
                    resource_selector: 17,
                }),
            ),
            parameter(
                &schema,
                13,
                "result_error",
                RESULT,
                GraphValue::ResultError(Box::new(GraphValue::Text("fault".to_owned()))),
            ),
            parameter(
                &schema,
                12,
                "result_ok",
                RESULT,
                GraphValue::ResultOk(Box::new(GraphValue::ExactRational(
                    Rational::fraction(3, 4).unwrap(),
                ))),
            ),
            parameter(&schema, 11, "option_none", OPTION, GraphValue::OptionNone),
            parameter(
                &schema,
                10,
                "option_some",
                OPTION,
                GraphValue::OptionSome(Box::new(GraphValue::Text("ready".to_owned()))),
            ),
            parameter(
                &schema,
                9,
                "record",
                RECORD,
                GraphValue::Record(vec![
                    RecordValueField {
                        field: RecordFieldId::new(1),
                        value: GraphValue::Boolean(true),
                    },
                    RecordValueField {
                        field: RecordFieldId::new(2),
                        value: GraphValue::ExactRational(Rational::fraction(5, 6).unwrap()),
                    },
                ]),
            ),
            parameter(
                &schema,
                8,
                "array",
                ARRAY,
                GraphValue::Array(vec![
                    GraphValue::ExactRational(Rational::fraction(7, 8).unwrap()),
                    GraphValue::ExactRational(Rational::fraction(-9, 10).unwrap()),
                ]),
            ),
            parameter(
                &schema,
                7,
                "bytes",
                BYTES,
                GraphValue::Bytes(vec![0, 1, 2, 255]),
            ),
            parameter(
                &schema,
                6,
                "text",
                TEXT,
                GraphValue::Text("exact μm".to_owned()),
            ),
            parameter(
                &schema,
                5,
                "unsigned",
                UNSIGNED,
                GraphValue::CanonicalU64(u64::MAX),
            ),
            parameter(
                &schema,
                4,
                "signed",
                SIGNED,
                GraphValue::CanonicalI64(i64::MIN),
            ),
            parameter(
                &schema,
                3,
                "interval",
                INTERVAL,
                GraphValue::MeasurementInterval {
                    lower: Rational::fraction(-1, 3).unwrap(),
                    upper: Rational::fraction(2, 3).unwrap(),
                },
            ),
            parameter(
                &schema,
                2,
                "exact",
                EXACT,
                GraphValue::ExactRational(Rational::fraction(1, 2).unwrap()),
            ),
            parameter(&schema, 1, "bool", BOOL, GraphValue::Boolean(true)),
        ];
        let source = NodeDefinition::new(
            GraphNodeId::new(2),
            NodeKind::new("unknown.vendor.motion-source", 37),
            "motion source",
            ExecutionDomain::Realtime {
                device_id: DeviceId([1; 16]),
            },
            Vec::new(),
            vec![PortDefinition::new(GraphPortId::new(1), "position", EXACT)],
            parameters,
        );
        let sink = NodeDefinition::new(
            GraphNodeId::new(1),
            NodeKind::new("org.alumina.motion-sink", 1),
            "motion sink",
            ExecutionDomain::Service {
                device_id: DeviceId([2; 16]),
            },
            vec![PortDefinition::new(GraphPortId::new(1), "position", EXACT)],
            Vec::new(),
            Vec::new(),
        );
        let host = NodeDefinition::new(
            GraphNodeId::new(3),
            NodeKind::new("org.alumina.host-monitor", 2),
            "host monitor",
            ExecutionDomain::HostExact,
            Vec::new(),
            vec![
                PortDefinition::new(GraphPortId::new(2), "samples", STREAM),
                PortDefinition::new(GraphPortId::new(1), "events", EVENT),
            ],
            Vec::new(),
        );
        GraphDocument::try_new(
            revision,
            schema,
            vec![
                ClockDefinition::new(
                    GraphClockId::new(3),
                    "device.millisecond",
                    ClockKind::Derived {
                        source: GraphClockId::new(2),
                        numerator: 1,
                        denominator: 240_000,
                    },
                ),
                ClockDefinition::new(
                    GraphClockId::new(2),
                    "device.cycle",
                    ClockKind::DeviceCycle {
                        device_id: DeviceId([1; 16]),
                        ticks_per_second: 240_000_000,
                    },
                ),
                ClockDefinition::new(
                    GraphClockId::new(1),
                    "host.monotonic",
                    ClockKind::HostMonotonic {
                        ticks_per_second: 1_000_000_000,
                    },
                ),
            ],
            vec![host, source, sink],
            vec![WireDefinition::new(
                GraphWireId::new(1),
                WireEndpoint {
                    node: GraphNodeId::new(2),
                    port: GraphPortId::new(1),
                },
                WireEndpoint {
                    node: GraphNodeId::new(1),
                    port: GraphPortId::new(1),
                },
            )],
        )
        .unwrap()
    }

    #[test]
    fn every_v1_shape_round_trips_and_digest_covers_exact_bytes() {
        let document = fixture(41);
        let encoded = encode_graph_document(&document).unwrap();
        assert_eq!(encoded.bytes()[..4], GRAPH_DOCUMENT_MAGIC);
        assert_eq!(encoded.digest(), sha256(encoded.bytes()).digest);
        assert_eq!(
            encoded.digest(),
            Digest([
                213, 184, 134, 200, 214, 85, 254, 209, 29, 15, 165, 79, 215, 163, 127, 151, 203,
                22, 162, 188, 151, 158, 225, 38, 170, 9, 251, 249, 133, 152, 206, 185,
            ])
        );

        let replay = replay_graph_document(encoded.bytes(), GraphLimits::interactive()).unwrap();
        assert_eq!(replay.document(), &document);
        assert_eq!(replay.encoding(), &encoded);
        assert_eq!(replay.document().nodes()[1].kind().version(), 37);
        assert_eq!(
            replay.document().nodes()[1].kind().name(),
            "unknown.vendor.motion-source"
        );

        let next = encode_graph_document(&fixture(42)).unwrap();
        assert_ne!(next.digest(), encoded.digest());
        assert_ne!(next.bytes(), encoded.bytes());
    }

    #[test]
    fn all_strict_prefixes_and_trailing_bytes_fail_replay() {
        let encoded = encode_graph_document(&fixture(1)).unwrap();
        for length in 0..encoded.bytes().len() {
            assert!(
                replay_graph_document(&encoded.bytes()[..length], GraphLimits::interactive())
                    .is_err(),
                "strict prefix {length} unexpectedly replayed"
            );
        }
        let mut trailing = encoded.into_bytes();
        trailing.push(0);
        assert_eq!(
            replay_graph_document(&trailing, GraphLimits::interactive()),
            Err(GraphWireError::TrailingBytes)
        );
    }

    #[test]
    fn envelope_policy_and_count_bombs_fail_before_allocation() {
        let encoded = encode_graph_document(&fixture(1)).unwrap();

        let mut bad_magic = encoded.bytes().to_vec();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            replay_graph_document(&bad_magic, GraphLimits::interactive()),
            Err(GraphWireError::InvalidMagic)
        );

        let mut bad_version = encoded.bytes().to_vec();
        bad_version[4..6].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            replay_graph_document(&bad_version, GraphLimits::interactive()),
            Err(GraphWireError::UnsupportedVersion(2))
        );

        let mut bad_flags = encoded.bytes().to_vec();
        bad_flags[6..8].copy_from_slice(&1_u16.to_le_bytes());
        assert_eq!(
            replay_graph_document(&bad_flags, GraphLimits::interactive()),
            Err(GraphWireError::UnsupportedFlags(1))
        );

        let mut self_granted = encoded.bytes().to_vec();
        let too_many_digits = GraphLimits::interactive().maximum_rational_digits as u64 + 1;
        self_granted[16..24].copy_from_slice(&too_many_digits.to_le_bytes());
        assert_eq!(
            replay_graph_document(&self_granted, GraphLimits::interactive()),
            Err(GraphWireError::LimitExceeded("embedded admission limit"))
        );

        let mut count_bomb = encoded.bytes().to_vec();
        let unit_count_offset = 8 + LIMIT_FIELD_COUNT * 8 + 8;
        count_bomb[unit_count_offset..unit_count_offset + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            replay_graph_document(&count_bomb, GraphLimits::interactive()),
            Err(GraphWireError::LimitExceeded("unit count"))
        );

        let mut narrow = GraphLimits::interactive();
        narrow.maximum_document_bytes = encoded.bytes().len() - 1;
        assert_eq!(
            replay_graph_document(encoded.bytes(), narrow),
            Err(GraphWireError::LimitExceeded(
                "admitted document byte length"
            ))
        );
    }

    #[test]
    fn invalid_utf8_unreduced_rationals_and_reordering_are_rejected() {
        let encoded = encode_graph_document(&fixture(1)).unwrap();
        let unit_count_offset = 8 + LIMIT_FIELD_COUNT * 8 + 8;
        let first_unit_offset = unit_count_offset + 4;

        let mut invalid_utf8 = encoded.bytes().to_vec();
        let first_symbol_offset = first_unit_offset + 4 + 4;
        invalid_utf8[first_symbol_offset] = 0xff;
        assert_eq!(
            replay_graph_document(&invalid_utf8, GraphLimits::interactive()),
            Err(GraphWireError::InvalidUtf8("unit symbol"))
        );

        let rational = [1, 1, 0, 0, 0, b'1', 1, 0, 0, 0, b'2'];
        let matches: Vec<_> = encoded
            .bytes()
            .windows(rational.len())
            .enumerate()
            .filter_map(|(index, bytes)| (bytes == rational).then_some(index))
            .collect();
        assert_eq!(matches.len(), 1);
        let mut unreduced = encoded.bytes().to_vec();
        unreduced[matches[0] + 5] = b'2';
        unreduced[matches[0] + 10] = b'4';
        assert_eq!(
            replay_graph_document(&unreduced, GraphLimits::interactive()),
            Err(GraphWireError::InvalidRational)
        );

        let mut decoder = Decoder::new(encoded.bytes());
        decoder.take(8 + LIMIT_FIELD_COUNT * 8 + 8).unwrap();
        assert_eq!(decoder.u32().unwrap(), 2);
        let first_id_offset = encoded.bytes().len() - decoder.remaining.len();
        decoder.u32().unwrap();
        decoder.text(UNIT_SYMBOL_BYTES, "unit").unwrap();
        decoder.take(7).unwrap();
        decoder
            .rational(GraphLimits::interactive().maximum_rational_digits)
            .unwrap();
        let second_id_offset = encoded.bytes().len() - decoder.remaining.len();
        let mut reordered = encoded.into_bytes();
        reordered[first_id_offset..first_id_offset + 4].copy_from_slice(&2_u32.to_le_bytes());
        reordered[second_id_offset..second_id_offset + 4].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            replay_graph_document(&reordered, GraphLimits::interactive()),
            Err(GraphWireError::NonCanonical)
        );
    }

    #[test]
    fn schema_bounds_rational_magnitudes_before_wire_encoding() {
        let mut limits = GraphLimits::interactive();
        limits.maximum_rational_digits = 2;
        assert_eq!(
            GraphSchema::try_new(
                limits,
                vec![UnitDefinition::new(
                    MM,
                    "mm",
                    BaseDimensions::LENGTH,
                    Rational::fraction(1, 1_000).unwrap(),
                )],
                Vec::new(),
            ),
            Err(GraphSchemaError::LimitExceeded("rational magnitude"))
        );
    }
}
