//! Canonical, bounded persistence for one exact graph-authoring session.
//!
//! `ALGS` binds the editable control workspace, its probe sidecar, the inert
//! cached-job workspace, and an optional complete component hierarchy plus
//! exact source map. It is host authoring state only and grants no semantic,
//! deployment, firmware, resource, timing, or safety authority.

use core::fmt;

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::{
    CanonicalGraphComponentEncoding, CanonicalGraphHierarchyEncoding,
    CanonicalGraphHierarchySourceMapEncoding, CanonicalGraphProbeEncoding,
    CanonicalGraphWorkspaceEncoding, GraphComponentDocument, GraphComponentLimits,
    GraphHierarchyDocument, GraphHierarchyError, GraphHierarchyFlattening, GraphHierarchyLimits,
    GraphHierarchySourceMapError, GraphHierarchySourceMapLimits, GraphLimits, GraphProbeDocument,
    GraphProbeError, GraphProbeLimits, GraphWorkspaceDocument, GraphWorkspaceError,
    GraphWorkspaceLimits, encode_graph_hierarchy, encode_graph_probes, encode_graph_workspace,
    replay_graph_hierarchy, replay_graph_hierarchy_source_map, replay_graph_probes,
    replay_graph_workspace,
};

/// Magic bytes at the beginning of each canonical authoring session.
pub const GRAPH_AUTHORING_SESSION_MAGIC: [u8; 4] = *b"ALGS";

/// Exact canonical authoring-session version implemented by this tree.
pub const GRAPH_AUTHORING_SESSION_VERSION: u16 = 1;

/// Hard first-release complete `ALGS` byte ceiling.
pub const MAX_GRAPH_AUTHORING_SESSION_BYTES: usize = 8 * 1024 * 1024;

const GRAPH_AUTHORING_SESSION_FLAGS: u16 = 0;
const SESSION_LIMIT_FIELD_COUNT: usize = 6;
const SESSION_FIXED_BYTES: usize = 4 + 2 + 2 + SESSION_LIMIT_FIELD_COUNT * 8 + 4 + 4 + 4 + 1;
const SESSION_HIERARCHY_FIXED_BYTES: usize = 32 + 4 + 4;

/// Caller-owned and canonically embedded authoring-session section bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphAuthoringSessionLimits {
    /// Maximum complete canonical session bytes.
    pub maximum_session_bytes: usize,
    /// Maximum embedded editable-control `ALGW` bytes.
    pub maximum_control_workspace_bytes: usize,
    /// Maximum embedded `ALGP` bytes.
    pub maximum_probe_bytes: usize,
    /// Maximum embedded inert cached-job `ALGW` bytes.
    pub maximum_cached_job_workspace_bytes: usize,
    /// Maximum embedded complete `ALGH` bytes when hierarchy is present.
    pub maximum_hierarchy_bytes: usize,
    /// Maximum embedded exact `ALGM` bytes when hierarchy is present.
    pub maximum_source_map_bytes: usize,
}

