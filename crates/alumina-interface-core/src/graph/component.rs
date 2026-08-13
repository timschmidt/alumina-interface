//! Canonical reusable-component and front-panel authoring package.
//!
//! `ALGC` embeds one complete canonical [`super::GraphWorkspaceDocument`], then
//! adds a public connector pane and presentation-only front-panel bindings.
//! The embedded `ALGR` remains the only executable structural authority. A
//! component package does not admit opaque node semantics, allocate resources,
//! lower hierarchy, or grant deployment authority.

use core::fmt;
use std::collections::BTreeSet;
use std::str;

use alumina_protocol::Digest;
use alumina_storage::sha256;

use super::{
    GraphLimits, GraphNodeId, GraphPortId, GraphTypeId, GraphWorkspaceDocument,
    GraphWorkspaceError, GraphWorkspaceLimits, WireEndpoint, encode_graph_workspace,
    replay_graph_workspace,
};

/// Magic bytes at the beginning of each canonical graph component.
pub const GRAPH_COMPONENT_MAGIC: [u8; 4] = *b"ALGC";

/// Exact canonical graph-component format implemented by this source tree.
pub const GRAPH_COMPONENT_VERSION: u16 = 1;

const GRAPH_COMPONENT_FLAGS: u16 = 0;
const COMPONENT_LIMIT_FIELD_COUNT: usize = 5;
const STABLE_NAME_BYTES: usize = 64;
const EXHAUSTED_U32_CURSOR: u64 = u32::MAX as u64 + 1;

/// Caller-owned and embedded bounds for one reusable component package.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphComponentLimits {
    /// Maximum complete canonical `ALGC` byte length, including embedded
    /// `ALGW` and `ALGR` bytes.
    pub maximum_component_bytes: usize,
    /// Maximum public input terminals.
    pub maximum_inputs: usize,
    /// Maximum public output terminals.
    pub maximum_outputs: usize,
    /// Maximum front-panel controls and indicators.
    pub maximum_panel_items: usize,
    /// Maximum front-panel x/y/right/bottom logical coordinate.
    pub maximum_panel_coordinate: u32,
}

