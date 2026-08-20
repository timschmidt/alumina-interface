//! Canonical, bounded diagnostic-probe authoring sidecar.
//!
//! `ALGP` binds named presentation probes to exact output endpoints in one
//! canonical `ALGW`. It also retains one optional Boolean-stream edge trigger
//! and bounded host-side pre/post window. It grants no firmware read access,
//! telemetry bandwidth, device trigger implementation, or deployment authority.
//! Runtime capture remains a separate capability-checked protocol operation;
//! this document says only what an editor intends to observe and how much host
//! memory it may retain.

use core::fmt;
use std::collections::{BTreeSet, VecDeque};
use std::str;

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::{
    GraphClockId, GraphNodeId, GraphPortId, GraphSimulation, GraphTraceEntryKind, GraphTypeId,
    GraphValue, GraphWorkspaceDocument, GraphWorkspaceError, TypeKind, WireEndpoint,
    encode_graph_workspace,
};

/// Magic bytes at the beginning of each canonical graph-probe sidecar.
pub const GRAPH_PROBE_MAGIC: [u8; 4] = *b"ALGP";

/// Exact canonical graph-probe format implemented by this source tree.
pub const GRAPH_PROBE_VERSION: u16 = 2;

const GRAPH_PROBE_FLAGS: u16 = 0;
const PROBE_LIMIT_FIELD_COUNT: usize = 4;
const PROBE_NAME_BYTES: usize = 64;
const EXHAUSTED_U32_CURSOR: u64 = u32::MAX as u64 + 1;

/// Caller-owned and embedded bounds for one probe sidecar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphProbeLimits {
    /// Maximum complete canonical `ALGP` byte length.
    pub maximum_probe_document_bytes: usize,
    /// Maximum retained probe definitions.
    pub maximum_probes: usize,
    /// Maximum host-retained values requested by one probe.
    pub maximum_samples_per_probe: u32,
    /// Maximum event-ordinal decimation stride.
    pub maximum_sample_stride: u32,
}

impl GraphProbeLimits {
    /// Bounded first logic-analyzer/plot authoring policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_probe_document_bytes: 2 * 1024 * 1024,
            maximum_probes: 256,
            maximum_samples_per_probe: 1_000_000,
            maximum_sample_stride: 1_000_000,
        }
    }

    fn validate(self) -> Result<(), GraphProbeError> {
        if self.maximum_probe_document_bytes == 0
            || self.maximum_probes == 0
            || self.maximum_samples_per_probe == 0
            || self.maximum_sample_stride == 0
        {
            Err(GraphProbeError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphProbeLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Stable probe identity local to one sidecar.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphProbeId(u32);

impl GraphProbeId {
    /// Construct an identity. Complete document validation rejects zero.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identity.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Host-retention policy applied to values observed at one output endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphProbeCapture {
    maximum_samples: u32,
    sample_stride: u32,
}

/// Edge transition selected on consecutive retained Boolean-stream samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphProbeEdge {
    /// The previous retained sample was false and the current sample is true.
    Rising,
    /// The previous retained sample was true and the current sample is false.
    Falling,
    /// Either logical transition.
    Either,
}

impl GraphProbeEdge {
    const fn code(self) -> u32 {
        match self {
            Self::Rising => 1,
            Self::Falling => 2,
            Self::Either => 3,
        }
    }

    const fn from_code(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Rising),
            2 => Some(Self::Falling),
            3 => Some(Self::Either),
            _ => None,
        }
    }

    const fn matches(self, previous: bool, current: bool) -> bool {
        match self {
            Self::Rising => !previous && current,
            Self::Falling => previous && !current,
            Self::Either => previous != current,
        }
    }
}

/// One replay-only edge-trigger request bound to a stable probe identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphProbeTrigger {
    probe: GraphProbeId,
    edge: GraphProbeEdge,
    pretrigger_samples: u32,
    posttrigger_samples: u32,
}

impl GraphProbeTrigger {
    /// Select one Boolean probe and the retained sample counts on each side of
    /// its first matching edge. The trigger sample itself is retained once.
    pub const fn new(
        probe: GraphProbeId,
        edge: GraphProbeEdge,
        pretrigger_samples: u32,
        posttrigger_samples: u32,
    ) -> Self {
        Self {
            probe,
            edge,
            pretrigger_samples,
            posttrigger_samples,
        }
    }

    /// Return the stable trigger-source probe.
    pub const fn probe(self) -> GraphProbeId {
        self.probe
    }

    /// Return the exact retained-sample transition condition.
    pub const fn edge(self) -> GraphProbeEdge {
        self.edge
    }

    /// Requested retained samples before the matching sample.
    pub const fn pretrigger_samples(self) -> u32 {
        self.pretrigger_samples
    }

    /// Requested retained samples after the matching sample.
    pub const fn posttrigger_samples(self) -> u32 {
        self.posttrigger_samples
    }
}

/// Exact match and available retained window derived from a canonical replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphProbeTriggerMatch {
    trigger: GraphProbeTrigger,
    clock: GraphClockId,
    trigger_tick: u64,
    trigger_sequence: u64,
    first_tick: u64,
    last_tick: u64,
    retained_pretrigger_samples: u32,
    retained_posttrigger_samples: u32,
}

impl GraphProbeTriggerMatch {
    /// Return the canonical request that produced this match.
    pub const fn trigger(self) -> GraphProbeTrigger {
        self.trigger
    }

    /// Return the graph clock carried by every retained trigger-source sample.
    pub const fn clock(self) -> GraphClockId {
        self.clock
    }

    /// Exact tick of the first matching retained sample.
    pub const fn trigger_tick(self) -> u64 {
        self.trigger_tick
    }

    /// Monotonic source sequence of the matching retained sample.
    pub const fn trigger_sequence(self) -> u64 {
        self.trigger_sequence
    }

    /// Exact first tick available in the bounded replay window.
    pub const fn first_tick(self) -> u64 {
        self.first_tick
    }

    /// Exact last tick available in the bounded replay window.
    pub const fn last_tick(self) -> u64 {
        self.last_tick
    }

    /// Number of pretrigger samples actually available.
    pub const fn retained_pretrigger_samples(self) -> u32 {
        self.retained_pretrigger_samples
    }

    /// Number of posttrigger samples actually available.
    pub const fn retained_posttrigger_samples(self) -> u32 {
        self.retained_posttrigger_samples
    }

    /// Whether the complete requested pretrigger window was present.
    pub const fn pretrigger_complete(self) -> bool {
        self.retained_pretrigger_samples == self.trigger.pretrigger_samples
    }

    /// Whether the complete requested posttrigger window was present.
    pub const fn posttrigger_complete(self) -> bool {
        self.retained_posttrigger_samples == self.trigger.posttrigger_samples
    }
}

/// Replay state for the optional canonical host-side trigger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphProbeTriggerResolution {
    /// The sidecar defines no trigger.
    Disabled,
    /// The trigger is valid but the finite replay contains no matching edge.
    Waiting(GraphProbeTrigger),
    /// The first matching edge and available exact window were resolved.
    Matched(GraphProbeTriggerMatch),
}