impl GraphAuthoringSessionLimits {
    /// First bounded browser/native authoring-session policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_session_bytes: MAX_GRAPH_AUTHORING_SESSION_BYTES,
            maximum_control_workspace_bytes: 2 * 1024 * 1024,
            maximum_probe_bytes: 2 * 1024 * 1024,
            maximum_cached_job_workspace_bytes: 2 * 1024 * 1024,
            maximum_hierarchy_bytes: 4 * 1024 * 1024,
            maximum_source_map_bytes: 4 * 1024 * 1024,
        }
    }

    fn validate(self) -> Result<(), GraphAuthoringSessionError> {
        if self.maximum_session_bytes == 0
            || self.maximum_control_workspace_bytes == 0
            || self.maximum_probe_bytes == 0
            || self.maximum_cached_job_workspace_bytes == 0
            || self.maximum_hierarchy_bytes == 0
            || self.maximum_source_map_bytes == 0
        {
            Err(GraphAuthoringSessionError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphAuthoringSessionLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Complete caller admission policy for nested canonical replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphAuthoringSessionReplayLimits {
    /// Outer session and section bounds.
    pub session: GraphAuthoringSessionLimits,
    /// Both embedded workspace envelopes.
    pub workspace: GraphWorkspaceLimits,
    /// Structural graphs inside every workspace/component.
    pub graph: GraphLimits,
    /// Probe sidecar bounds.
    pub probes: GraphProbeLimits,
    /// Hierarchy package bounds.
    pub hierarchy: GraphHierarchyLimits,
    /// Component package bounds.
    pub components: GraphComponentLimits,
    /// Hierarchy source-map bounds.
    pub source_map: GraphHierarchySourceMapLimits,
}

impl GraphAuthoringSessionReplayLimits {
    /// First complete interactive replay policy.
    pub const fn interactive() -> Self {
        Self {
            session: GraphAuthoringSessionLimits::interactive(),
            workspace: GraphWorkspaceLimits::interactive(),
            graph: GraphLimits::interactive(),
            probes: GraphProbeLimits::interactive(),
            hierarchy: GraphHierarchyLimits::interactive(),
            components: GraphComponentLimits::interactive(),
            source_map: GraphHierarchySourceMapLimits::interactive(),
        }
    }
}

impl Default for GraphAuthoringSessionReplayLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Caller input for the optional exact component-hierarchy branch.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphAuthoringHierarchyInput {
    selected_component: Digest,
    hierarchy: GraphHierarchyDocument,
    source_map: CanonicalGraphHierarchySourceMapEncoding,
}

impl GraphAuthoringHierarchyInput {
    /// Bind one selected component to a complete hierarchy and exact source map.
    pub const fn new(
        selected_component: Digest,
        hierarchy: GraphHierarchyDocument,
        source_map: CanonicalGraphHierarchySourceMapEncoding,
    ) -> Self {
        Self {
            selected_component,
            hierarchy,
            source_map,
        }
    }
}

/// Validated optional hierarchy retained by one authoring session.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphAuthoringSessionHierarchy {
    selected_component: Digest,
    selected_component_index: usize,
    hierarchy: GraphHierarchyDocument,
    hierarchy_encoding: CanonicalGraphHierarchyEncoding,
    flattening: GraphHierarchyFlattening,
    source_map: CanonicalGraphHierarchySourceMapEncoding,
}

impl GraphAuthoringSessionHierarchy {
    /// Return the exact selected `ALGC` identity.
    pub const fn selected_component(&self) -> Digest {
        self.selected_component
    }

    /// Borrow the selected component proven to belong to this hierarchy.
    pub fn selected_component_document(&self) -> &GraphComponentDocument {
        self.hierarchy.dependencies()[self.selected_component_index].document()
    }

    /// Borrow exact canonical bytes for the selected component.
    pub fn selected_component_encoding(&self) -> &CanonicalGraphComponentEncoding {
        self.hierarchy.dependencies()[self.selected_component_index].encoding()
    }

    /// Borrow the complete canonical hierarchy document.
    pub const fn document(&self) -> &GraphHierarchyDocument {
        &self.hierarchy
    }

    /// Borrow the exact complete `ALGH` encoding.
    pub const fn encoding(&self) -> &CanonicalGraphHierarchyEncoding {
        &self.hierarchy_encoding
    }

    /// Borrow the freshly regenerated flattening and total provenance.
    pub const fn flattening(&self) -> &GraphHierarchyFlattening {
        &self.flattening
    }

    /// Borrow the byte-for-byte replayed `ALGM` encoding.
    pub const fn source_map(&self) -> &CanonicalGraphHierarchySourceMapEncoding {
        &self.source_map
    }
}

/// Complete validated host authoring state.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphAuthoringSessionDocument {
    limits: GraphAuthoringSessionLimits,
    control_workspace: GraphWorkspaceDocument,
    control_encoding: CanonicalGraphWorkspaceEncoding,
    probes: GraphProbeDocument,
    probe_encoding: CanonicalGraphProbeEncoding,
    cached_job_workspace: GraphWorkspaceDocument,
    cached_job_encoding: CanonicalGraphWorkspaceEncoding,
    hierarchy: Option<GraphAuthoringSessionHierarchy>,
}

impl GraphAuthoringSessionDocument {
    /// Validate every artifact and all cross-artifact identity relationships.
    pub fn try_new(
        limits: GraphAuthoringSessionLimits,
        source_map_admission: GraphHierarchySourceMapLimits,
        control_workspace: GraphWorkspaceDocument,
        probes: GraphProbeDocument,
        cached_job_workspace: GraphWorkspaceDocument,
        hierarchy: Option<GraphAuthoringHierarchyInput>,
    ) -> Result<Self, GraphAuthoringSessionError> {
        limits.validate()?;
        let control_encoding = encode_graph_workspace(&control_workspace)
            .map_err(GraphAuthoringSessionError::ControlWorkspace)?;
        check_section(
            control_encoding.bytes().len(),
            limits.maximum_control_workspace_bytes,
            "control workspace byte length",
        )?;
        let probe_encoding =
            encode_graph_probes(&probes).map_err(GraphAuthoringSessionError::Probes)?;
        check_section(
            probe_encoding.bytes().len(),
            limits.maximum_probe_bytes,
            "probe byte length",
        )?;
        if probes.workspace_digest() != control_encoding.digest() {
            return Err(GraphAuthoringSessionError::ProbeWorkspaceMismatch {
                expected: control_encoding.digest(),
                actual: probes.workspace_digest(),
            });
        }
        let cached_job_encoding = encode_graph_workspace(&cached_job_workspace)
            .map_err(GraphAuthoringSessionError::CachedJobWorkspace)?;
        check_section(
            cached_job_encoding.bytes().len(),
            limits.maximum_cached_job_workspace_bytes,
            "cached-job workspace byte length",
        )?;

        let hierarchy = hierarchy
            .map(|input| {
                validate_hierarchy_input(
                    input,
                    &control_workspace,
                    control_encoding.digest(),
                    limits,
                    source_map_admission,
                )
            })
            .transpose()?;
        let document = Self {
            limits,
            control_workspace,
            control_encoding,
            probes,
            probe_encoding,
            cached_job_workspace,
            cached_job_encoding,
            hierarchy,
        };
        encode_graph_authoring_session(&document)?;
        Ok(document)
    }

    /// Return embedded outer and section limits.
    pub const fn limits(&self) -> GraphAuthoringSessionLimits {
        self.limits
    }

    /// Borrow the editable control workspace.
    pub const fn control_workspace(&self) -> &GraphWorkspaceDocument {
        &self.control_workspace
    }

    /// Borrow exact canonical control-workspace bytes.
    pub const fn control_encoding(&self) -> &CanonicalGraphWorkspaceEncoding {
        &self.control_encoding
    }

    /// Borrow the exact control-workspace probe sidecar.
    pub const fn probes(&self) -> &GraphProbeDocument {
        &self.probes
    }

    /// Borrow exact canonical probe bytes.
    pub const fn probe_encoding(&self) -> &CanonicalGraphProbeEncoding {
        &self.probe_encoding
    }

    /// Borrow the inert cached-job workspace.
    pub const fn cached_job_workspace(&self) -> &GraphWorkspaceDocument {
        &self.cached_job_workspace
    }

    /// Borrow exact canonical cached-job workspace bytes.
    pub const fn cached_job_encoding(&self) -> &CanonicalGraphWorkspaceEncoding {
        &self.cached_job_encoding
    }

    /// Borrow the optional exact selected-component hierarchy branch.
    pub const fn hierarchy(&self) -> Option<&GraphAuthoringSessionHierarchy> {
        self.hierarchy.as_ref()
    }
}

/// Canonical session bytes and exact SHA-256 identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphAuthoringSessionEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphAuthoringSessionEncoding {
    /// Borrow complete canonical `ALGS` bytes.
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

/// Independently replayed canonical authoring session.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphAuthoringSessionReplay {
    document: GraphAuthoringSessionDocument,
    encoding: CanonicalGraphAuthoringSessionEncoding,
}

