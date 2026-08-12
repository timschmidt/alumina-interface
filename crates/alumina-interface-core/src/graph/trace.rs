//! Canonical deterministic host-simulation traces and independent replay.

use core::fmt;

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::simulation::{
    ExternalStreamSample, GraphSimulation, GraphSimulationError, GraphSimulationLimits,
    GraphSimulationRegistry, GraphTraceEntryKind, simulate_graph,
};
use super::wire::{decode_typed_value_bytes, encode_typed_value_bytes};
use super::{
    GraphClockId, GraphDocument, GraphNodeId, GraphPortId, GraphWireError, WireEndpoint,
    encode_graph_document,
};

/// Magic bytes at the beginning of each canonical graph trace.
pub const GRAPH_TRACE_MAGIC: [u8; 4] = *b"ALGT";

/// Exact canonical graph-trace format implemented by this source tree.
pub const GRAPH_TRACE_VERSION: u16 = 1;

const GRAPH_TRACE_FLAGS: u16 = 0;

/// Canonical trace bytes paired with their SHA-256 content identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphTrace {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphTrace {
    /// Borrow the complete canonical trace bytes.
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

/// Independently regenerated simulation and its byte-identical trace.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphTraceReplay {
    simulation: GraphSimulation,
    encoding: CanonicalGraphTrace,
}

impl GraphTraceReplay {
    /// Borrow the independently regenerated deterministic simulation.
    pub const fn simulation(&self) -> &GraphSimulation {
        &self.simulation
    }

    /// Borrow the byte-identical canonical trace.
    pub const fn encoding(&self) -> &CanonicalGraphTrace {
        &self.encoding
    }
}

/// Rejection at canonical trace encoding, decoding, or independent replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphTraceError {
    /// Input did not begin with [`GRAPH_TRACE_MAGIC`].
    InvalidMagic,
    /// The trace format version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved flags were nonzero.
    UnsupportedFlags(u16),
    /// A fixed-width or length-delimited field ran past the input.
    Truncated,
    /// A count, field, or total byte length exceeded caller policy.
    LimitExceeded(&'static str),
    /// An integer could not be represented by the canonical field width.
    IntegerOverflow(&'static str),
    /// A trace entry tag was unknown.
    InvalidEntryTag(u8),
    /// The trace named a different graph document.
    GraphDigestMismatch,
    /// The trace named a different semantic/implementation registry.
    RegistryDigestMismatch,
    /// Exact typed-value encoding or graph identity failed.
    Graph(GraphWireError),
    /// Deterministic simulation rejected the decoded external inputs.
    Simulation(GraphSimulationError),
    /// Independent simulation did not reproduce every canonical trace byte.
    ReplayDiverged,
}

impl From<GraphWireError> for GraphTraceError {
    fn from(value: GraphWireError) -> Self {
        Self::Graph(value)
    }
}

impl From<GraphSimulationError> for GraphTraceError {
    fn from(value: GraphSimulationError) -> Self {
        Self::Simulation(value)
    }
}

impl fmt::Display for GraphTraceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => formatter.write_str("graph trace magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "graph trace version {version} is unsupported")
            }
            Self::UnsupportedFlags(flags) => {
                write!(formatter, "graph trace flags {flags:#06x} are unsupported")
            }
            Self::Truncated => formatter.write_str("graph trace is truncated"),
            Self::LimitExceeded(name) => write!(formatter, "graph trace {name} exceeds policy"),
            Self::IntegerOverflow(name) => {
                write!(formatter, "graph trace {name} exceeds its integer width")
            }
            Self::InvalidEntryTag(tag) => {
                write!(formatter, "graph trace entry tag {tag} is invalid")
            }
            Self::GraphDigestMismatch => formatter.write_str("graph trace graph digest differs"),
            Self::RegistryDigestMismatch => {
                formatter.write_str("graph trace implementation registry digest differs")
            }
            Self::Graph(error) => write!(formatter, "graph trace value rejected: {error}"),
            Self::Simulation(error) => {
                write!(formatter, "graph trace simulation rejected: {error}")
            }
            Self::ReplayDiverged => {
                formatter.write_str("graph trace did not reproduce byte for byte")
            }
        }
    }
}

impl std::error::Error for GraphTraceError {}