impl GraphProbeCapture {
    /// Request a bounded number of retained values, keeping every `stride`th
    /// observed value. A stride of one retains every value.
    pub const fn new(maximum_samples: u32, sample_stride: u32) -> Self {
        Self {
            maximum_samples,
            sample_stride,
        }
    }

    /// Maximum values retained in host memory for this probe.
    pub const fn maximum_samples(self) -> u32 {
        self.maximum_samples
    }

    /// Event-ordinal decimation stride, independent of physical clock units.
    pub const fn sample_stride(self) -> u32 {
        self.sample_stride
    }
}

/// One named output binding and bounded host-retention request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphProbeDefinition {
    id: GraphProbeId,
    name: String,
    source: WireEndpoint,
    value_type: GraphTypeId,
    capture: GraphProbeCapture,
}

impl GraphProbeDefinition {
    /// Construct an unresolved probe. Complete document validation resolves
    /// and stores the exact output value type from the bound workspace.
    pub fn new(
        id: GraphProbeId,
        name: impl Into<String>,
        source: WireEndpoint,
        capture: GraphProbeCapture,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            source,
            value_type: GraphTypeId::new(0),
            capture,
        }
    }

    /// Return the sidecar-local stable identity.
    pub const fn id(&self) -> GraphProbeId {
        self.id
    }

    /// Return the stable probe name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact observed graph output endpoint.
    pub const fn source(&self) -> WireEndpoint {
        self.source
    }

    /// Return the exact value type resolved from the bound workspace.
    pub const fn value_type(&self) -> GraphTypeId {
        self.value_type
    }

    /// Return the bounded host-retention request.
    pub const fn capture(&self) -> GraphProbeCapture {
        self.capture
    }
}

/// Canonical probe sidecar bound to one exact workspace identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphProbeDocument {
    limits: GraphProbeLimits,
    revision: u64,
    next_probe_id: u64,
    workspace_digest: Digest,
    trigger: Option<GraphProbeTrigger>,
    probes: Vec<GraphProbeDefinition>,
}

impl GraphProbeDocument {
    /// Validate and canonicalize one complete probe sidecar.
    pub fn try_new(
        limits: GraphProbeLimits,
        revision: u64,
        next_probe_id: u64,
        trigger: Option<GraphProbeTrigger>,
        workspace: &GraphWorkspaceDocument,
        mut probes: Vec<GraphProbeDefinition>,
    ) -> Result<Self, GraphProbeError> {
        limits.validate()?;
        if probes.len() > limits.maximum_probes {
            return Err(GraphProbeError::LimitExceeded("probe count"));
        }
        probes.sort_unstable_by_key(GraphProbeDefinition::id);
        let workspace_digest = workspace_identity(workspace)?;
        validate_probes(workspace, &mut probes, limits)?;
        validate_trigger(workspace, &probes, trigger, limits)?;
        validate_identity_cursor(
            next_probe_id,
            probes.iter().map(|probe| u64::from(probe.id.get())),
        )?;
        Ok(Self {
            limits,
            revision,
            next_probe_id,
            workspace_digest,
            trigger,
            probes,
        })
    }

    /// Return embedded probe bounds.
    pub const fn limits(&self) -> GraphProbeLimits {
        self.limits
    }

    /// Return monotonic sidecar revision metadata.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Return the next probe identity or exhausted sentinel.
    pub const fn next_probe_id(&self) -> u64 {
        self.next_probe_id
    }

    /// Return the exact canonical `ALGW` identity to which probes bind.
    pub const fn workspace_digest(&self) -> Digest {
        self.workspace_digest
    }

    /// Return the optional replay-only trigger request.
    pub const fn trigger(&self) -> Option<GraphProbeTrigger> {
        self.trigger
    }

    /// Borrow probes in canonical identity order.
    pub fn probes(&self) -> &[GraphProbeDefinition] {
        &self.probes
    }

    /// Resolve one probe by stable identity.
    pub fn probe(&self, id: GraphProbeId) -> Option<&GraphProbeDefinition> {
        self.probes
            .binary_search_by_key(&id, GraphProbeDefinition::id)
            .ok()
            .map(|index| &self.probes[index])
    }

    /// Whether one exact output endpoint already has a probe binding.
    pub fn observes(&self, source: WireEndpoint) -> bool {
        self.probes.iter().any(|probe| probe.source == source)
    }

    /// Transactionally rebind unchanged probe endpoints to a revised
    /// workspace. Every endpoint and exact value type must remain valid. An
    /// identical workspace identity is an exact no-op and does not consume a
    /// document revision.
    pub fn replace_workspace(
        &mut self,
        workspace: &GraphWorkspaceDocument,
    ) -> Result<(), GraphProbeError> {
        if workspace_identity(workspace)? == self.workspace_digest {
            return Ok(());
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphProbeError::RevisionOverflow)?;
        let candidate = Self::try_new(
            self.limits,
            revision,
            self.next_probe_id,
            self.trigger,
            workspace,
            self.probes.clone(),
        )?;
        *self = candidate;
        Ok(())
    }