impl GraphAuthoringSessionReplay {
    /// Borrow the fully reconstructed authoring session.
    pub const fn document(&self) -> &GraphAuthoringSessionDocument {
        &self.document
    }

    /// Borrow exact imported and regenerated bytes.
    pub const fn encoding(&self) -> &CanonicalGraphAuthoringSessionEncoding {
        &self.encoding
    }

    /// Consume the replay result and return the validated document.
    pub fn into_document(self) -> GraphAuthoringSessionDocument {
        self.document
    }
}

/// Rejection at the canonical authoring-session boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphAuthoringSessionError {
    /// A caller or embedded session policy contained zero.
    ZeroLimit,
    /// A bounded section, document, or embedded policy exceeded admission.
    LimitExceeded(&'static str),
    /// Input did not begin with [`GRAPH_AUTHORING_SESSION_MAGIC`].
    InvalidMagic,
    /// Authoring-session version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved flags were nonzero.
    UnsupportedFlags(u16),
    /// Optional-hierarchy tag was neither absent nor present.
    InvalidHierarchyTag(u8),
    /// A fixed-width or length-delimited field ran past input.
    Truncated,
    /// A canonical count or length could not fit its wire width.
    IntegerOverflow(&'static str),
    /// Valid bytes remained after the complete session.
    TrailingBytes,
    /// Fresh nested replay or re-encoding changed a canonical fact.
    NonCanonical,
    /// The probe sidecar names another control workspace.
    ProbeWorkspaceMismatch {
        /// Exact control workspace identity.
        expected: Digest,
        /// Workspace identity carried by `ALGP`.
        actual: Digest,
    },
    /// The selected component digest is absent from the complete hierarchy.
    SelectedComponentMissing(Digest),
    /// The selected component embeds another workspace.
    SelectedWorkspaceMismatch {
        /// Exact editable control workspace identity.
        expected: Digest,
        /// Workspace identity embedded by the selected component.
        actual: Digest,
    },
    /// Editable control `ALGW` validation failed.
    ControlWorkspace(GraphWorkspaceError),
    /// `ALGP` validation failed.
    Probes(GraphProbeError),
    /// Inert cached-job `ALGW` validation failed.
    CachedJobWorkspace(GraphWorkspaceError),
    /// Complete `ALGH` validation failed.
    Hierarchy(GraphHierarchyError),
    /// Exact `ALGM` replay failed.
    SourceMap(GraphHierarchySourceMapError),
}

impl fmt::Display for GraphAuthoringSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph authoring-session policy contains zero"),
            Self::LimitExceeded(name) => {
                write!(formatter, "graph authoring-session {name} exceeds policy")
            }
            Self::InvalidMagic => formatter.write_str("graph authoring-session magic is invalid"),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "graph authoring-session version {version} is unsupported"
            ),
            Self::UnsupportedFlags(flags) => write!(
                formatter,
                "graph authoring-session flags {flags:#06x} are unsupported"
            ),
            Self::InvalidHierarchyTag(tag) => write!(
                formatter,
                "graph authoring-session hierarchy tag {tag} is invalid"
            ),
            Self::Truncated => formatter.write_str("graph authoring session is truncated"),
            Self::IntegerOverflow(name) => write!(
                formatter,
                "graph authoring-session {name} exceeds its integer width"
            ),
            Self::TrailingBytes => {
                formatter.write_str("graph authoring session has trailing bytes")
            }
            Self::NonCanonical => {
                formatter.write_str("graph authoring-session bytes are noncanonical")
            }
            Self::ProbeWorkspaceMismatch { expected, actual } => write!(
                formatter,
                "graph authoring-session probe workspace {actual:?} does not match {expected:?}"
            ),
            Self::SelectedComponentMissing(component) => write!(
                formatter,
                "graph authoring-session selected component {component:?} is absent"
            ),
            Self::SelectedWorkspaceMismatch { expected, actual } => write!(
                formatter,
                "graph authoring-session selected workspace {actual:?} does not match {expected:?}"
            ),
            Self::ControlWorkspace(error) => {
                write!(
                    formatter,
                    "graph authoring control workspace failed: {error}"
                )
            }
            Self::Probes(error) => write!(formatter, "graph authoring probes failed: {error}"),
            Self::CachedJobWorkspace(error) => {
                write!(
                    formatter,
                    "graph authoring cached-job workspace failed: {error}"
                )
            }
            Self::Hierarchy(error) => {
                write!(formatter, "graph authoring hierarchy failed: {error}")
            }
            Self::SourceMap(error) => {
                write!(formatter, "graph authoring source map failed: {error}")
            }
        }
    }
}