impl GraphComponentLimits {
    /// Bounded first component-editor policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_component_bytes: 24 * 1024 * 1024,
            maximum_inputs: 128,
            maximum_outputs: 128,
            maximum_panel_items: 256,
            maximum_panel_coordinate: 1_000_000,
        }
    }

    fn validate(self) -> Result<(), GraphComponentError> {
        if self.maximum_component_bytes == 0
            || self.maximum_inputs == 0
            || self.maximum_outputs == 0
            || self.maximum_panel_items == 0
            || self.maximum_panel_coordinate == 0
        {
            Err(GraphComponentError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphComponentLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Stable component-local public-input identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphComponentInputId(u32);

impl GraphComponentInputId {
    /// Construct an identity. Complete component validation rejects zero.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identity.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable component-local public-output identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphComponentOutputId(u32);

impl GraphComponentOutputId {
    /// Construct an identity. Complete component validation rejects zero.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identity.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable component-local front-panel item identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct GraphFrontPanelItemId(u32);

impl GraphFrontPanelItemId {
    /// Construct an identity. Complete component validation rejects zero.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the canonical integer identity.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One public component input mapped to an unowned internal graph input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphComponentInput {
    id: GraphComponentInputId,
    name: String,
    target: WireEndpoint,
}

impl GraphComponentInput {
    /// Construct a public input. The component validates identity, name,
    /// direction, uniqueness, and that the internal target has no wire owner.
    pub fn new(id: GraphComponentInputId, name: impl Into<String>, target: WireEndpoint) -> Self {
        Self {
            id,
            name: name.into(),
            target,
        }
    }

    /// Return the component-local terminal identity.
    pub const fn id(&self) -> GraphComponentInputId {
        self.id
    }

    /// Return the stable connector name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the mapped internal input endpoint.
    pub const fn target(&self) -> WireEndpoint {
        self.target
    }
}

/// One public component output mapped to an internal graph output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphComponentOutput {
    id: GraphComponentOutputId,
    name: String,
    source: WireEndpoint,
}

impl GraphComponentOutput {
    /// Construct a public output. The component validates identity, name,
    /// direction, and endpoint uniqueness.
    pub fn new(id: GraphComponentOutputId, name: impl Into<String>, source: WireEndpoint) -> Self {
        Self {
            id,
            name: name.into(),
            source,
        }
    }

    /// Return the component-local terminal identity.
    pub const fn id(&self) -> GraphComponentOutputId {
        self.id
    }

    /// Return the stable connector name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the mapped internal output endpoint.
    pub const fn source(&self) -> WireEndpoint {
        self.source
    }
}

/// Presentation-only integer rectangle for one front-panel item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphFrontPanelRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl GraphFrontPanelRect {
    /// Construct a logical-pixel rectangle. Complete component validation
    /// rejects negative origins, zero extents, and out-of-policy edges.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Return the presentation-only horizontal origin.
    pub const fn x(self) -> i32 {
        self.x
    }

    /// Return the presentation-only vertical origin.
    pub const fn y(self) -> i32 {
        self.y
    }

    /// Return the presentation-only width.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Return the presentation-only height.
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Exact authority to which a front-panel control or indicator is bound.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GraphFrontPanelBinding {
    /// A runtime control supplies one public component input.
    InputControl(GraphComponentInputId),
    /// An authoring control transactionally replaces one exact node parameter.
    ParameterControl {
        /// Internal graph node.
        node: GraphNodeId,
        /// Node-local parameter identity.
        parameter: u32,
    },
    /// An indicator observes one public component output.
    OutputIndicator(GraphComponentOutputId),
}

/// One stable front-panel control or indicator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphFrontPanelItem {
    id: GraphFrontPanelItemId,
    name: String,
    binding: GraphFrontPanelBinding,
    rect: GraphFrontPanelRect,
}

impl GraphFrontPanelItem {
    /// Construct an item. The complete component resolves its binding and
    /// validates its presentation rectangle.
    pub fn new(
        id: GraphFrontPanelItemId,
        name: impl Into<String>,
        binding: GraphFrontPanelBinding,
        rect: GraphFrontPanelRect,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            binding,
            rect,
        }
    }

    /// Return the stable panel-item identity.
    pub const fn id(&self) -> GraphFrontPanelItemId {
        self.id
    }

    /// Return the stable panel-item name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact binding authority.
    pub const fn binding(&self) -> GraphFrontPanelBinding {
        self.binding
    }

    /// Return presentation-only layout metadata.
    pub const fn rect(&self) -> GraphFrontPanelRect {
        self.rect
    }
}

/// Canonical reusable component authoring document.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphComponentDocument {
    limits: GraphComponentLimits,
    revision: u64,
    component_version: u32,
    name: String,
    next_input_id: u64,
    next_output_id: u64,
    next_panel_item_id: u64,
    workspace: GraphWorkspaceDocument,
    workspace_digest: Digest,
    inputs: Vec<GraphComponentInput>,
    outputs: Vec<GraphComponentOutput>,
    panel_items: Vec<GraphFrontPanelItem>,
}

impl GraphComponentDocument {
    /// Validate and canonicalize one complete component package.
    #[allow(
        clippy::too_many_arguments,
        reason = "component identity, monotonic allocators, embedded workspace, connector pane, and front panel remain explicit"
    )]
    pub fn try_new(
        limits: GraphComponentLimits,
        revision: u64,
        component_version: u32,
        name: impl Into<String>,
        next_input_id: u64,
        next_output_id: u64,
        next_panel_item_id: u64,
        workspace: GraphWorkspaceDocument,
        mut inputs: Vec<GraphComponentInput>,
        mut outputs: Vec<GraphComponentOutput>,
        mut panel_items: Vec<GraphFrontPanelItem>,
    ) -> Result<Self, GraphComponentError> {
        limits.validate()?;
        if component_version == 0 {
            return Err(GraphComponentError::ZeroComponentVersion);
        }
        let name = name.into();
        if !valid_stable_name(&name) {
            return Err(GraphComponentError::InvalidName("component"));
        }
        if inputs.len() > limits.maximum_inputs {
            return Err(GraphComponentError::LimitExceeded("public input count"));
        }
        if outputs.len() > limits.maximum_outputs {
            return Err(GraphComponentError::LimitExceeded("public output count"));
        }
        if panel_items.len() > limits.maximum_panel_items {
            return Err(GraphComponentError::LimitExceeded("front-panel item count"));
        }
        inputs.sort_unstable_by_key(GraphComponentInput::id);
        outputs.sort_unstable_by_key(GraphComponentOutput::id);
        panel_items.sort_unstable_by_key(GraphFrontPanelItem::id);
        validate_component_shape(&workspace, &inputs, &outputs, &panel_items, limits)?;
        validate_identity_cursor(
            next_input_id,
            inputs.iter().map(|input| u64::from(input.id.get())),
            "public input",
        )?;
        validate_identity_cursor(
            next_output_id,
            outputs.iter().map(|output| u64::from(output.id.get())),
            "public output",
        )?;
        validate_identity_cursor(
            next_panel_item_id,
            panel_items.iter().map(|item| u64::from(item.id.get())),
            "front-panel item",
        )?;
        let workspace_digest = encode_graph_workspace(&workspace)?.digest();
        Ok(Self {
            limits,
            revision,
            component_version,
            name,
            next_input_id,
            next_output_id,
            next_panel_item_id,
            workspace,
            workspace_digest,
            inputs,
            outputs,
            panel_items,
        })
    }

    /// Return embedded component limits.
    pub const fn limits(&self) -> GraphComponentLimits {
        self.limits
    }

    /// Return monotonic component-document revision metadata.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Return the declared reusable behavior version.
    pub const fn component_version(&self) -> u32 {
        self.component_version
    }

    /// Return the stable namespaced component name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the next public-input identity or exhausted sentinel.
    pub const fn next_input_id(&self) -> u64 {
        self.next_input_id
    }

    /// Return the next public-output identity or exhausted sentinel.
    pub const fn next_output_id(&self) -> u64 {
        self.next_output_id
    }

    /// Return the next front-panel-item identity or exhausted sentinel.
    pub const fn next_panel_item_id(&self) -> u64 {
        self.next_panel_item_id
    }

    /// Borrow the complete canonical authoring workspace.
    pub const fn workspace(&self) -> &GraphWorkspaceDocument {
        &self.workspace
    }

    /// Return the identity of the exact embedded `ALGW` bytes.
    pub const fn workspace_digest(&self) -> Digest {
        self.workspace_digest
    }

    /// Borrow public inputs in canonical identity order.
    pub fn inputs(&self) -> &[GraphComponentInput] {
        &self.inputs
    }

    /// Borrow public outputs in canonical identity order.
    pub fn outputs(&self) -> &[GraphComponentOutput] {
        &self.outputs
    }

    /// Borrow front-panel items in canonical identity order.
    pub fn panel_items(&self) -> &[GraphFrontPanelItem] {
        &self.panel_items
    }

    /// Resolve a public input by stable identity.
    pub fn input(&self, id: GraphComponentInputId) -> Option<&GraphComponentInput> {
        self.inputs
            .binary_search_by_key(&id, GraphComponentInput::id)
            .ok()
            .map(|index| &self.inputs[index])
    }

    /// Resolve a public output by stable identity.
    pub fn output(&self, id: GraphComponentOutputId) -> Option<&GraphComponentOutput> {
        self.outputs
            .binary_search_by_key(&id, GraphComponentOutput::id)
            .ok()
            .map(|index| &self.outputs[index])
    }

    /// Resolve a front-panel item by stable identity.
    pub fn panel_item(&self, id: GraphFrontPanelItemId) -> Option<&GraphFrontPanelItem> {
        self.panel_items
            .binary_search_by_key(&id, GraphFrontPanelItem::id)
            .ok()
            .map(|index| &self.panel_items[index])
    }

    /// Resolve the exact type supplied to one public input.
    pub fn input_value_type(&self, id: GraphComponentInputId) -> Option<GraphTypeId> {
        let target = self.input(id)?.target;
        input_port(&self.workspace, target).map(super::PortDefinition::value_type)
    }

    /// Resolve the exact type produced by one public output.
    pub fn output_value_type(&self, id: GraphComponentOutputId) -> Option<GraphTypeId> {
        let source = self.output(id)?.source;
        output_port(&self.workspace, source).map(super::PortDefinition::value_type)
    }

    /// Resolve the exact type controlled or observed by one panel item.
    pub fn panel_item_value_type(&self, id: GraphFrontPanelItemId) -> Option<GraphTypeId> {
        match self.panel_item(id)?.binding {
            GraphFrontPanelBinding::InputControl(input) => self.input_value_type(input),
            GraphFrontPanelBinding::ParameterControl { node, parameter } => self
                .workspace
                .graph()
                .node(node)?
                .parameters()
                .iter()
                .find(|candidate| candidate.id() == parameter)
                .map(|candidate| candidate.value().value_type()),
            GraphFrontPanelBinding::OutputIndicator(output) => self.output_value_type(output),
        }
    }

    /// Transactionally replace the embedded workspace and advance component
    /// revision. Connector and front-panel bindings must remain valid.
    pub fn replace_workspace(
        &mut self,
        workspace: GraphWorkspaceDocument,
    ) -> Result<(), GraphComponentError> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(GraphComponentError::RevisionOverflow)?;
        let candidate = Self::try_new(
            self.limits,
            revision,
            self.component_version,
            self.name.clone(),
            self.next_input_id,
            self.next_output_id,
            self.next_panel_item_id,
            workspace,
            self.inputs.clone(),
            self.outputs.clone(),
            self.panel_items.clone(),
        )?;
        *self = candidate;
        Ok(())
    }
}