/// Encode one deterministic simulation into canonical bounded trace bytes.
pub fn encode_graph_trace(
    document: &GraphDocument,
    simulation: &GraphSimulation,
    limits: GraphSimulationLimits,
) -> Result<CanonicalGraphTrace, GraphTraceError> {
    limits.validate()?;
    let graph_digest = encode_graph_document(document)?.digest();
    if simulation.graph_digest() != graph_digest {
        return Err(GraphTraceError::GraphDigestMismatch);
    }
    if simulation.entries().len() > limits.maximum_trace_entries {
        return Err(GraphTraceError::LimitExceeded("entry count"));
    }
    let mut encoder = Encoder::default();
    encoder.bytes(&GRAPH_TRACE_MAGIC);
    encoder.u16(GRAPH_TRACE_VERSION);
    encoder.u16(GRAPH_TRACE_FLAGS);
    encoder.digest(simulation.graph_digest());
    encoder.digest(simulation.registry_digest());
    encoder.u32(simulation.horizon().root_clock().get());
    encoder.u64(simulation.horizon().inclusive_root_tick());
    encoder.count(simulation.entries().len(), "entry count")?;
    for entry in simulation.entries() {
        encoder.u8(match entry.kind() {
            GraphTraceEntryKind::ExternalInput => 0,
            GraphTraceEntryKind::NodeOutput => 1,
        });
        encoder.u32(entry.endpoint().node.get());
        encoder.u32(entry.endpoint().port.get());
        encoder.u32(entry.clock().get());
        encoder.u64(entry.clock_tick());
        encoder.u64(entry.sequence());
        let value = encode_typed_value_bytes(document.schema(), entry.value())?;
        encoder.count(value.len(), "typed value length")?;
        encoder.bytes(&value);
        if encoder.0.len() > limits.maximum_trace_bytes {
            return Err(GraphTraceError::LimitExceeded("byte length"));
        }
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphTrace {
        bytes: encoder.0,
        digest,
    })
}

/// Decode external inputs, rerun the fixed simulator, and require exact bytes.
pub fn replay_graph_trace(
    bytes: &[u8],
    document: &GraphDocument,
    registry: &GraphSimulationRegistry,
    limits: GraphSimulationLimits,
) -> Result<GraphTraceReplay, GraphTraceError> {
    limits.validate()?;
    if bytes.len() > limits.maximum_trace_bytes {
        return Err(GraphTraceError::LimitExceeded("byte length"));
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_TRACE_MAGIC.len())? != GRAPH_TRACE_MAGIC {
        return Err(GraphTraceError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_TRACE_VERSION {
        return Err(GraphTraceError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_TRACE_FLAGS {
        return Err(GraphTraceError::UnsupportedFlags(flags));
    }
    let graph_digest = decoder.digest()?;
    if graph_digest != encode_graph_document(document)?.digest() {
        return Err(GraphTraceError::GraphDigestMismatch);
    }
    let registry_digest = decoder.digest()?;
    if registry_digest != registry.digest() {
        return Err(GraphTraceError::RegistryDigestMismatch);
    }
    let horizon =
        super::GraphSimulationHorizon::new(GraphClockId::new(decoder.u32()?), decoder.u64()?);
    let count = decoder.count(limits.maximum_trace_entries, "entry count")?;
    let mut external = Vec::new();
    for _ in 0..count {
        let kind = match decoder.u8()? {
            0 => GraphTraceEntryKind::ExternalInput,
            1 => GraphTraceEntryKind::NodeOutput,
            tag => return Err(GraphTraceError::InvalidEntryTag(tag)),
        };
        let endpoint = WireEndpoint {
            node: GraphNodeId::new(decoder.u32()?),
            port: GraphPortId::new(decoder.u32()?),
        };
        let _clock = GraphClockId::new(decoder.u32()?);
        let clock_tick = decoder.u64()?;
        let sequence = decoder.u64()?;
        let value_bytes =
            decoder.bounded_bytes(limits.maximum_trace_bytes, "typed value length")?;
        let value = decode_typed_value_bytes(document.schema(), value_bytes)?;
        if kind == GraphTraceEntryKind::ExternalInput {
            external.push(ExternalStreamSample::new(
                endpoint, clock_tick, sequence, value,
            ));
        }
    }
    if !decoder.is_empty() {
        return Err(GraphTraceError::ReplayDiverged);
    }
    let simulation = simulate_graph(document, registry, horizon, &external, limits)?;
    let encoding = encode_graph_trace(document, &simulation, limits)?;
    if encoding.bytes() != bytes {
        return Err(GraphTraceError::ReplayDiverged);
    }
    Ok(GraphTraceReplay {
        simulation,
        encoding,
    })
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

    fn digest(&mut self, value: Digest) {
        self.bytes(&value.0);
    }

    fn count(&mut self, value: usize, name: &'static str) -> Result<(), GraphTraceError> {
        self.u32(u32::try_from(value).map_err(|_| GraphTraceError::IntegerOverflow(name))?);
        Ok(())
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

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphTraceError> {
        if length > self.remaining.len() {
            return Err(GraphTraceError::Truncated);
        }
        let (value, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], GraphTraceError> {
        self.take(N)?
            .try_into()
            .map_err(|_| GraphTraceError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, GraphTraceError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, GraphTraceError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, GraphTraceError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, GraphTraceError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn digest(&mut self) -> Result<Digest, GraphTraceError> {
        Ok(Digest(self.array()?))
    }

    fn count(&mut self, maximum: usize, name: &'static str) -> Result<usize, GraphTraceError> {
        let value =
            usize::try_from(self.u32()?).map_err(|_| GraphTraceError::IntegerOverflow(name))?;
        if value > maximum {
            Err(GraphTraceError::LimitExceeded(name))
        } else {
            Ok(value)
        }
    }

    fn bounded_bytes(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<&'a [u8], GraphTraceError> {
        let length = self.count(maximum, name)?;
        self.take(length)
    }
}