impl std::error::Error for GraphAuthoringSessionError {}

/// Encode one validated complete authoring session.
pub fn encode_graph_authoring_session(
    document: &GraphAuthoringSessionDocument,
) -> Result<CanonicalGraphAuthoringSessionEncoding, GraphAuthoringSessionError> {
    let control = document.control_encoding.bytes();
    let probes = document.probe_encoding.bytes();
    let cached_jobs = document.cached_job_encoding.bytes();
    check_section(
        control.len(),
        document.limits.maximum_control_workspace_bytes,
        "control workspace byte length",
    )?;
    check_section(
        probes.len(),
        document.limits.maximum_probe_bytes,
        "probe byte length",
    )?;
    check_section(
        cached_jobs.len(),
        document.limits.maximum_cached_job_workspace_bytes,
        "cached-job workspace byte length",
    )?;
    let control_length = to_u32(control.len(), "control workspace byte length")?;
    let probe_length = to_u32(probes.len(), "probe byte length")?;
    let cached_job_length = to_u32(cached_jobs.len(), "cached-job workspace byte length")?;
    let mut encoded_length = SESSION_FIXED_BYTES
        .checked_add(control.len())
        .and_then(|length| length.checked_add(probes.len()))
        .and_then(|length| length.checked_add(cached_jobs.len()))
        .ok_or(GraphAuthoringSessionError::IntegerOverflow(
            "document byte length",
        ))?;
    let hierarchy_lengths = if let Some(hierarchy) = &document.hierarchy {
        let hierarchy_bytes = hierarchy.hierarchy_encoding.bytes();
        let source_map = hierarchy.source_map.bytes();
        check_section(
            hierarchy_bytes.len(),
            document.limits.maximum_hierarchy_bytes,
            "hierarchy byte length",
        )?;
        check_section(
            source_map.len(),
            document.limits.maximum_source_map_bytes,
            "source-map byte length",
        )?;
        let hierarchy_length = to_u32(hierarchy_bytes.len(), "hierarchy byte length")?;
        let source_map_length = to_u32(source_map.len(), "source-map byte length")?;
        encoded_length = encoded_length
            .checked_add(SESSION_HIERARCHY_FIXED_BYTES)
            .and_then(|length| length.checked_add(hierarchy_bytes.len()))
            .and_then(|length| length.checked_add(source_map.len()))
            .ok_or(GraphAuthoringSessionError::IntegerOverflow(
                "document byte length",
            ))?;
        Some((hierarchy_length, source_map_length))
    } else {
        None
    };
    check_section(
        encoded_length,
        document.limits.maximum_session_bytes,
        "document byte length",
    )?;

    let mut encoder = Encoder::with_capacity(encoded_length);
    encoder.bytes(&GRAPH_AUTHORING_SESSION_MAGIC);
    encoder.u16(GRAPH_AUTHORING_SESSION_VERSION);
    encoder.u16(GRAPH_AUTHORING_SESSION_FLAGS);
    encode_limits(&mut encoder, document.limits)?;
    encoder.u32(control_length);
    encoder.bytes(control);
    encoder.u32(probe_length);
    encoder.bytes(probes);
    encoder.u32(cached_job_length);
    encoder.bytes(cached_jobs);
    if let (Some(hierarchy), Some((hierarchy_length, source_map_length))) =
        (&document.hierarchy, hierarchy_lengths)
    {
        encoder.u8(1);
        encoder.digest(hierarchy.selected_component);
        encoder.u32(hierarchy_length);
        encoder.bytes(hierarchy.hierarchy_encoding.bytes());
        encoder.u32(source_map_length);
        encoder.bytes(hierarchy.source_map.bytes());
    } else {
        encoder.u8(0);
    }
    if encoder.0.len() != encoded_length {
        return Err(GraphAuthoringSessionError::NonCanonical);
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphAuthoringSessionEncoding {
        bytes: encoder.0,
        digest,
    })
}