/// Canonical component bytes paired with their SHA-256 content identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphComponentEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphComponentEncoding {
    /// Borrow complete canonical bytes.
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

/// Successfully replayed component and its verified canonical identity.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphComponentReplay {
    document: GraphComponentDocument,
    encoding: CanonicalGraphComponentEncoding,
}

impl GraphComponentReplay {
    /// Borrow the reconstructed component.
    pub const fn document(&self) -> &GraphComponentDocument {
        &self.document
    }

    /// Borrow byte-for-byte verified canonical encoding.
    pub const fn encoding(&self) -> &CanonicalGraphComponentEncoding {
        &self.encoding
    }

    /// Consume replay and return the component document.
    pub fn into_document(self) -> GraphComponentDocument {
        self.document
    }
}

/// Rejection at the component authoring or canonical replay boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphComponentError {
    /// A component policy contained zero.
    ZeroLimit,
    /// A count, byte length, embedded policy, or coordinate exceeded policy.
    LimitExceeded(&'static str),
    /// Declared reusable behavior version was zero.
    ZeroComponentVersion,
    /// Input did not begin with [`GRAPH_COMPONENT_MAGIC`].
    InvalidMagic,
    /// Component format version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved component flags were nonzero.
    UnsupportedFlags(u16),
    /// A fixed-width or length-delimited field ran past input.
    Truncated,
    /// A declared string was not UTF-8.
    InvalidUtf8(&'static str),
    /// A canonical field could not represent an in-memory value.
    IntegerOverflow(&'static str),
    /// Valid fields remained after the component.
    TrailingBytes,
    /// Decoding and canonical reconstruction changed at least one byte.
    NonCanonical,
    /// A stable component, terminal, or panel name was malformed.
    InvalidName(&'static str),
    /// A stable structural identity was zero.
    ZeroIdentifier(&'static str),
    /// A stable structural identity was duplicated.
    DuplicateIdentifier(&'static str),
    /// A stable connector or panel name was duplicated.
    DuplicateName(&'static str),
    /// A monotonic allocation cursor was invalid.
    InvalidIdentifierCursor(&'static str),
    /// Component-document revision could not advance.
    RevisionOverflow,
    /// A public input did not resolve to an internal input.
    UnknownInput(WireEndpoint),
    /// A public output did not resolve to an internal output.
    UnknownOutput(WireEndpoint),
    /// A public input target already had an internal wire owner.
    ConnectedPublicInput(WireEndpoint),
    /// Multiple public terminals mapped the same internal endpoint.
    DuplicateEndpoint(&'static str),
    /// A panel input control referenced no public input.
    UnknownComponentInput(GraphComponentInputId),
    /// A panel output indicator referenced no public output.
    UnknownComponentOutput(GraphComponentOutputId),
    /// A panel parameter control referenced no node.
    UnknownNode(GraphNodeId),
    /// A panel parameter control referenced no exact retained parameter.
    UnknownParameter {
        /// Internal node identity.
        node: GraphNodeId,
        /// Node-local parameter identity.
        parameter: u32,
    },
    /// Multiple panel items attempted to own one exact binding.
    DuplicatePanelBinding,
    /// A panel rectangle was negative, empty, overflowing, or out of bounds.
    InvalidPanelRect(GraphFrontPanelItemId),
    /// A canonical binding tag was unknown.
    UnsupportedBindingTag(u8),
    /// Embedded `ALGW` encoding/replay failed.
    Workspace(GraphWorkspaceError),
}

impl From<GraphWorkspaceError> for GraphComponentError {
    fn from(value: GraphWorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl fmt::Display for GraphComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph component policy contains zero"),
            Self::LimitExceeded(name) => {
                write!(formatter, "graph component {name} exceeds policy")
            }
            Self::ZeroComponentVersion => {
                formatter.write_str("graph component behavior version is zero")
            }
            Self::InvalidMagic => formatter.write_str("graph component magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "graph component version {version} is unsupported"
                )
            }
            Self::UnsupportedFlags(flags) => {
                write!(
                    formatter,
                    "graph component flags {flags:#06x} are unsupported"
                )
            }
            Self::Truncated => formatter.write_str("graph component is truncated"),
            Self::InvalidUtf8(name) => write!(formatter, "graph component {name} is not UTF-8"),
            Self::IntegerOverflow(name) => {
                write!(
                    formatter,
                    "graph component {name} exceeds its integer width"
                )
            }
            Self::TrailingBytes => formatter.write_str("graph component has trailing bytes"),
            Self::NonCanonical => formatter.write_str("graph component bytes are noncanonical"),
            Self::InvalidName(name) => write!(formatter, "graph component {name} name is invalid"),
            Self::ZeroIdentifier(kind) => {
                write!(formatter, "graph component {kind} identity is zero")
            }
            Self::DuplicateIdentifier(kind) => {
                write!(formatter, "graph component {kind} identity is duplicated")
            }
            Self::DuplicateName(kind) => {
                write!(formatter, "graph component {kind} name is duplicated")
            }
            Self::InvalidIdentifierCursor(kind) => {
                write!(formatter, "graph component next {kind} identity is invalid")
            }
            Self::RevisionOverflow => formatter.write_str("graph component revision is exhausted"),
            Self::UnknownInput(endpoint) => {
                write!(formatter, "graph component input {endpoint:?} is unknown")
            }
            Self::UnknownOutput(endpoint) => {
                write!(formatter, "graph component output {endpoint:?} is unknown")
            }
            Self::ConnectedPublicInput(endpoint) => write!(
                formatter,
                "graph component public input {endpoint:?} already has an internal wire"
            ),
            Self::DuplicateEndpoint(kind) => {
                write!(formatter, "graph component {kind} endpoint is duplicated")
            }
            Self::UnknownComponentInput(input) => {
                write!(
                    formatter,
                    "graph component panel input {input:?} is unknown"
                )
            }
            Self::UnknownComponentOutput(output) => {
                write!(
                    formatter,
                    "graph component panel output {output:?} is unknown"
                )
            }
            Self::UnknownNode(node) => {
                write!(formatter, "graph component panel node {node:?} is unknown")
            }
            Self::UnknownParameter { node, parameter } => write!(
                formatter,
                "graph component panel node {node:?} parameter {parameter} is unknown"
            ),
            Self::DuplicatePanelBinding => {
                formatter.write_str("graph component front-panel binding is duplicated")
            }
            Self::InvalidPanelRect(item) => {
                write!(
                    formatter,
                    "graph component panel item {item:?} rectangle is invalid"
                )
            }
            Self::UnsupportedBindingTag(tag) => {
                write!(
                    formatter,
                    "graph component panel binding tag {tag} is unsupported"
                )
            }
            Self::Workspace(error) => write!(formatter, "embedded workspace failed: {error}"),
        }
    }
}

impl std::error::Error for GraphComponentError {}

/// Encode one validated component and compute its content identity.
pub fn encode_graph_component(
    document: &GraphComponentDocument,
) -> Result<CanonicalGraphComponentEncoding, GraphComponentError> {
    let workspace = encode_graph_workspace(document.workspace())?;
    if workspace.digest() != document.workspace_digest {
        return Err(GraphComponentError::NonCanonical);
    }
    let mut encoder = Encoder::default();
    encoder.bytes(&GRAPH_COMPONENT_MAGIC);
    encoder.u16(GRAPH_COMPONENT_VERSION);
    encoder.u16(GRAPH_COMPONENT_FLAGS);
    encode_limits(&mut encoder, document.limits)?;
    encoder.u64(document.revision);
    encoder.u32(document.component_version);
    encoder.string(&document.name, "component name")?;
    encoder.u64(document.next_input_id);
    encoder.u64(document.next_output_id);
    encoder.u64(document.next_panel_item_id);
    encoder.u32(
        u32::try_from(workspace.bytes().len())
            .map_err(|_| GraphComponentError::IntegerOverflow("embedded workspace length"))?,
    );
    encoder.bytes(workspace.bytes());
    encoder.u32(
        u32::try_from(document.inputs.len())
            .map_err(|_| GraphComponentError::IntegerOverflow("public input count"))?,
    );
    for input in &document.inputs {
        encoder.u32(input.id.get());
        encoder.string(&input.name, "public input name")?;
        encode_endpoint(&mut encoder, input.target);
    }
    encoder.u32(
        u32::try_from(document.outputs.len())
            .map_err(|_| GraphComponentError::IntegerOverflow("public output count"))?,
    );
    for output in &document.outputs {
        encoder.u32(output.id.get());
        encoder.string(&output.name, "public output name")?;
        encode_endpoint(&mut encoder, output.source);
    }
    encoder.u32(
        u32::try_from(document.panel_items.len())
            .map_err(|_| GraphComponentError::IntegerOverflow("front-panel item count"))?,
    );
    for item in &document.panel_items {
        encoder.u32(item.id.get());
        encoder.string(&item.name, "front-panel item name")?;
        encode_binding(&mut encoder, item.binding);
        encoder.i32(item.rect.x);
        encoder.i32(item.rect.y);
        encoder.u32(item.rect.width);
        encoder.u32(item.rect.height);
    }
    if encoder.0.len() > document.limits.maximum_component_bytes {
        return Err(GraphComponentError::LimitExceeded("document byte length"));
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphComponentEncoding {
        bytes: encoder.0,
        digest,
    })
}

/// Decode, validate, canonically re-encode, and identify untrusted `ALGC` bytes.
pub fn replay_graph_component(
    bytes: &[u8],
    component_admission: GraphComponentLimits,
    workspace_admission: GraphWorkspaceLimits,
    graph_admission: GraphLimits,
) -> Result<GraphComponentReplay, GraphComponentError> {
    component_admission.validate()?;
    if bytes.len() > component_admission.maximum_component_bytes {
        return Err(GraphComponentError::LimitExceeded(
            "admitted document byte length",
        ));
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_COMPONENT_MAGIC.len())? != GRAPH_COMPONENT_MAGIC {
        return Err(GraphComponentError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_COMPONENT_VERSION {
        return Err(GraphComponentError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_COMPONENT_FLAGS {
        return Err(GraphComponentError::UnsupportedFlags(flags));
    }
    let limits = decode_limits(&mut decoder)?;
    if !limits_within(limits, component_admission) {
        return Err(GraphComponentError::LimitExceeded(
            "embedded admission limit",
        ));
    }
    limits.validate()?;
    if bytes.len() > limits.maximum_component_bytes {
        return Err(GraphComponentError::LimitExceeded(
            "embedded document byte length",
        ));
    }
    let revision = decoder.u64()?;
    let component_version = decoder.u32()?;
    let name = decoder.string("component name")?;
    let next_input_id = decoder.u64()?;
    let next_output_id = decoder.u64()?;
    let next_panel_item_id = decoder.u64()?;
    let workspace_length = usize::try_from(decoder.u32()?)
        .map_err(|_| GraphComponentError::IntegerOverflow("embedded workspace length"))?;
    let workspace_bytes = decoder.take(workspace_length)?;
    let workspace = replay_graph_workspace(workspace_bytes, workspace_admission, graph_admission)?
        .into_document();
    let input_count = decoder.count(limits.maximum_inputs, "public input count")?;
    let mut inputs = Vec::with_capacity(input_count);
    for _ in 0..input_count {
        inputs.push(GraphComponentInput::new(
            GraphComponentInputId::new(decoder.u32()?),
            decoder.string("public input name")?,
            decode_endpoint(&mut decoder)?,
        ));
    }
    let output_count = decoder.count(limits.maximum_outputs, "public output count")?;
    let mut outputs = Vec::with_capacity(output_count);
    for _ in 0..output_count {
        outputs.push(GraphComponentOutput::new(
            GraphComponentOutputId::new(decoder.u32()?),
            decoder.string("public output name")?,
            decode_endpoint(&mut decoder)?,
        ));
    }
    let item_count = decoder.count(limits.maximum_panel_items, "front-panel item count")?;
    let mut panel_items = Vec::with_capacity(item_count);
    for _ in 0..item_count {
        panel_items.push(GraphFrontPanelItem::new(
            GraphFrontPanelItemId::new(decoder.u32()?),
            decoder.string("front-panel item name")?,
            decode_binding(&mut decoder)?,
            GraphFrontPanelRect::new(
                decoder.i32()?,
                decoder.i32()?,
                decoder.u32()?,
                decoder.u32()?,
            ),
        ));
    }
    if !decoder.is_empty() {
        return Err(GraphComponentError::TrailingBytes);
    }
    let document = GraphComponentDocument::try_new(
        limits,
        revision,
        component_version,
        name,
        next_input_id,
        next_output_id,
        next_panel_item_id,
        workspace,
        inputs,
        outputs,
        panel_items,
    )?;
    let encoding = encode_graph_component(&document)?;
    if encoding.bytes() != bytes {
        return Err(GraphComponentError::NonCanonical);
    }
    Ok(GraphComponentReplay { document, encoding })
}

fn validate_component_shape(
    workspace: &GraphWorkspaceDocument,
    inputs: &[GraphComponentInput],
    outputs: &[GraphComponentOutput],
    panel_items: &[GraphFrontPanelItem],
    limits: GraphComponentLimits,
) -> Result<(), GraphComponentError> {
    let mut terminal_names = BTreeSet::new();
    let mut input_endpoints = BTreeSet::new();
    let mut prior_input = None;
    for input in inputs {
        if input.id.get() == 0 {
            return Err(GraphComponentError::ZeroIdentifier("public input"));
        }
        if prior_input == Some(input.id) {
            return Err(GraphComponentError::DuplicateIdentifier("public input"));
        }
        prior_input = Some(input.id);
        validate_terminal_name(&input.name, &mut terminal_names)?;
        if input_port(workspace, input.target).is_none() {
            return Err(GraphComponentError::UnknownInput(input.target));
        }
        if workspace
            .graph()
            .wires()
            .iter()
            .any(|wire| wire.target() == input.target)
        {
            return Err(GraphComponentError::ConnectedPublicInput(input.target));
        }
        if !input_endpoints.insert(input.target) {
            return Err(GraphComponentError::DuplicateEndpoint("public input"));
        }
    }

    let mut output_endpoints = BTreeSet::new();
    let mut prior_output = None;
    for output in outputs {
        if output.id.get() == 0 {
            return Err(GraphComponentError::ZeroIdentifier("public output"));
        }
        if prior_output == Some(output.id) {
            return Err(GraphComponentError::DuplicateIdentifier("public output"));
        }
        prior_output = Some(output.id);
        validate_terminal_name(&output.name, &mut terminal_names)?;
        if output_port(workspace, output.source).is_none() {
            return Err(GraphComponentError::UnknownOutput(output.source));
        }
        if !output_endpoints.insert(output.source) {
            return Err(GraphComponentError::DuplicateEndpoint("public output"));
        }
    }

    let mut item_names = BTreeSet::new();
    let mut bindings = BTreeSet::new();
    let mut prior_item = None;
    for item in panel_items {
        if item.id.get() == 0 {
            return Err(GraphComponentError::ZeroIdentifier("front-panel item"));
        }
        if prior_item == Some(item.id) {
            return Err(GraphComponentError::DuplicateIdentifier("front-panel item"));
        }
        prior_item = Some(item.id);
        if !valid_stable_name(&item.name) {
            return Err(GraphComponentError::InvalidName("front-panel item"));
        }
        if !item_names.insert(item.name.as_str()) {
            return Err(GraphComponentError::DuplicateName("front-panel item"));
        }
        if !bindings.insert(item.binding) {
            return Err(GraphComponentError::DuplicatePanelBinding);
        }
        validate_binding(workspace, inputs, outputs, item.binding)?;
        validate_panel_rect(item.id, item.rect, limits)?;
    }
    Ok(())
}

fn validate_terminal_name<'a>(
    name: &'a str,
    names: &mut BTreeSet<&'a str>,
) -> Result<(), GraphComponentError> {
    if !valid_stable_name(name) {
        return Err(GraphComponentError::InvalidName("terminal"));
    }
    if !names.insert(name) {
        return Err(GraphComponentError::DuplicateName("terminal"));
    }
    Ok(())
}

fn validate_binding(
    workspace: &GraphWorkspaceDocument,
    inputs: &[GraphComponentInput],
    outputs: &[GraphComponentOutput],
    binding: GraphFrontPanelBinding,
) -> Result<(), GraphComponentError> {
    match binding {
        GraphFrontPanelBinding::InputControl(input) => {
            if inputs
                .binary_search_by_key(&input, GraphComponentInput::id)
                .is_err()
            {
                return Err(GraphComponentError::UnknownComponentInput(input));
            }
        }
        GraphFrontPanelBinding::ParameterControl { node, parameter } => {
            let node_definition = workspace
                .graph()
                .node(node)
                .ok_or(GraphComponentError::UnknownNode(node))?;
            if parameter == 0
                || !node_definition
                    .parameters()
                    .iter()
                    .any(|candidate| candidate.id() == parameter)
            {
                return Err(GraphComponentError::UnknownParameter { node, parameter });
            }
        }
        GraphFrontPanelBinding::OutputIndicator(output) => {
            if outputs
                .binary_search_by_key(&output, GraphComponentOutput::id)
                .is_err()
            {
                return Err(GraphComponentError::UnknownComponentOutput(output));
            }
        }
    }
    Ok(())
}

fn validate_panel_rect(
    item: GraphFrontPanelItemId,
    rect: GraphFrontPanelRect,
    limits: GraphComponentLimits,
) -> Result<(), GraphComponentError> {
    let Ok(x) = u32::try_from(rect.x) else {
        return Err(GraphComponentError::InvalidPanelRect(item));
    };
    let Ok(y) = u32::try_from(rect.y) else {
        return Err(GraphComponentError::InvalidPanelRect(item));
    };
    let Some(right) = x.checked_add(rect.width) else {
        return Err(GraphComponentError::InvalidPanelRect(item));
    };
    let Some(bottom) = y.checked_add(rect.height) else {
        return Err(GraphComponentError::InvalidPanelRect(item));
    };
    if rect.width == 0
        || rect.height == 0
        || right > limits.maximum_panel_coordinate
        || bottom > limits.maximum_panel_coordinate
    {
        return Err(GraphComponentError::InvalidPanelRect(item));
    }
    Ok(())
}

fn input_port(
    workspace: &GraphWorkspaceDocument,
    endpoint: WireEndpoint,
) -> Option<&super::PortDefinition> {
    workspace
        .graph()
        .node(endpoint.node)?
        .inputs()
        .iter()
        .find(|port| port.id() == endpoint.port)
}

fn output_port(
    workspace: &GraphWorkspaceDocument,
    endpoint: WireEndpoint,
) -> Option<&super::PortDefinition> {
    workspace
        .graph()
        .node(endpoint.node)?
        .outputs()
        .iter()
        .find(|port| port.id() == endpoint.port)
}

fn validate_identity_cursor(
    cursor: u64,
    retained: impl Iterator<Item = u64>,
    kind: &'static str,
) -> Result<(), GraphComponentError> {
    let maximum = retained.max().unwrap_or(0);
    let minimum = maximum
        .checked_add(1)
        .ok_or(GraphComponentError::InvalidIdentifierCursor(kind))?;
    if cursor < minimum || cursor == 0 || cursor > EXHAUSTED_U32_CURSOR {
        Err(GraphComponentError::InvalidIdentifierCursor(kind))
    } else {
        Ok(())
    }
}

fn valid_stable_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= STABLE_NAME_BYTES
        && bytes[0].is_ascii_alphabetic()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn encode_endpoint(encoder: &mut Encoder, endpoint: WireEndpoint) {
    encoder.u32(endpoint.node.get());
    encoder.u32(endpoint.port.get());
}

fn decode_endpoint(decoder: &mut Decoder<'_>) -> Result<WireEndpoint, GraphComponentError> {
    Ok(WireEndpoint {
        node: GraphNodeId::new(decoder.u32()?),
        port: GraphPortId::new(decoder.u32()?),
    })
}

fn encode_binding(encoder: &mut Encoder, binding: GraphFrontPanelBinding) {
    match binding {
        GraphFrontPanelBinding::InputControl(input) => {
            encoder.u8(1);
            encoder.u32(input.get());
        }
        GraphFrontPanelBinding::ParameterControl { node, parameter } => {
            encoder.u8(2);
            encoder.u32(node.get());
            encoder.u32(parameter);
        }
        GraphFrontPanelBinding::OutputIndicator(output) => {
            encoder.u8(3);
            encoder.u32(output.get());
        }
    }
}

fn decode_binding(
    decoder: &mut Decoder<'_>,
) -> Result<GraphFrontPanelBinding, GraphComponentError> {
    match decoder.u8()? {
        1 => Ok(GraphFrontPanelBinding::InputControl(
            GraphComponentInputId::new(decoder.u32()?),
        )),
        2 => Ok(GraphFrontPanelBinding::ParameterControl {
            node: GraphNodeId::new(decoder.u32()?),
            parameter: decoder.u32()?,
        }),
        3 => Ok(GraphFrontPanelBinding::OutputIndicator(
            GraphComponentOutputId::new(decoder.u32()?),
        )),
        tag => Err(GraphComponentError::UnsupportedBindingTag(tag)),
    }
}

fn encode_limits(
    encoder: &mut Encoder,
    limits: GraphComponentLimits,
) -> Result<(), GraphComponentError> {
    let values = [
        limits.maximum_component_bytes,
        limits.maximum_inputs,
        limits.maximum_outputs,
        limits.maximum_panel_items,
        usize::try_from(limits.maximum_panel_coordinate)
            .map_err(|_| GraphComponentError::IntegerOverflow("panel coordinate limit"))?,
    ];
    for value in values {
        encoder.u64(
            u64::try_from(value)
                .map_err(|_| GraphComponentError::IntegerOverflow("limit value"))?,
        );
    }
    Ok(())
}

fn decode_limits(decoder: &mut Decoder<'_>) -> Result<GraphComponentLimits, GraphComponentError> {
    let mut values = [0_usize; COMPONENT_LIMIT_FIELD_COUNT];
    for value in &mut values {
        *value = usize::try_from(decoder.u64()?)
            .map_err(|_| GraphComponentError::IntegerOverflow("limit value"))?;
    }
    Ok(GraphComponentLimits {
        maximum_component_bytes: values[0],
        maximum_inputs: values[1],
        maximum_outputs: values[2],
        maximum_panel_items: values[3],
        maximum_panel_coordinate: u32::try_from(values[4])
            .map_err(|_| GraphComponentError::IntegerOverflow("panel coordinate limit"))?,
    })
}

const fn limits_within(embedded: GraphComponentLimits, admission: GraphComponentLimits) -> bool {
    embedded.maximum_component_bytes <= admission.maximum_component_bytes
        && embedded.maximum_inputs <= admission.maximum_inputs
        && embedded.maximum_outputs <= admission.maximum_outputs
        && embedded.maximum_panel_items <= admission.maximum_panel_items
        && embedded.maximum_panel_coordinate <= admission.maximum_panel_coordinate
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

    fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn string(&mut self, value: &str, name: &'static str) -> Result<(), GraphComponentError> {
        self.u16(
            u16::try_from(value.len()).map_err(|_| GraphComponentError::IntegerOverflow(name))?,
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

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphComponentError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(GraphComponentError::Truncated)?;
        let result = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GraphComponentError::Truncated)?;
        self.cursor = end;
        Ok(result)
    }

    fn u8(&mut self) -> Result<u8, GraphComponentError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, GraphComponentError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| GraphComponentError::Truncated)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, GraphComponentError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| GraphComponentError::Truncated)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn i32(&mut self) -> Result<i32, GraphComponentError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| GraphComponentError::Truncated)?;
        Ok(i32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, GraphComponentError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| GraphComponentError::Truncated)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn string(&mut self, name: &'static str) -> Result<String, GraphComponentError> {
        let length = usize::from(self.u16()?);
        if length > STABLE_NAME_BYTES {
            return Err(GraphComponentError::LimitExceeded(name));
        }
        str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| GraphComponentError::InvalidUtf8(name))
    }

    fn count(&mut self, maximum: usize, name: &'static str) -> Result<usize, GraphComponentError> {
        let count =
            usize::try_from(self.u32()?).map_err(|_| GraphComponentError::IntegerOverflow(name))?;
        if count > maximum {
            Err(GraphComponentError::LimitExceeded(name))
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
        GraphNodePlacement, GraphWireId, RepresentativeControlSignal,
        compile_representative_exact_control_graph,
    };

    fn endpoint(node: u32, port: u32) -> WireEndpoint {
        WireEndpoint {
            node: GraphNodeId::new(node),
            port: GraphPortId::new(port),
        }
    }

    fn workspace() -> GraphWorkspaceDocument {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let graph = fixture.document().clone();
        let placements = graph
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let index = i32::try_from(index).unwrap();
                GraphNodePlacement::new(node.id(), index * 40, index * 20)
            })
            .collect();
        GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            20,
            23,
            graph,
            placements,
        )
        .unwrap()
    }

    fn output() -> GraphComponentOutput {
        GraphComponentOutput::new(
            GraphComponentOutputId::new(1),
            "permitted_output",
            RepresentativeControlSignal::PermittedOutput.endpoint(),
        )
    }

    fn parameter_item() -> GraphFrontPanelItem {
        GraphFrontPanelItem::new(
            GraphFrontPanelItemId::new(1),
            "proportional_gain",
            GraphFrontPanelBinding::ParameterControl {
                node: GraphNodeId::new(8),
                parameter: 1,
            },
            GraphFrontPanelRect::new(20, 20, 180, 48),
        )
    }

    fn output_item() -> GraphFrontPanelItem {
        GraphFrontPanelItem::new(
            GraphFrontPanelItemId::new(2),
            "permitted_output",
            GraphFrontPanelBinding::OutputIndicator(GraphComponentOutputId::new(1)),
            GraphFrontPanelRect::new(220, 20, 180, 48),
        )
    }

    fn component() -> GraphComponentDocument {
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.reference_pid",
            1,
            2,
            3,
            workspace(),
            Vec::new(),
            vec![output()],
            vec![parameter_item(), output_item()],
        )
        .unwrap()
    }

    #[test]
    fn canonical_component_round_trips_with_exact_binding_types() {
        let component = component();
        assert_eq!(
            component.output_value_type(GraphComponentOutputId::new(1)),
            Some(GraphTypeId::new(5))
        );
        assert_eq!(
            component.panel_item_value_type(GraphFrontPanelItemId::new(1)),
            Some(GraphTypeId::new(3))
        );
        assert_eq!(
            component.panel_item_value_type(GraphFrontPanelItemId::new(2)),
            Some(GraphTypeId::new(5))
        );
        let encoding = encode_graph_component(&component).unwrap();
        let replay = replay_graph_component(
            encoding.bytes(),
            GraphComponentLimits::interactive(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.document(), &component);
        assert_eq!(replay.encoding(), &encoding);
        assert_eq!(replay.encoding().bytes(), encoding.bytes());
    }

    #[test]
    fn public_input_must_be_unowned_and_directionally_exact() {
        let connected = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.input_test",
            2,
            1,
            1,
            workspace(),
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "setpoint_samples",
                endpoint(4, 1),
            )],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            connected,
            Err(GraphComponentError::ConnectedPublicInput(endpoint(4, 1)))
        );

        let mut disconnected = workspace();
        disconnected.disconnect(GraphWireId::new(1)).unwrap();
        let valid = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.input_test",
            2,
            1,
            2,
            disconnected,
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "setpoint_samples",
                endpoint(4, 1),
            )],
            Vec::new(),
            vec![GraphFrontPanelItem::new(
                GraphFrontPanelItemId::new(1),
                "setpoint_samples",
                GraphFrontPanelBinding::InputControl(GraphComponentInputId::new(1)),
                GraphFrontPanelRect::new(0, 0, 120, 40),
            )],
        )
        .unwrap();
        assert_eq!(
            valid.input_value_type(GraphComponentInputId::new(1)),
            Some(GraphTypeId::new(4))
        );

        let wrong_direction = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.input_test",
            2,
            1,
            1,
            workspace(),
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "not_an_input",
                endpoint(1, 1),
            )],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            wrong_direction,
            Err(GraphComponentError::UnknownInput(endpoint(1, 1)))
        );
    }

    #[test]
    fn workspace_replacement_is_transactional_across_bindings() {
        let mut component = component();
        let old_digest = component.workspace_digest();
        let mut moved = component.workspace().clone();
        moved.move_node(GraphNodeId::new(8), 99, 101).unwrap();
        component.replace_workspace(moved).unwrap();
        assert_eq!(component.revision(), 2);
        assert_ne!(component.workspace_digest(), old_digest);

        let retained = component.clone();
        let mut missing_binding = component.workspace().clone();
        missing_binding.delete_node(GraphNodeId::new(18)).unwrap();
        assert_eq!(
            component.replace_workspace(missing_binding),
            Err(GraphComponentError::UnknownOutput(endpoint(18, 3)))
        );
        assert_eq!(component, retained);
    }

    #[test]
    fn panel_and_cursor_invariants_reject_ambiguous_authority() {
        let duplicate_binding = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.reference_pid",
            1,
            2,
            4,
            workspace(),
            Vec::new(),
            vec![output()],
            vec![
                parameter_item(),
                GraphFrontPanelItem::new(
                    GraphFrontPanelItemId::new(3),
                    "proportional_gain_copy",
                    GraphFrontPanelBinding::ParameterControl {
                        node: GraphNodeId::new(8),
                        parameter: 1,
                    },
                    GraphFrontPanelRect::new(20, 80, 180, 48),
                ),
            ],
        );
        assert_eq!(
            duplicate_binding,
            Err(GraphComponentError::DuplicatePanelBinding)
        );

        let invalid_rect = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.reference_pid",
            1,
            2,
            2,
            workspace(),
            Vec::new(),
            vec![output()],
            vec![GraphFrontPanelItem::new(
                GraphFrontPanelItemId::new(1),
                "permitted_output",
                GraphFrontPanelBinding::OutputIndicator(GraphComponentOutputId::new(1)),
                GraphFrontPanelRect::new(-1, 0, 10, 10),
            )],
        );
        assert_eq!(
            invalid_rect,
            Err(GraphComponentError::InvalidPanelRect(
                GraphFrontPanelItemId::new(1)
            ))
        );

        let invalid_cursor = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.reference_pid",
            1,
            1,
            3,
            workspace(),
            Vec::new(),
            vec![output()],
            vec![parameter_item(), output_item()],
        );
        assert_eq!(
            invalid_cursor,
            Err(GraphComponentError::InvalidIdentifierCursor(
                "public output"
            ))
        );
    }

    #[test]
    fn replay_rejects_outer_corruption_and_caller_limit_escalation() {
        let encoding = encode_graph_component(&component()).unwrap();
        let mut bad_magic = encoding.bytes().to_vec();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            replay_graph_component(
                &bad_magic,
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            ),
            Err(GraphComponentError::InvalidMagic)
        );

        let mut trailing = encoding.bytes().to_vec();
        trailing.push(0);
        assert_eq!(
            replay_graph_component(
                &trailing,
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            ),
            Err(GraphComponentError::TrailingBytes)
        );

        let mut restrictive = GraphComponentLimits::interactive();
        restrictive.maximum_outputs = 1;
        restrictive.maximum_panel_items = 2;
        restrictive.maximum_panel_coordinate = 400;
        assert_eq!(
            replay_graph_component(
                encoding.bytes(),
                restrictive,
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            ),
            Err(GraphComponentError::LimitExceeded(
                "embedded admission limit"
            ))
        );
    }

    #[test]
    fn replay_rejects_an_alternative_valid_record_order() {
        let document = GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "control.order_test",
            1,
            3,
            1,
            workspace(),
            Vec::new(),
            vec![
                GraphComponentOutput::new(
                    GraphComponentOutputId::new(1),
                    "error",
                    RepresentativeControlSignal::Error.endpoint(),
                ),
                GraphComponentOutput::new(
                    GraphComponentOutputId::new(2),
                    "permitted_output",
                    RepresentativeControlSignal::PermittedOutput.endpoint(),
                ),
            ],
            Vec::new(),
        )
        .unwrap();
        let encoding = encode_graph_component(&document).unwrap();
        let mut decoder = Decoder::new(encoding.bytes());
        decoder.take(GRAPH_COMPONENT_MAGIC.len()).unwrap();
        decoder.u16().unwrap();
        decoder.u16().unwrap();
        for _ in 0..COMPONENT_LIMIT_FIELD_COUNT {
            decoder.u64().unwrap();
        }
        decoder.u64().unwrap();
        decoder.u32().unwrap();
        decoder.string("component name").unwrap();
        decoder.u64().unwrap();
        decoder.u64().unwrap();
        decoder.u64().unwrap();
        let workspace_length = usize::try_from(decoder.u32().unwrap()).unwrap();
        decoder.take(workspace_length).unwrap();
        assert_eq!(decoder.u32().unwrap(), 0);
        assert_eq!(decoder.u32().unwrap(), 2);
        let first_start = decoder.cursor;
        decoder.u32().unwrap();
        decoder.string("public output name").unwrap();
        decode_endpoint(&mut decoder).unwrap();
        let first_end = decoder.cursor;
        decoder.u32().unwrap();
        decoder.string("public output name").unwrap();
        decode_endpoint(&mut decoder).unwrap();
        let second_end = decoder.cursor;

        let mut reordered = Vec::with_capacity(encoding.bytes().len());
        reordered.extend_from_slice(&encoding.bytes()[..first_start]);
        reordered.extend_from_slice(&encoding.bytes()[first_end..second_end]);
        reordered.extend_from_slice(&encoding.bytes()[first_start..first_end]);
        reordered.extend_from_slice(&encoding.bytes()[second_end..]);
        assert_eq!(reordered.len(), encoding.bytes().len());
        assert_eq!(
            replay_graph_component(
                &reordered,
                GraphComponentLimits::interactive(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
            ),
            Err(GraphComponentError::NonCanonical)
        );
    }
}