    /// Transactionally add one probe without reusing identities.
    pub fn add_probe(
        &mut self,
        workspace: &GraphWorkspaceDocument,
        name: impl Into<String>,
        source: WireEndpoint,
        capture: GraphProbeCapture,
    ) -> Result<GraphProbeId, GraphProbeError> {
        self.require_workspace(workspace)?;
        let value =
            u32::try_from(self.next_probe_id).map_err(|_| GraphProbeError::IdentifierExhausted)?;
        let following = self
            .next_probe_id
            .checked_add(1)
            .filter(|next| *next <= EXHAUSTED_U32_CURSOR)
            .ok_or(GraphProbeError::IdentifierExhausted)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphProbeError::RevisionOverflow)?;
        let id = GraphProbeId::new(value);
        let mut probes = self.probes.clone();
        probes.push(GraphProbeDefinition::new(id, name, source, capture));
        let candidate = Self::try_new(
            self.limits,
            revision,
            following,
            self.trigger,
            workspace,
            probes,
        )?;
        *self = candidate;
        Ok(id)
    }

    /// Transactionally remove one probe without rewinding its identity cursor.
    pub fn remove_probe(
        &mut self,
        workspace: &GraphWorkspaceDocument,
        id: GraphProbeId,
    ) -> Result<(), GraphProbeError> {
        self.require_workspace(workspace)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphProbeError::RevisionOverflow)?;
        let mut probes = self.probes.clone();
        let index = probes
            .binary_search_by_key(&id, GraphProbeDefinition::id)
            .map_err(|_| GraphProbeError::UnknownProbe(id))?;
        probes.remove(index);
        let trigger = self.trigger.filter(|trigger| trigger.probe != id);
        let candidate = Self::try_new(
            self.limits,
            revision,
            self.next_probe_id,
            trigger,
            workspace,
            probes,
        )?;
        *self = candidate;
        Ok(())
    }

    /// Whether one probe is a Boolean Stream and can source an edge trigger.
    pub fn supports_edge_trigger(
        &self,
        workspace: &GraphWorkspaceDocument,
        id: GraphProbeId,
    ) -> Result<bool, GraphProbeError> {
        self.require_workspace(workspace)?;
        let probe = self.probe(id).ok_or(GraphProbeError::UnknownProbe(id))?;
        Ok(is_boolean_stream(workspace, probe.value_type))
    }

    /// Transactionally install or replace the one replay-only trigger. Setting
    /// an identical trigger is an exact no-op.
    pub fn set_trigger(
        &mut self,
        workspace: &GraphWorkspaceDocument,
        trigger: GraphProbeTrigger,
    ) -> Result<(), GraphProbeError> {
        self.require_workspace(workspace)?;
        if self.trigger == Some(trigger) {
            return Ok(());
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphProbeError::RevisionOverflow)?;
        let candidate = Self::try_new(
            self.limits,
            revision,
            self.next_probe_id,
            Some(trigger),
            workspace,
            self.probes.clone(),
        )?;
        *self = candidate;
        Ok(())
    }

    /// Transactionally remove the replay-only trigger. Clearing an already
    /// disabled trigger is an exact no-op.
    pub fn clear_trigger(
        &mut self,
        workspace: &GraphWorkspaceDocument,
    ) -> Result<(), GraphProbeError> {
        self.require_workspace(workspace)?;
        if self.trigger.is_none() {
            return Ok(());
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphProbeError::RevisionOverflow)?;
        let candidate = Self::try_new(
            self.limits,
            revision,
            self.next_probe_id,
            None,
            workspace,
            self.probes.clone(),
        )?;
        *self = candidate;
        Ok(())
    }

    fn require_workspace(&self, workspace: &GraphWorkspaceDocument) -> Result<(), GraphProbeError> {
        let received = workspace_identity(workspace)?;
        if received == self.workspace_digest {
            Ok(())
        } else {
            Err(GraphProbeError::WorkspaceIdentityMismatch {
                expected: self.workspace_digest,
                received,
            })
        }
    }
}

/// Canonical probe bytes paired with their SHA-256 content identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphProbeEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphProbeEncoding {
    /// Borrow complete canonical bytes.
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

/// Successfully replayed sidecar and its byte-for-byte verified identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphProbeReplay {
    document: GraphProbeDocument,
    encoding: CanonicalGraphProbeEncoding,
}

impl GraphProbeReplay {
    /// Borrow the reconstructed probe document.
    pub const fn document(&self) -> &GraphProbeDocument {
        &self.document
    }

    /// Borrow canonical bytes and identity.
    pub const fn encoding(&self) -> &CanonicalGraphProbeEncoding {
        &self.encoding
    }

    /// Consume the replay and return the probe document.
    pub fn into_document(self) -> GraphProbeDocument {
        self.document
    }
}

/// Rejection at probe authoring or canonical replay boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphProbeError {
    /// A caller or embedded policy contained zero.
    ZeroLimit,
    /// A count, byte length, capture value, or embedded policy exceeded bounds.
    LimitExceeded(&'static str),
    /// Input did not begin with [`GRAPH_PROBE_MAGIC`].
    InvalidMagic,
    /// Probe format version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved probe flags were nonzero.
    UnsupportedFlags(u16),
    /// The encoded edge-trigger condition was not assigned by this version.
    InvalidTriggerKind(u32),
    /// A fixed-width or length-delimited field ran past input.
    Truncated,
    /// A declared probe name was not UTF-8.
    InvalidUtf8,
    /// A canonical field could not fit its required integer width.
    IntegerOverflow(&'static str),
    /// Valid fields remained after the probe document.
    TrailingBytes,
    /// Decoding and reconstruction changed at least one byte.
    NonCanonical,
    /// A probe name was malformed.
    InvalidName,
    /// A probe identity was zero.
    ZeroIdentifier,
    /// A probe identity was duplicated.
    DuplicateIdentifier,
    /// A probe name was duplicated.
    DuplicateName,
    /// More than one probe bound the same output endpoint.
    DuplicateSource(WireEndpoint),
    /// The next monotonic probe identity was invalid.
    InvalidIdentifierCursor,
    /// The probe identity namespace is exhausted.
    IdentifierExhausted,
    /// Probe-document revision could not advance.
    RevisionOverflow,
    /// A requested probe did not exist.
    UnknownProbe(GraphProbeId),
    /// The configured trigger did not name a retained probe.
    UnknownTriggerProbe(GraphProbeId),
    /// The configured trigger source was not a Boolean Stream.
    NonBooleanTriggerProbe {
        /// Rejected trigger-source probe.
        probe: GraphProbeId,
        /// Exact output type retained by that probe.
        value_type: GraphTypeId,
    },
    /// A probe did not resolve to a graph output endpoint.
    UnknownOutput(WireEndpoint),
    /// A replayed or rebound endpoint changed exact value type.
    ValueTypeMismatch {
        /// Bound output endpoint.
        source: WireEndpoint,
        /// Type retained by the sidecar.
        expected: GraphTypeId,
        /// Type resolved from the workspace.
        received: GraphTypeId,
    },
    /// Supplied workspace bytes did not match the sidecar binding.
    WorkspaceIdentityMismatch {
        /// Identity retained by the sidecar.
        expected: Digest,
        /// Identity calculated over the supplied workspace.
        received: Digest,
    },
    /// A replay came from another canonical graph than the bound workspace.
    SimulationGraphIdentityMismatch {
        /// Graph identity retained by the supplied workspace.
        expected: Digest,
        /// Graph identity retained by the simulation.
        received: Digest,
    },
    /// One trigger-source trace sample was not Boolean.
    TriggerSampleTypeMismatch(GraphProbeId),
    /// One trigger-source output changed clocks within a canonical replay.
    TriggerSampleClockMismatch {
        /// Clock carried by the first retained source sample.
        expected: GraphClockId,
        /// Clock carried by a later retained source sample.
        received: GraphClockId,
    },
    /// Canonical workspace identity calculation failed.
    Workspace(GraphWorkspaceError),
}

impl fmt::Display for GraphProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph probe limit is zero"),
            Self::LimitExceeded(name) => write!(formatter, "graph probe {name} exceeds policy"),
            Self::InvalidMagic => formatter.write_str("graph probe magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "graph probe version {version} is unsupported")
            }
            Self::UnsupportedFlags(flags) => {
                write!(formatter, "graph probe flags {flags:#06x} are unsupported")
            }
            Self::InvalidTriggerKind(kind) => {
                write!(formatter, "graph probe trigger kind {kind} is unsupported")
            }
            Self::Truncated => formatter.write_str("graph probe document is truncated"),
            Self::InvalidUtf8 => formatter.write_str("graph probe name is not UTF-8"),
            Self::IntegerOverflow(name) => write!(formatter, "graph probe {name} overflowed"),
            Self::TrailingBytes => formatter.write_str("graph probe document has trailing bytes"),
            Self::NonCanonical => formatter.write_str("graph probe bytes are noncanonical"),
            Self::InvalidName => formatter.write_str("graph probe name is invalid"),
            Self::ZeroIdentifier => formatter.write_str("graph probe identity is zero"),
            Self::DuplicateIdentifier => formatter.write_str("graph probe identity is duplicated"),
            Self::DuplicateName => formatter.write_str("graph probe name is duplicated"),
            Self::DuplicateSource(source) => {
                write!(formatter, "graph output {source:?} has duplicate probes")
            }
            Self::InvalidIdentifierCursor => {
                formatter.write_str("graph probe identity cursor is invalid")
            }
            Self::IdentifierExhausted => {
                formatter.write_str("graph probe identities are exhausted")
            }
            Self::RevisionOverflow => formatter.write_str("graph probe revision is exhausted"),
            Self::UnknownProbe(id) => write!(formatter, "graph probe {id:?} is unknown"),
            Self::UnknownTriggerProbe(id) => {
                write!(formatter, "graph trigger probe {id:?} is unknown")
            }
            Self::NonBooleanTriggerProbe { probe, .. } => {
                write!(
                    formatter,
                    "graph trigger probe {probe:?} is not a Boolean Stream"
                )
            }
            Self::UnknownOutput(source) => write!(formatter, "graph output {source:?} is unknown"),
            Self::ValueTypeMismatch { source, .. } => {
                write!(formatter, "graph probe output {source:?} changed type")
            }
            Self::WorkspaceIdentityMismatch { .. } => {
                formatter.write_str("graph probe workspace identity does not match")
            }
            Self::SimulationGraphIdentityMismatch { .. } => {
                formatter.write_str("graph probe replay identity does not match the workspace")
            }
            Self::TriggerSampleTypeMismatch(probe) => {
                write!(
                    formatter,
                    "graph trigger probe {probe:?} emitted a non-Boolean sample"
                )
            }
            Self::TriggerSampleClockMismatch { .. } => {
                formatter.write_str("graph trigger source changed clocks within one replay")
            }
            Self::Workspace(error) => write!(formatter, "graph probe workspace failed: {error}"),
        }
    }
}