/// Replay every nested artifact and require exact complete session re-encoding.
pub fn replay_graph_authoring_session(
    bytes: &[u8],
    admission: GraphAuthoringSessionReplayLimits,
) -> Result<GraphAuthoringSessionReplay, GraphAuthoringSessionError> {
    admission.session.validate()?;
    check_section(
        bytes.len(),
        admission.session.maximum_session_bytes,
        "admitted document byte length",
    )?;
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_AUTHORING_SESSION_MAGIC.len())? != GRAPH_AUTHORING_SESSION_MAGIC {
        return Err(GraphAuthoringSessionError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_AUTHORING_SESSION_VERSION {
        return Err(GraphAuthoringSessionError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_AUTHORING_SESSION_FLAGS {
        return Err(GraphAuthoringSessionError::UnsupportedFlags(flags));
    }
    let limits = decode_limits(&mut decoder)?;
    limits.validate()?;
    if !limits_within(limits, admission.session) {
        return Err(GraphAuthoringSessionError::LimitExceeded(
            "embedded admission limit",
        ));
    }
    check_section(
        bytes.len(),
        limits.maximum_session_bytes,
        "embedded document byte length",
    )?;

    let control_bytes = decoder.length_prefixed(
        limits.maximum_control_workspace_bytes,
        "control workspace byte length",
    )?;
    let probe_bytes = decoder.length_prefixed(limits.maximum_probe_bytes, "probe byte length")?;
    let cached_job_bytes = decoder.length_prefixed(
        limits.maximum_cached_job_workspace_bytes,
        "cached-job workspace byte length",
    )?;
    let hierarchy_tag = decoder.u8()?;
    let hierarchy_bytes = match hierarchy_tag {
        0 => None,
        1 => {
            let selected = decoder.digest()?;
            let hierarchy =
                decoder.length_prefixed(limits.maximum_hierarchy_bytes, "hierarchy byte length")?;
            let source_map = decoder
                .length_prefixed(limits.maximum_source_map_bytes, "source-map byte length")?;
            Some((selected, hierarchy, source_map))
        }
        tag => return Err(GraphAuthoringSessionError::InvalidHierarchyTag(tag)),
    };
    if !decoder.is_empty() {
        return Err(GraphAuthoringSessionError::TrailingBytes);
    }

    let control_replay =
        replay_graph_workspace(control_bytes, admission.workspace, admission.graph)
            .map_err(GraphAuthoringSessionError::ControlWorkspace)?;
    let probe_replay =
        replay_graph_probes(probe_bytes, control_replay.document(), admission.probes)
            .map_err(GraphAuthoringSessionError::Probes)?;
    let cached_job_replay =
        replay_graph_workspace(cached_job_bytes, admission.workspace, admission.graph)
            .map_err(GraphAuthoringSessionError::CachedJobWorkspace)?;
    let hierarchy = hierarchy_bytes
        .map(|(selected_component, hierarchy_bytes, source_map_bytes)| {
            let replay = replay_graph_hierarchy(
                hierarchy_bytes,
                admission.hierarchy,
                admission.components,
                admission.workspace,
                admission.graph,
            )
            .map_err(GraphAuthoringSessionError::Hierarchy)?;
            let source_replay = replay_graph_hierarchy_source_map(
                source_map_bytes,
                replay.document(),
                admission.source_map,
            )
            .map_err(GraphAuthoringSessionError::SourceMap)?;
            Ok(GraphAuthoringHierarchyInput::new(
                selected_component,
                replay.document().clone(),
                source_replay.encoding().clone(),
            ))
        })
        .transpose()?;
    let document = GraphAuthoringSessionDocument::try_new(
        limits,
        admission.source_map,
        control_replay.document().clone(),
        probe_replay.document().clone(),
        cached_job_replay.document().clone(),
        hierarchy,
    )?;
    let encoding = encode_graph_authoring_session(&document)?;
    if encoding.bytes() != bytes {
        return Err(GraphAuthoringSessionError::NonCanonical);
    }
    Ok(GraphAuthoringSessionReplay { document, encoding })
}

fn validate_hierarchy_input(
    input: GraphAuthoringHierarchyInput,
    control_workspace: &GraphWorkspaceDocument,
    control_digest: Digest,
    limits: GraphAuthoringSessionLimits,
    source_map_admission: GraphHierarchySourceMapLimits,
) -> Result<GraphAuthoringSessionHierarchy, GraphAuthoringSessionError> {
    let hierarchy_encoding =
        encode_graph_hierarchy(&input.hierarchy).map_err(GraphAuthoringSessionError::Hierarchy)?;
    check_section(
        hierarchy_encoding.bytes().len(),
        limits.maximum_hierarchy_bytes,
        "hierarchy byte length",
    )?;
    let (selected_component_index, selected) = input
        .hierarchy
        .dependencies()
        .iter()
        .enumerate()
        .find(|(_, dependency)| dependency.digest() == input.selected_component)
        .ok_or(GraphAuthoringSessionError::SelectedComponentMissing(
            input.selected_component,
        ))?;
    let selected_workspace = encode_graph_workspace(selected.document().workspace())
        .map_err(GraphAuthoringSessionError::ControlWorkspace)?;
    if selected_workspace.digest() != control_digest {
        return Err(GraphAuthoringSessionError::SelectedWorkspaceMismatch {
            expected: control_digest,
            actual: selected_workspace.digest(),
        });
    }
    if selected.document().workspace() != control_workspace {
        return Err(GraphAuthoringSessionError::NonCanonical);
    }
    let source_replay = replay_graph_hierarchy_source_map(
        input.source_map.bytes(),
        &input.hierarchy,
        source_map_admission,
    )
    .map_err(GraphAuthoringSessionError::SourceMap)?;
    if source_replay.encoding() != &input.source_map {
        return Err(GraphAuthoringSessionError::NonCanonical);
    }
    check_section(
        input.source_map.bytes().len(),
        limits.maximum_source_map_bytes,
        "source-map byte length",
    )?;
    Ok(GraphAuthoringSessionHierarchy {
        selected_component: input.selected_component,
        selected_component_index,
        hierarchy: input.hierarchy,
        hierarchy_encoding,
        flattening: source_replay.into_flattening(),
        source_map: input.source_map,
    })
}

fn check_section(
    actual: usize,
    maximum: usize,
    name: &'static str,
) -> Result<(), GraphAuthoringSessionError> {
    if actual > maximum {
        Err(GraphAuthoringSessionError::LimitExceeded(name))
    } else {
        Ok(())
    }
}

fn encode_limits(
    encoder: &mut Encoder,
    limits: GraphAuthoringSessionLimits,
) -> Result<(), GraphAuthoringSessionError> {
    for value in [
        limits.maximum_session_bytes,
        limits.maximum_control_workspace_bytes,
        limits.maximum_probe_bytes,
        limits.maximum_cached_job_workspace_bytes,
        limits.maximum_hierarchy_bytes,
        limits.maximum_source_map_bytes,
    ] {
        encoder.u64(
            u64::try_from(value)
                .map_err(|_| GraphAuthoringSessionError::IntegerOverflow("limit value"))?,
        );
    }
    Ok(())
}

fn decode_limits(
    decoder: &mut Decoder<'_>,
) -> Result<GraphAuthoringSessionLimits, GraphAuthoringSessionError> {
    let mut values = [0_usize; SESSION_LIMIT_FIELD_COUNT];
    for value in &mut values {
        *value = usize::try_from(decoder.u64()?)
            .map_err(|_| GraphAuthoringSessionError::IntegerOverflow("limit value"))?;
    }
    Ok(GraphAuthoringSessionLimits {
        maximum_session_bytes: values[0],
        maximum_control_workspace_bytes: values[1],
        maximum_probe_bytes: values[2],
        maximum_cached_job_workspace_bytes: values[3],
        maximum_hierarchy_bytes: values[4],
        maximum_source_map_bytes: values[5],
    })
}

const fn limits_within(
    embedded: GraphAuthoringSessionLimits,
    admission: GraphAuthoringSessionLimits,
) -> bool {
    embedded.maximum_session_bytes <= admission.maximum_session_bytes
        && embedded.maximum_control_workspace_bytes <= admission.maximum_control_workspace_bytes
        && embedded.maximum_probe_bytes <= admission.maximum_probe_bytes
        && embedded.maximum_cached_job_workspace_bytes
            <= admission.maximum_cached_job_workspace_bytes
        && embedded.maximum_hierarchy_bytes <= admission.maximum_hierarchy_bytes
        && embedded.maximum_source_map_bytes <= admission.maximum_source_map_bytes
}

fn to_u32(value: usize, name: &'static str) -> Result<u32, GraphAuthoringSessionError> {
    u32::try_from(value).map_err(|_| GraphAuthoringSessionError::IntegerOverflow(name))
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

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphAuthoringSessionError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(GraphAuthoringSessionError::Truncated)?;
        let result = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GraphAuthoringSessionError::Truncated)?;
        self.cursor = end;
        Ok(result)
    }

    fn digest(&mut self) -> Result<Digest, GraphAuthoringSessionError> {
        let bytes: [u8; 32] = self
            .take(32)?
            .try_into()
            .map_err(|_| GraphAuthoringSessionError::Truncated)?;
        Ok(Digest(bytes))
    }

    fn u8(&mut self) -> Result<u8, GraphAuthoringSessionError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, GraphAuthoringSessionError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| GraphAuthoringSessionError::Truncated)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, GraphAuthoringSessionError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| GraphAuthoringSessionError::Truncated)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, GraphAuthoringSessionError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| GraphAuthoringSessionError::Truncated)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn length_prefixed(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<&'a [u8], GraphAuthoringSessionError> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| GraphAuthoringSessionError::IntegerOverflow(name))?;
        check_section(length, maximum, name)?;
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
        GraphComponentDocument, GraphComponentInstance, GraphNodeId, GraphNodePlacement,
        GraphProbeDocument, NodeDefinition, compile_representative_exact_control_graph,
        encode_graph_component, encode_graph_hierarchy_source_map,
        graph_component_instance_prototype,
    };

    fn session_fixture(
        hierarchy_present: bool,
    ) -> (
        GraphAuthoringSessionDocument,
        CanonicalGraphAuthoringSessionEncoding,
    ) {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let graph = fixture.document().clone();
        let next_node = u64::from(graph.nodes().last().unwrap().id().get()) + 1;
        let next_wire = u64::from(graph.wires().last().unwrap().id().get()) + 1;
        let placements = graph
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let coordinate = i32::try_from(index).unwrap() * 20;
                GraphNodePlacement::new(node.id(), coordinate, coordinate)
            })
            .collect();
        let workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            next_node,
            next_wire,
            graph,
            placements,
        )
        .unwrap();
        let probes = GraphProbeDocument::try_new(
            GraphProbeLimits::interactive(),
            1,
            1,
            None,
            &workspace,
            Vec::new(),
        )
        .unwrap();
        let hierarchy = hierarchy_present.then(|| {
            let component = GraphComponentDocument::try_new(
                GraphComponentLimits::interactive(),
                1,
                1,
                "control.authoring_session",
                1,
                1,
                1,
                workspace.clone(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
            let component_digest = encode_graph_component(&component).unwrap().digest();
            let prototype =
                graph_component_instance_prototype(&component, "Session component").unwrap();
            let instance_id = GraphNodeId::new(1);
            let instance = NodeDefinition::new(
                instance_id,
                prototype.kind().clone(),
                "Session component",
                prototype.domain(),
                prototype.inputs().to_vec(),
                prototype.outputs().to_vec(),
                prototype.parameters().to_vec(),
            );
            let root_graph = super::super::GraphDocument::try_new(
                1,
                workspace.graph().schema().clone(),
                workspace.graph().clocks().to_vec(),
                vec![instance],
                Vec::new(),
            )
            .unwrap();
            let root = GraphWorkspaceDocument::try_new(
                GraphWorkspaceLimits::interactive(),
                1,
                2,
                1,
                root_graph,
                vec![GraphNodePlacement::new(instance_id, 40, 40)],
            )
            .unwrap();
            let hierarchy = GraphHierarchyDocument::try_new(
                GraphHierarchyLimits::interactive(),
                1,
                root,
                vec![component],
                vec![GraphComponentInstance::root(instance_id, component_digest)],
            )
            .unwrap();
            let flattening = super::super::flatten_graph_hierarchy(&hierarchy).unwrap();
            let source_map = encode_graph_hierarchy_source_map(
                &flattening,
                GraphHierarchySourceMapLimits::interactive(),
            )
            .unwrap();
            GraphAuthoringHierarchyInput::new(component_digest, hierarchy, source_map)
        });
        let document = GraphAuthoringSessionDocument::try_new(
            GraphAuthoringSessionLimits::interactive(),
            GraphHierarchySourceMapLimits::interactive(),
            workspace.clone(),
            probes,
            workspace,
            hierarchy,
        )
        .unwrap();
        let encoding = encode_graph_authoring_session(&document).unwrap();
        (document, encoding)
    }

    #[test]
    fn complete_hierarchy_session_round_trips_every_identity() {
        let (document, encoding) = session_fixture(true);
        assert_eq!(encoding.bytes().len(), 14_794);
        assert_eq!(
            encoding.digest().0,
            [
                0x96, 0x40, 0x6a, 0x0c, 0x98, 0xbd, 0xec, 0x43, 0xe4, 0xf8, 0x46, 0x63, 0x62, 0x2f,
                0xa8, 0x28, 0x2f, 0xee, 0x32, 0x5a, 0xb4, 0x0f, 0x3c, 0x89, 0x80, 0x4e, 0x45, 0xdf,
                0x70, 0xe2, 0x6b, 0xf1,
            ]
        );
        assert_eq!(&encoding.bytes()[..4], b"ALGS");
        let hierarchy = document.hierarchy().unwrap();
        assert_eq!(
            hierarchy.selected_component_document().workspace(),
            document.control_workspace()
        );
        assert_eq!(hierarchy.flattening().node_provenance().len(), 21);
        assert_eq!(hierarchy.flattening().wire_provenance().len(), 25);
        let replay = replay_graph_authoring_session(
            encoding.bytes(),
            GraphAuthoringSessionReplayLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.document(), &document);
        assert_eq!(replay.encoding(), &encoding);
    }

    #[test]
    fn absent_hierarchy_corruption_and_limits_fail_closed() {
        let (document, encoding) = session_fixture(false);
        assert!(document.hierarchy().is_none());
        let replay = replay_graph_authoring_session(
            encoding.bytes(),
            GraphAuthoringSessionReplayLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.document(), &document);

        let mut bad_magic = encoding.bytes().to_vec();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            replay_graph_authoring_session(
                &bad_magic,
                GraphAuthoringSessionReplayLimits::interactive()
            ),
            Err(GraphAuthoringSessionError::InvalidMagic)
        );
        let mut bad_version = encoding.bytes().to_vec();
        bad_version[4..6].copy_from_slice(&0_u16.to_le_bytes());
        assert_eq!(
            replay_graph_authoring_session(
                &bad_version,
                GraphAuthoringSessionReplayLimits::interactive()
            ),
            Err(GraphAuthoringSessionError::UnsupportedVersion(0))
        );
        let hierarchy_tag = SESSION_FIXED_BYTES
            + document.control_encoding().bytes().len()
            + document.probe_encoding().bytes().len()
            + document.cached_job_encoding().bytes().len()
            - 1;
        let mut bad_tag = encoding.bytes().to_vec();
        bad_tag[hierarchy_tag] = 2;
        assert_eq!(
            replay_graph_authoring_session(
                &bad_tag,
                GraphAuthoringSessionReplayLimits::interactive()
            ),
            Err(GraphAuthoringSessionError::InvalidHierarchyTag(2))
        );
        let mut trailing = encoding.bytes().to_vec();
        trailing.push(0);
        assert_eq!(
            replay_graph_authoring_session(
                &trailing,
                GraphAuthoringSessionReplayLimits::interactive()
            ),
            Err(GraphAuthoringSessionError::TrailingBytes)
        );
        let mut tight = GraphAuthoringSessionReplayLimits::interactive();
        tight.session.maximum_session_bytes = encoding.bytes().len() - 1;
        assert_eq!(
            replay_graph_authoring_session(encoding.bytes(), tight),
            Err(GraphAuthoringSessionError::LimitExceeded(
                "admitted document byte length"
            ))
        );
    }

    #[test]
    fn selected_component_must_embed_the_exact_control_workspace() {
        let (document, _) = session_fixture(true);
        let hierarchy = document.hierarchy().unwrap();
        let input = GraphAuthoringHierarchyInput::new(
            Digest([0x55; 32]),
            hierarchy.document().clone(),
            hierarchy.source_map().clone(),
        );
        assert_eq!(
            GraphAuthoringSessionDocument::try_new(
                GraphAuthoringSessionLimits::interactive(),
                GraphHierarchySourceMapLimits::interactive(),
                document.control_workspace().clone(),
                document.probes().clone(),
                document.cached_job_workspace().clone(),
                Some(input),
            ),
            Err(GraphAuthoringSessionError::SelectedComponentMissing(
                Digest([0x55; 32])
            ))
        );
    }
}