impl std::error::Error for GraphProbeError {}

impl From<GraphWorkspaceError> for GraphProbeError {
    fn from(value: GraphWorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

/// Encode one validated probe sidecar and compute its exact identity.
pub fn encode_graph_probes(
    document: &GraphProbeDocument,
) -> Result<CanonicalGraphProbeEncoding, GraphProbeError> {
    let mut encoder = Encoder::default();
    encoder.bytes(&GRAPH_PROBE_MAGIC);
    encoder.u16(GRAPH_PROBE_VERSION);
    encoder.u16(GRAPH_PROBE_FLAGS);
    encode_limits(&mut encoder, document.limits)?;
    encoder.u64(document.revision);
    encoder.u64(document.next_probe_id);
    encoder.bytes(&document.workspace_digest.0);
    encode_trigger(&mut encoder, document.trigger);
    encoder.u32(
        u32::try_from(document.probes.len())
            .map_err(|_| GraphProbeError::IntegerOverflow("probe count"))?,
    );
    for probe in &document.probes {
        encoder.u32(probe.id.get());
        encoder.u32(probe.source.node.get());
        encoder.u32(probe.source.port.get());
        encoder.u32(probe.value_type.get());
        encoder.u32(probe.capture.maximum_samples);
        encoder.u32(probe.capture.sample_stride);
        encoder.string(&probe.name)?;
    }
    if encoder.0.len() > document.limits.maximum_probe_document_bytes {
        return Err(GraphProbeError::LimitExceeded("document byte length"));
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphProbeEncoding {
        bytes: encoder.0,
        digest,
    })
}

/// Replay canonical probe bytes against the exact external workspace.
pub fn replay_graph_probes(
    encoded: &[u8],
    workspace: &GraphWorkspaceDocument,
    caller_limits: GraphProbeLimits,
) -> Result<GraphProbeReplay, GraphProbeError> {
    caller_limits.validate()?;
    if encoded.len() > caller_limits.maximum_probe_document_bytes {
        return Err(GraphProbeError::LimitExceeded("document byte length"));
    }
    let mut decoder = Decoder::new(encoded);
    if decoder.array::<4>()? != GRAPH_PROBE_MAGIC {
        return Err(GraphProbeError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_PROBE_VERSION {
        return Err(GraphProbeError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_PROBE_FLAGS {
        return Err(GraphProbeError::UnsupportedFlags(flags));
    }
    let embedded_limits = decode_limits(&mut decoder)?;
    embedded_limits.validate()?;
    if !limits_within(embedded_limits, caller_limits) {
        return Err(GraphProbeError::LimitExceeded("embedded limits"));
    }
    let revision = decoder.u64()?;
    let next_probe_id = decoder.u64()?;
    let encoded_workspace_digest = Digest(decoder.array()?);
    let trigger = decode_trigger(&mut decoder)?;
    let count = decoder.count(embedded_limits.maximum_probes, "probe count")?;
    let mut probes = Vec::with_capacity(count);
    for _ in 0..count {
        let id = GraphProbeId::new(decoder.u32()?);
        let source = WireEndpoint {
            node: GraphNodeId::new(decoder.u32()?),
            port: GraphPortId::new(decoder.u32()?),
        };
        let value_type = GraphTypeId::new(decoder.u32()?);
        let capture = GraphProbeCapture::new(decoder.u32()?, decoder.u32()?);
        let name = decoder.string()?;
        probes.push(GraphProbeDefinition {
            id,
            name,
            source,
            value_type,
            capture,
        });
    }
    if !decoder.is_empty() {
        return Err(GraphProbeError::TrailingBytes);
    }
    let received_workspace_digest = workspace_identity(workspace)?;
    if encoded_workspace_digest != received_workspace_digest {
        return Err(GraphProbeError::WorkspaceIdentityMismatch {
            expected: encoded_workspace_digest,
            received: received_workspace_digest,
        });
    }
    let document = GraphProbeDocument::try_new(
        embedded_limits,
        revision,
        next_probe_id,
        trigger,
        workspace,
        probes,
    )?;
    let canonical = encode_graph_probes(&document)?;
    if canonical.bytes() != encoded {
        return Err(GraphProbeError::NonCanonical);
    }
    Ok(GraphProbeReplay {
        document,
        encoding: canonical,
    })
}

/// Resolve the first matching retained edge and its exact bounded replay
/// window. This scans canonical host simulation output only; it does not arm or
/// configure a device capture engine.
pub fn resolve_graph_probe_trigger(
    document: &GraphProbeDocument,
    workspace: &GraphWorkspaceDocument,
    simulation: &GraphSimulation,
) -> Result<GraphProbeTriggerResolution, GraphProbeError> {
    document.require_workspace(workspace)?;
    if simulation.graph_digest() != workspace.graph_digest() {
        return Err(GraphProbeError::SimulationGraphIdentityMismatch {
            expected: workspace.graph_digest(),
            received: simulation.graph_digest(),
        });
    }
    let Some(trigger) = document.trigger else {
        return Ok(GraphProbeTriggerResolution::Disabled);
    };
    let probe = document
        .probe(trigger.probe)
        .ok_or(GraphProbeError::UnknownTriggerProbe(trigger.probe))?;
    let requested_pre = usize::try_from(trigger.pretrigger_samples)
        .map_err(|_| GraphProbeError::IntegerOverflow("trigger pretrigger samples"))?;
    let mut pretrigger = VecDeque::with_capacity(requested_pre.saturating_add(1));
    let mut previous = None;
    let mut source_ordinal = 0_u64;
    let mut source_clock = None;
    let mut matched = None;
    let mut retained_posttrigger_samples = 0_u32;
    let mut last_tick = 0_u64;
    for entry in simulation.entries().iter().filter(|entry| {
        entry.kind() == GraphTraceEntryKind::NodeOutput && entry.endpoint() == probe.source
    }) {
        let retained = source_ordinal.is_multiple_of(u64::from(probe.capture.sample_stride));
        source_ordinal = source_ordinal
            .checked_add(1)
            .ok_or(GraphProbeError::IntegerOverflow("trigger sample ordinal"))?;
        if !retained {
            continue;
        }
        if let Some(expected) = source_clock {
            if entry.clock() != expected {
                return Err(GraphProbeError::TriggerSampleClockMismatch {
                    expected,
                    received: entry.clock(),
                });
            }
        } else {
            source_clock = Some(entry.clock());
        }
        let GraphValue::Boolean(value) = entry.value().value() else {
            return Err(GraphProbeError::TriggerSampleTypeMismatch(trigger.probe));
        };
        if matched.is_some() {
            retained_posttrigger_samples = retained_posttrigger_samples.checked_add(1).ok_or(
                GraphProbeError::IntegerOverflow("retained posttrigger samples"),
            )?;
            last_tick = entry.clock_tick();
            if retained_posttrigger_samples == trigger.posttrigger_samples {
                break;
            }
            continue;
        }
        pretrigger.push_back((entry.clock_tick(), entry.sequence()));
        if pretrigger.len() > requested_pre.saturating_add(1) {
            pretrigger.pop_front();
        }
        if previous.is_some_and(|previous| trigger.edge.matches(previous, *value)) {
            let retained_pretrigger_samples = u32::try_from(pretrigger.len().saturating_sub(1))
                .map_err(|_| GraphProbeError::IntegerOverflow("retained pretrigger samples"))?;
            matched = Some((
                entry.clock(),
                entry.clock_tick(),
                entry.sequence(),
                pretrigger
                    .front()
                    .map_or(entry.clock_tick(), |sample| sample.0),
                retained_pretrigger_samples,
            ));
            last_tick = entry.clock_tick();
            if trigger.posttrigger_samples == 0 {
                break;
            }
        }
        previous = Some(*value);
    }
    let Some((clock, trigger_tick, trigger_sequence, first_tick, retained_pretrigger_samples)) =
        matched
    else {
        return Ok(GraphProbeTriggerResolution::Waiting(trigger));
    };
    Ok(GraphProbeTriggerResolution::Matched(
        GraphProbeTriggerMatch {
            trigger,
            clock,
            trigger_tick,
            trigger_sequence,
            first_tick,
            last_tick,
            retained_pretrigger_samples,
            retained_posttrigger_samples,
        },
    ))
}

fn validate_probes(
    workspace: &GraphWorkspaceDocument,
    probes: &mut [GraphProbeDefinition],
    limits: GraphProbeLimits,
) -> Result<(), GraphProbeError> {
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for probe in probes {
        if probe.id.get() == 0 {
            return Err(GraphProbeError::ZeroIdentifier);
        }
        if !ids.insert(probe.id) {
            return Err(GraphProbeError::DuplicateIdentifier);
        }
        if !valid_probe_name(&probe.name) {
            return Err(GraphProbeError::InvalidName);
        }
        if !names.insert(probe.name.as_str()) {
            return Err(GraphProbeError::DuplicateName);
        }
        if !sources.insert(probe.source) {
            return Err(GraphProbeError::DuplicateSource(probe.source));
        }
        if probe.capture.maximum_samples == 0
            || probe.capture.maximum_samples > limits.maximum_samples_per_probe
        {
            return Err(GraphProbeError::LimitExceeded("sample count"));
        }
        if probe.capture.sample_stride == 0
            || probe.capture.sample_stride > limits.maximum_sample_stride
        {
            return Err(GraphProbeError::LimitExceeded("sample stride"));
        }
        let value_type = output_value_type(workspace, probe.source)
            .ok_or(GraphProbeError::UnknownOutput(probe.source))?;
        if probe.value_type.get() != 0 && probe.value_type != value_type {
            return Err(GraphProbeError::ValueTypeMismatch {
                source: probe.source,
                expected: probe.value_type,
                received: value_type,
            });
        }
        probe.value_type = value_type;
    }
    Ok(())
}

fn validate_trigger(
    workspace: &GraphWorkspaceDocument,
    probes: &[GraphProbeDefinition],
    trigger: Option<GraphProbeTrigger>,
    limits: GraphProbeLimits,
) -> Result<(), GraphProbeError> {
    let Some(trigger) = trigger else {
        return Ok(());
    };
    let probe = probes
        .binary_search_by_key(&trigger.probe, GraphProbeDefinition::id)
        .ok()
        .map(|index| &probes[index])
        .ok_or(GraphProbeError::UnknownTriggerProbe(trigger.probe))?;
    if !is_boolean_stream(workspace, probe.value_type) {
        return Err(GraphProbeError::NonBooleanTriggerProbe {
            probe: trigger.probe,
            value_type: probe.value_type,
        });
    }
    let window_samples = u64::from(trigger.pretrigger_samples)
        .checked_add(1)
        .and_then(|count| count.checked_add(u64::from(trigger.posttrigger_samples)))
        .ok_or(GraphProbeError::IntegerOverflow("trigger window"))?;
    if window_samples > u64::from(limits.maximum_samples_per_probe)
        || window_samples > u64::from(probe.capture.maximum_samples)
    {
        return Err(GraphProbeError::LimitExceeded("trigger window"));
    }
    Ok(())
}

fn is_boolean_stream(workspace: &GraphWorkspaceDocument, value_type: GraphTypeId) -> bool {
    let schema = workspace.graph().schema();
    let Some(definition) = schema.value_type(value_type) else {
        return false;
    };
    let TypeKind::Stream { sample, .. } = definition.kind() else {
        return false;
    };
    schema
        .value_type(*sample)
        .is_some_and(|sample| matches!(sample.kind(), TypeKind::Boolean))
}

fn encode_trigger(encoder: &mut Encoder, trigger: Option<GraphProbeTrigger>) {
    if let Some(trigger) = trigger {
        encoder.u32(trigger.edge.code());
        encoder.u32(trigger.probe.get());
        encoder.u32(trigger.pretrigger_samples);
        encoder.u32(trigger.posttrigger_samples);
    } else {
        for _ in 0..4 {
            encoder.u32(0);
        }
    }
}

fn decode_trigger(decoder: &mut Decoder<'_>) -> Result<Option<GraphProbeTrigger>, GraphProbeError> {
    let kind = decoder.u32()?;
    let probe = GraphProbeId::new(decoder.u32()?);
    let pretrigger_samples = decoder.u32()?;
    let posttrigger_samples = decoder.u32()?;
    if kind == 0 {
        return Ok(None);
    }
    let edge = GraphProbeEdge::from_code(kind).ok_or(GraphProbeError::InvalidTriggerKind(kind))?;
    Ok(Some(GraphProbeTrigger::new(
        probe,
        edge,
        pretrigger_samples,
        posttrigger_samples,
    )))
}

fn output_value_type(
    workspace: &GraphWorkspaceDocument,
    source: WireEndpoint,
) -> Option<GraphTypeId> {
    workspace
        .graph()
        .node(source.node)?
        .outputs()
        .iter()
        .find(|port| port.id() == source.port)
        .map(super::PortDefinition::value_type)
}

fn workspace_identity(workspace: &GraphWorkspaceDocument) -> Result<Digest, GraphProbeError> {
    Ok(encode_graph_workspace(workspace)?.digest())
}

fn validate_identity_cursor(
    cursor: u64,
    identifiers: impl Iterator<Item = u64>,
) -> Result<(), GraphProbeError> {
    let maximum = identifiers.max().unwrap_or(0);
    let minimum = maximum
        .checked_add(1)
        .ok_or(GraphProbeError::InvalidIdentifierCursor)?;
    if cursor == 0 || cursor < minimum || cursor > EXHAUSTED_U32_CURSOR {
        Err(GraphProbeError::InvalidIdentifierCursor)
    } else {
        Ok(())
    }
}

fn valid_probe_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= PROBE_NAME_BYTES
        && bytes[0].is_ascii_alphabetic()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn encode_limits(encoder: &mut Encoder, limits: GraphProbeLimits) -> Result<(), GraphProbeError> {
    let values = [
        u64::try_from(limits.maximum_probe_document_bytes)
            .map_err(|_| GraphProbeError::IntegerOverflow("document byte limit"))?,
        u64::try_from(limits.maximum_probes)
            .map_err(|_| GraphProbeError::IntegerOverflow("probe count limit"))?,
        u64::from(limits.maximum_samples_per_probe),
        u64::from(limits.maximum_sample_stride),
    ];
    for value in values {
        encoder.u64(value);
    }
    Ok(())
}

fn decode_limits(decoder: &mut Decoder<'_>) -> Result<GraphProbeLimits, GraphProbeError> {
    let mut values = [0_u64; PROBE_LIMIT_FIELD_COUNT];
    for value in &mut values {
        *value = decoder.u64()?;
    }
    Ok(GraphProbeLimits {
        maximum_probe_document_bytes: usize::try_from(values[0])
            .map_err(|_| GraphProbeError::IntegerOverflow("document byte limit"))?,
        maximum_probes: usize::try_from(values[1])
            .map_err(|_| GraphProbeError::IntegerOverflow("probe count limit"))?,
        maximum_samples_per_probe: u32::try_from(values[2])
            .map_err(|_| GraphProbeError::IntegerOverflow("sample count limit"))?,
        maximum_sample_stride: u32::try_from(values[3])
            .map_err(|_| GraphProbeError::IntegerOverflow("sample stride limit"))?,
    })
}

const fn limits_within(embedded: GraphProbeLimits, caller: GraphProbeLimits) -> bool {
    embedded.maximum_probe_document_bytes <= caller.maximum_probe_document_bytes
        && embedded.maximum_probes <= caller.maximum_probes
        && embedded.maximum_samples_per_probe <= caller.maximum_samples_per_probe
        && embedded.maximum_sample_stride <= caller.maximum_sample_stride
}

#[derive(Default)]
struct Encoder(Vec<u8>);

impl Encoder {
    fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
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

    fn string(&mut self, value: &str) -> Result<(), GraphProbeError> {
        self.u32(
            u32::try_from(value.len())
                .map_err(|_| GraphProbeError::IntegerOverflow("name length"))?,
        );
        self.bytes(value.as_bytes());
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

    fn take(&mut self, count: usize) -> Result<&'a [u8], GraphProbeError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or(GraphProbeError::Truncated)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GraphProbeError::Truncated)?;
        self.cursor = end;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], GraphProbeError> {
        self.take(N)?
            .try_into()
            .map_err(|_| GraphProbeError::Truncated)
    }

    fn u16(&mut self) -> Result<u16, GraphProbeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, GraphProbeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, GraphProbeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn count(&mut self, maximum: usize, name: &'static str) -> Result<usize, GraphProbeError> {
        let count =
            usize::try_from(self.u32()?).map_err(|_| GraphProbeError::IntegerOverflow(name))?;
        if count > maximum {
            Err(GraphProbeError::LimitExceeded(name))
        } else {
            Ok(count)
        }
    }

    fn string(&mut self) -> Result<String, GraphProbeError> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| GraphProbeError::IntegerOverflow("name length"))?;
        if length > PROBE_NAME_BYTES {
            return Err(GraphProbeError::LimitExceeded("name length"));
        }
        let value = str::from_utf8(self.take(length)?).map_err(|_| GraphProbeError::InvalidUtf8)?;
        Ok(value.to_owned())
    }

    const fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        GraphNodePlacement, GraphWorkspaceLimits, RepresentativeControlSignal,
        compile_representative_exact_control_graph,
    };

    fn workspace() -> GraphWorkspaceDocument {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let placements = fixture
            .document()
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| {
                GraphNodePlacement::new(node.id(), i32::try_from(index).unwrap() * 20, 0)
            })
            .collect();
        GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            22,
            26,
            fixture.document().clone(),
            placements,
        )
        .unwrap()
    }

    fn probes(workspace: &GraphWorkspaceDocument) -> GraphProbeDocument {
        let definitions = [
            (1, "error", RepresentativeControlSignal::Error.endpoint()),
            (
                2,
                "integral-prior",
                RepresentativeControlSignal::IntegralPrior.endpoint(),
            ),
            (
                3,
                "controller-clamped",
                RepresentativeControlSignal::ClampedController.endpoint(),
            ),
            (
                4,
                "output-permitted",
                RepresentativeControlSignal::PermittedOutput.endpoint(),
            ),
        ]
        .into_iter()
        .map(|(id, name, source)| {
            GraphProbeDefinition::new(
                GraphProbeId::new(id),
                name,
                source,
                GraphProbeCapture::new(4_096, 1),
            )
        })
        .collect();
        GraphProbeDocument::try_new(
            GraphProbeLimits::interactive(),
            1,
            5,
            None,
            workspace,
            definitions,
        )
        .unwrap()
    }

    fn triggered_probes(workspace: &GraphWorkspaceDocument) -> GraphProbeDocument {
        let probes = vec![
            GraphProbeDefinition::new(
                GraphProbeId::new(1),
                "measurement-in-range",
                RepresentativeControlSignal::MeasurementWithinRange.endpoint(),
                GraphProbeCapture::new(4_096, 1),
            ),
            GraphProbeDefinition::new(
                GraphProbeId::new(2),
                "error",
                RepresentativeControlSignal::Error.endpoint(),
                GraphProbeCapture::new(4_096, 1),
            ),
        ];
        GraphProbeDocument::try_new(
            GraphProbeLimits::interactive(),
            1,
            3,
            Some(GraphProbeTrigger::new(
                GraphProbeId::new(1),
                GraphProbeEdge::Falling,
                2,
                2,
            )),
            workspace,
            probes,
        )
        .unwrap()
    }

    #[test]
    fn canonical_probe_sidecar_round_trips_exactly() {
        let workspace = workspace();
        let document = probes(&workspace);
        let encoded = encode_graph_probes(&document).unwrap();
        let replay =
            replay_graph_probes(encoded.bytes(), &workspace, GraphProbeLimits::interactive())
                .unwrap();
        assert_eq!(replay.document(), &document);
        assert_eq!(replay.encoding(), &encoded);
        assert_eq!(document.probes().len(), 4);
        assert!(document.observes(RepresentativeControlSignal::Error.endpoint()));
        assert_eq!(document.probes()[0].value_type(), GraphTypeId::new(5));
    }

    #[test]
    fn probe_mutation_and_workspace_rebinding_are_transactional() {
        let mut workspace = workspace();
        let mut document = probes(&workspace);
        let unchanged = document.clone();
        document.replace_workspace(&workspace).unwrap();
        assert_eq!(document, unchanged);
        let old_digest = document.workspace_digest();
        workspace.move_node(GraphNodeId::new(7), 800, 50).unwrap();
        document.replace_workspace(&workspace).unwrap();
        assert_ne!(document.workspace_digest(), old_digest);
        let id = document
            .add_probe(
                &workspace,
                "extra-output",
                WireEndpoint {
                    node: GraphNodeId::new(16),
                    port: GraphPortId::new(3),
                },
                GraphProbeCapture::new(64, 2),
            )
            .unwrap();
        assert_eq!(id, GraphProbeId::new(5));
        document.remove_probe(&workspace, id).unwrap();
        assert_eq!(document.next_probe_id(), 6);

        let before = document.clone();
        workspace.delete_node(GraphNodeId::new(7)).unwrap();
        assert_eq!(
            document.replace_workspace(&workspace),
            Err(GraphProbeError::UnknownOutput(
                RepresentativeControlSignal::Error.endpoint()
            ))
        );
        assert_eq!(document, before);
    }

    #[test]
    fn boolean_edge_trigger_round_trips_and_resolves_exact_pre_post_window() {
        let workspace = workspace();
        let document = triggered_probes(&workspace);
        let encoded = encode_graph_probes(&document).unwrap();
        let replay =
            replay_graph_probes(encoded.bytes(), &workspace, GraphProbeLimits::interactive())
                .unwrap();
        assert_eq!(replay.document(), &document);
        assert_eq!(replay.encoding(), &encoded);
        assert_eq!(document.trigger().unwrap().edge(), GraphProbeEdge::Falling);
        assert!(
            document
                .supports_edge_trigger(&workspace, GraphProbeId::new(1))
                .unwrap()
        );
        assert!(
            !document
                .supports_edge_trigger(&workspace, GraphProbeId::new(2))
                .unwrap()
        );

        let fixture = compile_representative_exact_control_graph().unwrap();
        let GraphProbeTriggerResolution::Matched(matched) =
            resolve_graph_probe_trigger(&document, &workspace, fixture.simulation()).unwrap()
        else {
            panic!("reference falling edge did not match");
        };
        assert_eq!(matched.trigger_tick(), 3);
        assert_eq!(matched.trigger_sequence(), 3);
        assert_eq!((matched.first_tick(), matched.last_tick()), (1, 5));
        assert_eq!(matched.retained_pretrigger_samples(), 2);
        assert_eq!(matched.retained_posttrigger_samples(), 2);
        assert!(matched.pretrigger_complete());
        assert!(matched.posttrigger_complete());
    }

    #[test]
    fn trigger_resolution_preserves_stride_waiting_disabled_and_truncation_states() {
        let workspace = workspace();
        let fixture = compile_representative_exact_control_graph().unwrap();
        let external = |capture, trigger| {
            GraphProbeDocument::try_new(
                GraphProbeLimits::interactive(),
                1,
                2,
                trigger,
                &workspace,
                vec![GraphProbeDefinition::new(
                    GraphProbeId::new(1),
                    "external-permit",
                    RepresentativeControlSignal::ExternalPermit.endpoint(),
                    capture,
                )],
            )
            .unwrap()
        };
        let strided = external(
            GraphProbeCapture::new(16, 2),
            Some(GraphProbeTrigger::new(
                GraphProbeId::new(1),
                GraphProbeEdge::Falling,
                1,
                0,
            )),
        );
        let GraphProbeTriggerResolution::Matched(strided_match) =
            resolve_graph_probe_trigger(&strided, &workspace, fixture.simulation()).unwrap()
        else {
            panic!("strided falling edge did not match");
        };
        assert_eq!(strided_match.trigger_tick(), 4);
        assert_eq!(
            (strided_match.first_tick(), strided_match.last_tick()),
            (2, 4)
        );

        let truncated = external(
            GraphProbeCapture::new(11, 1),
            Some(GraphProbeTrigger::new(
                GraphProbeId::new(1),
                GraphProbeEdge::Falling,
                5,
                5,
            )),
        );
        let GraphProbeTriggerResolution::Matched(truncated_match) =
            resolve_graph_probe_trigger(&truncated, &workspace, fixture.simulation()).unwrap()
        else {
            panic!("truncated falling edge did not match");
        };
        assert_eq!(
            (truncated_match.first_tick(), truncated_match.last_tick()),
            (0, 5)
        );
        assert_eq!(truncated_match.retained_pretrigger_samples(), 4);
        assert_eq!(truncated_match.retained_posttrigger_samples(), 1);
        assert!(!truncated_match.pretrigger_complete());
        assert!(!truncated_match.posttrigger_complete());

        let waiting = external(
            GraphProbeCapture::new(3, 1),
            Some(GraphProbeTrigger::new(
                GraphProbeId::new(1),
                GraphProbeEdge::Rising,
                1,
                1,
            )),
        );
        assert!(matches!(
            resolve_graph_probe_trigger(&waiting, &workspace, fixture.simulation()).unwrap(),
            GraphProbeTriggerResolution::Waiting(_)
        ));
        assert_eq!(
            resolve_graph_probe_trigger(&probes(&workspace), &workspace, fixture.simulation())
                .unwrap(),
            GraphProbeTriggerResolution::Disabled
        );
    }

    #[test]
    fn trigger_edits_are_transactional_and_removing_source_clears_trigger() {
        let workspace = workspace();
        let mut document = triggered_probes(&workspace);
        let initial = document.clone();
        document
            .set_trigger(&workspace, initial.trigger().unwrap())
            .unwrap();
        assert_eq!(document, initial);

        document
            .set_trigger(
                &workspace,
                GraphProbeTrigger::new(GraphProbeId::new(1), GraphProbeEdge::Either, 1, 1),
            )
            .unwrap();
        assert_eq!(document.revision(), initial.revision() + 1);
        document
            .remove_probe(&workspace, GraphProbeId::new(1))
            .unwrap();
        assert_eq!(document.trigger(), None);
        assert_eq!(document.next_probe_id(), 3);

        let cleared = document.clone();
        document.clear_trigger(&workspace).unwrap();
        assert_eq!(document, cleared);
    }

    #[test]
    fn trigger_rejects_unknown_non_boolean_and_excessive_window() {
        let workspace = workspace();
        let definitions = vec![GraphProbeDefinition::new(
            GraphProbeId::new(1),
            "error",
            RepresentativeControlSignal::Error.endpoint(),
            GraphProbeCapture::new(4, 1),
        )];
        assert!(matches!(
            GraphProbeDocument::try_new(
                GraphProbeLimits::interactive(),
                1,
                2,
                Some(GraphProbeTrigger::new(
                    GraphProbeId::new(1),
                    GraphProbeEdge::Falling,
                    1,
                    1,
                )),
                &workspace,
                definitions.clone(),
            ),
            Err(GraphProbeError::NonBooleanTriggerProbe { .. })
        ));
        assert_eq!(
            GraphProbeDocument::try_new(
                GraphProbeLimits::interactive(),
                1,
                2,
                Some(GraphProbeTrigger::new(
                    GraphProbeId::new(2),
                    GraphProbeEdge::Falling,
                    1,
                    1,
                )),
                &workspace,
                definitions,
            ),
            Err(GraphProbeError::UnknownTriggerProbe(GraphProbeId::new(2)))
        );

        let mut document = triggered_probes(&workspace);
        let before = document.clone();
        assert_eq!(
            document.set_trigger(
                &workspace,
                GraphProbeTrigger::new(GraphProbeId::new(1), GraphProbeEdge::Falling, 4_096, 0,),
            ),
            Err(GraphProbeError::LimitExceeded("trigger window"))
        );
        assert_eq!(document, before);
    }

    #[test]
    fn replay_rejects_wrong_workspace_corruption_trailing_bytes_and_tighter_policy() {
        let workspace = workspace();
        let encoded = encode_graph_probes(&probes(&workspace)).unwrap();
        for end in 0..encoded.bytes().len() {
            assert!(
                replay_graph_probes(
                    &encoded.bytes()[..end],
                    &workspace,
                    GraphProbeLimits::interactive(),
                )
                .is_err()
            );
        }
        let mut other = workspace.clone();
        other.move_node(GraphNodeId::new(1), 9_000, 0).unwrap();
        assert!(matches!(
            replay_graph_probes(encoded.bytes(), &other, GraphProbeLimits::interactive()),
            Err(GraphProbeError::WorkspaceIdentityMismatch { .. })
        ));

        let mut corrupted = encoded.bytes().to_vec();
        corrupted[0] ^= 1;
        assert_eq!(
            replay_graph_probes(&corrupted, &workspace, GraphProbeLimits::interactive()),
            Err(GraphProbeError::InvalidMagic)
        );
        let mut trailing = encoded.bytes().to_vec();
        trailing.push(0);
        assert_eq!(
            replay_graph_probes(&trailing, &workspace, GraphProbeLimits::interactive()),
            Err(GraphProbeError::TrailingBytes)
        );
        const TRIGGER_KIND_OFFSET: usize = 4 + 2 + 2 + 4 * 8 + 8 + 8 + 32;
        let mut invalid_trigger = encoded.bytes().to_vec();
        invalid_trigger[TRIGGER_KIND_OFFSET] = 9;
        assert_eq!(
            replay_graph_probes(
                &invalid_trigger,
                &workspace,
                GraphProbeLimits::interactive()
            ),
            Err(GraphProbeError::InvalidTriggerKind(9))
        );
        let mut noncanonical_disabled_trigger = encoded.bytes().to_vec();
        noncanonical_disabled_trigger[TRIGGER_KIND_OFFSET + 4] = 1;
        assert_eq!(
            replay_graph_probes(
                &noncanonical_disabled_trigger,
                &workspace,
                GraphProbeLimits::interactive()
            ),
            Err(GraphProbeError::NonCanonical)
        );
        let mut tight = GraphProbeLimits::interactive();
        tight.maximum_probes = 3;
        assert_eq!(
            replay_graph_probes(encoded.bytes(), &workspace, tight),
            Err(GraphProbeError::LimitExceeded("embedded limits"))
        );
    }

    #[test]
    fn invalid_sources_names_duplicates_and_capture_bounds_fail_closed() {
        let workspace = workspace();
        let base = GraphProbeDefinition::new(
            GraphProbeId::new(1),
            "valid",
            RepresentativeControlSignal::Error.endpoint(),
            GraphProbeCapture::new(1, 1),
        );
        let invalid = GraphProbeDefinition::new(
            GraphProbeId::new(2),
            "invalid name",
            RepresentativeControlSignal::IntegralPrior.endpoint(),
            GraphProbeCapture::new(1, 1),
        );
        assert_eq!(
            GraphProbeDocument::try_new(
                GraphProbeLimits::interactive(),
                0,
                3,
                None,
                &workspace,
                vec![base.clone(), invalid],
            ),
            Err(GraphProbeError::InvalidName)
        );
        let duplicate = GraphProbeDefinition::new(
            GraphProbeId::new(2),
            "duplicate-source",
            base.source(),
            GraphProbeCapture::new(1, 1),
        );
        assert_eq!(
            GraphProbeDocument::try_new(
                GraphProbeLimits::interactive(),
                0,
                3,
                None,
                &workspace,
                vec![base.clone(), duplicate],
            ),
            Err(GraphProbeError::DuplicateSource(base.source()))
        );
        let unbounded = GraphProbeDefinition::new(
            GraphProbeId::new(2),
            "unbounded",
            RepresentativeControlSignal::IntegralPrior.endpoint(),
            GraphProbeCapture::new(0, 1),
        );
        assert_eq!(
            GraphProbeDocument::try_new(
                GraphProbeLimits::interactive(),
                0,
                3,
                None,
                &workspace,
                vec![base, unbounded],
            ),
            Err(GraphProbeError::LimitExceeded("sample count"))
        );
        let unknown = GraphProbeDefinition::new(
            GraphProbeId::new(1),
            "unknown",
            WireEndpoint {
                node: GraphNodeId::new(1),
                port: GraphPortId::new(99),
            },
            GraphProbeCapture::new(1, 1),
        );
        assert_eq!(
            GraphProbeDocument::try_new(
                GraphProbeLimits::interactive(),
                0,
                2,
                None,
                &workspace,
                vec![unknown],
            ),
            Err(GraphProbeError::UnknownOutput(WireEndpoint {
                node: GraphNodeId::new(1),
                port: GraphPortId::new(99),
            }))
        );
    }
}
