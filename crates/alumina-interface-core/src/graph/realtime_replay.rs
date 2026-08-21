//! Bounded deterministic replay of lowered Realtime graph bytes in firmware actors.
//!
//! This is an offline verification boundary. It invokes the same portable
//! fixed-memory service/realtime actors used by firmware, but it does not
//! authenticate a device, publish storage, acquire a resource, or grant any
//! deployment, start, safety, or physical-I/O authority.

use core::fmt;

use alumina_board::{GraphOpcodeDescriptor, GraphResourceDescriptor, ResourceId};
use alumina_capability::{decode_resource_id, encode_resource_id};
use alumina_graph_ir::{MAX_GRAPH_IR_NODES, graph_ir_content_digest};
use alumina_protocol::{DeviceCycle, DeviceId, Digest};
use alumina_runtime::graph::{
    FixedGraphRealtimeActor, FixedGraphServiceActor, GraphExecutionError, GraphLiveError,
    GraphReleaseReport, GraphRunIdentity, GraphRuntimeAuthority, GraphRuntimeLimits,
    ReloadableGraphBridge,
};
use alumina_storage::sha256;

use super::{GraphDeploymentReport, GraphDeploymentTarget};

/// Domain tag for the deterministic replay-evidence digest.
pub const GRAPH_DEPLOYMENT_REPLAY_MAGIC: [u8; 8] = *b"ALGRREP1";

/// Hard maximum release attempts admitted by the offline replay boundary.
pub const MAX_GRAPH_DEPLOYMENT_REPLAY_RELEASES: usize = 4_096;
/// Hard maximum distinct resource states supplied for one release.
pub const MAX_GRAPH_DEPLOYMENT_REPLAY_INPUTS_PER_RELEASE: usize = 256;
/// Hard maximum provider calls retained for one release.
pub const MAX_GRAPH_DEPLOYMENT_REPLAY_READS_PER_RELEASE: usize = 2 * MAX_GRAPH_IR_NODES;
/// Hard maximum canonical `ALGRREP1` bytes accepted before hashing or decoding.
pub const MAX_GRAPH_DEPLOYMENT_REPLAY_EVIDENCE_BYTES: usize = 228
    + MAX_GRAPH_DEPLOYMENT_REPLAY_RELEASES
        * (13
            + MAX_GRAPH_DEPLOYMENT_REPLAY_INPUTS_PER_RELEASE * 5
            + 12
            + MAX_GRAPH_DEPLOYMENT_REPLAY_READS_PER_RELEASE * 4
            + 32);

const REPLAY_SERVICE_STATE_BYTES: usize = 2_048;
const REPLAY_REALTIME_STATE_BYTES: usize = 2_048;
const REPLAY_SERVICE_CHANNEL_BYTES: usize = 4_096;
const REPLAY_REALTIME_CHANNEL_BYTES: usize = 4_096;
const REPLAY_BRIDGE_BYTES: usize = 4_096;

type ReplayServiceActor<'a> = FixedGraphServiceActor<
    'a,
    REPLAY_SERVICE_STATE_BYTES,
    REPLAY_SERVICE_CHANNEL_BYTES,
    REPLAY_BRIDGE_BYTES,
>;
type ReplayRealtimeActor<'a> = FixedGraphRealtimeActor<
    'a,
    REPLAY_REALTIME_STATE_BYTES,
    REPLAY_REALTIME_CHANNEL_BYTES,
    REPLAY_BRIDGE_BYTES,
>;

/// Bounded host/WASM policy for one firmware-actor replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphDeploymentReplayLimits {
    /// Maximum requested release attempts.
    pub maximum_releases: usize,
    /// Maximum distinct resource states supplied to one release.
    pub maximum_inputs_per_release: usize,
    /// Maximum resource-provider calls retained from one release.
    pub maximum_reads_per_release: usize,
}

impl GraphDeploymentReplayLimits {
    /// Interactive policy below the fixed absolute replay ceilings.
    pub const fn interactive() -> Self {
        Self {
            maximum_releases: 1_024,
            maximum_inputs_per_release: 128,
            maximum_reads_per_release: MAX_GRAPH_DEPLOYMENT_REPLAY_READS_PER_RELEASE,
        }
    }

    fn validate(self) -> Result<(), GraphDeploymentReplayError> {
        if self.maximum_releases == 0
            || self.maximum_inputs_per_release == 0
            || self.maximum_reads_per_release == 0
            || self.maximum_releases > MAX_GRAPH_DEPLOYMENT_REPLAY_RELEASES
            || self.maximum_inputs_per_release > MAX_GRAPH_DEPLOYMENT_REPLAY_INPUTS_PER_RELEASE
            || self.maximum_reads_per_release > MAX_GRAPH_DEPLOYMENT_REPLAY_READS_PER_RELEASE
        {
            Err(GraphDeploymentReplayError::InvalidLimits)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphDeploymentReplayLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// One typed resource state supplied to an offline release.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphDeploymentResourceSample {
    resource: ResourceId,
    value: Option<bool>,
}

impl GraphDeploymentResourceSample {
    /// Construct one available or explicitly unavailable Boolean state.
    pub const fn new(resource: ResourceId, value: Option<bool>) -> Self {
        Self { resource, value }
    }

    /// Return the exact typed physical-resource identity.
    pub const fn resource(self) -> ResourceId {
        self.resource
    }

    /// Return the modeled stable state, or `None` for unavailable evidence.
    pub const fn value(self) -> Option<bool> {
        self.value
    }
}

/// Caller-owned state for one exact Realtime release attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDeploymentReplayInput {
    cycle: DeviceCycle,
    safety_authorized: bool,
    resources: Vec<GraphDeploymentResourceSample>,
}

impl GraphDeploymentReplayInput {
    /// Construct one release input. Complete validation occurs before replay.
    pub fn new(
        cycle: DeviceCycle,
        safety_authorized: bool,
        resources: Vec<GraphDeploymentResourceSample>,
    ) -> Self {
        Self {
            cycle,
            safety_authorized,
            resources,
        }
    }

    /// Return the requested exact device cycle.
    pub const fn cycle(&self) -> DeviceCycle {
        self.cycle
    }

    /// Return the modeled non-bypassable safety admission at this boundary.
    pub const fn safety_authorized(&self) -> bool {
        self.safety_authorized
    }

    /// Borrow the caller-supplied distinct resource states.
    pub fn resources(&self) -> &[GraphDeploymentResourceSample] {
        &self.resources
    }
}

/// Completed release facts or one exact terminal firmware execution fault.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphDeploymentReplayReleaseOutcome {
    /// All due nodes completed.
    Completed(GraphReleaseReport),
    /// This attempt latched the actor bridge's first-cause fault.
    Faulted(GraphExecutionError),
}

/// One attempted release with actual ordered provider-call evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDeploymentReplayRelease {
    cycle: DeviceCycle,
    reads: Vec<ResourceId>,
    outcome: GraphDeploymentReplayReleaseOutcome,
}

impl GraphDeploymentReplayRelease {
    /// Return the requested release cycle.
    pub const fn cycle(&self) -> DeviceCycle {
        self.cycle
    }

    /// Borrow resource IDs in actual firmware-provider call order.
    pub fn reads(&self) -> &[ResourceId] {
        &self.reads
    }

    /// Return the completed report or terminal first-cause fault.
    pub const fn outcome(&self) -> GraphDeploymentReplayReleaseOutcome {
        self.outcome
    }
}

/// Immutable canonical input/output transcript for one firmware-actor replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphDeploymentReplayEvidence1 {
    encoded: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphDeploymentReplayEvidence1 {
    /// Borrow the complete canonical `ALGRREP1` transcript bytes.
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    /// Return the SHA-256 identity of the complete transcript.
    pub const fn digest(&self) -> Digest {
        self.digest
    }
}

/// Deterministic offline result produced by the actual portable firmware actors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDeploymentReplay {
    run: GraphRunIdentity,
    implementation_digest: Digest,
    evidence: CanonicalGraphDeploymentReplayEvidence1,
    requested_releases: usize,
    releases: Vec<GraphDeploymentReplayRelease>,
}

impl GraphDeploymentReplay {
    /// Return the exact boot-local execution identity modeled by this replay.
    pub const fn run(&self) -> GraphRunIdentity {
        self.run
    }

    /// Return the exact fixed implementation-registry identity.
    pub const fn implementation_digest(&self) -> Digest {
        self.implementation_digest
    }

    /// Return the SHA-256 domain-separated input/output replay identity.
    pub const fn evidence_digest(&self) -> Digest {
        self.evidence.digest()
    }

    /// Borrow the complete canonical input/output replay evidence.
    pub const fn evidence(&self) -> &CanonicalGraphDeploymentReplayEvidence1 {
        &self.evidence
    }

    /// Return the number of release inputs requested by the caller.
    pub const fn requested_releases(&self) -> usize {
        self.requested_releases
    }

    /// Borrow attempted releases in exact cycle order.
    pub fn releases(&self) -> &[GraphDeploymentReplayRelease] {
        &self.releases
    }

    /// Return true only when every requested release completed without a fault.
    pub fn complete(&self) -> bool {
        self.releases.len() == self.requested_releases
            && self.releases.iter().all(|release| {
                matches!(
                    release.outcome,
                    GraphDeploymentReplayReleaseOutcome::Completed(_)
                )
            })
    }

    /// Return the terminal fault when replay stopped at its first cause.
    pub fn terminal_fault(&self) -> Option<GraphExecutionError> {
        self.releases
            .last()
            .and_then(|release| match release.outcome {
                GraphDeploymentReplayReleaseOutcome::Completed(_) => None,
                GraphDeploymentReplayReleaseOutcome::Faulted(error) => Some(error),
            })
    }
}

/// Rejection before an exact offline replay result can be formed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphDeploymentReplayError {
    /// One caller limit was zero or above its fixed ceiling.
    InvalidLimits,
    /// The package contains Service work beyond tick-zero-free replay scope.
    ServiceDomainPresent,
    /// The package has no Realtime node to release.
    RealtimeDomainAbsent,
    /// No release input was supplied.
    Empty,
    /// A bounded input or output collection exceeded policy.
    LimitExceeded {
        /// Stable rejected collection name.
        aspect: &'static str,
        /// Zero-based release index, when applicable.
        release: Option<usize>,
    },
    /// Two states in one release named the same typed resource.
    DuplicateResource {
        /// Zero-based release index.
        release: usize,
        /// Duplicate resource identity.
        resource: ResourceId,
    },
    /// Requested cycles did not strictly increase.
    NonmonotonicCycle {
        /// Zero-based release index.
        release: usize,
    },
    /// Fixed-width evidence encoding overflowed.
    Arithmetic,
    /// Raw evidence exceeded the absolute byte ceiling before hashing.
    EvidenceTooLarge,
    /// Raw evidence did not match its claimed SHA-256 identity.
    EvidenceDigestMismatch,
    /// Raw evidence was truncated, noncanonical, or carried an invalid field.
    EvidenceMalformed(&'static str),
    /// Raw evidence named an identity other than the supplied deployment.
    EvidenceIdentityMismatch(&'static str),
    /// Artifact-declared replay policy exceeded caller admission.
    EvidenceLimitEscalation(&'static str),
    /// Fresh actor execution did not reproduce the supplied canonical bytes.
    EvidenceReplayMismatch,
    /// Independent firmware actor admission, preparation, or release failed.
    Actor(GraphLiveError),
}

impl fmt::Display for GraphDeploymentReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => {
                formatter.write_str("graph deployment replay limits are invalid")
            }
            Self::ServiceDomainPresent => formatter
                .write_str("graph deployment replay currently accepts Realtime-only packages"),
            Self::RealtimeDomainAbsent => {
                formatter.write_str("graph deployment replay has no Realtime domain")
            }
            Self::Empty => formatter.write_str("graph deployment replay has no release input"),
            Self::LimitExceeded { aspect, release } => match release {
                Some(release) => write!(
                    formatter,
                    "graph deployment replay release {release} exceeds {aspect} policy"
                ),
                None => write!(formatter, "graph deployment replay exceeds {aspect} policy"),
            },
            Self::DuplicateResource { release, resource } => write!(
                formatter,
                "graph deployment replay release {release} duplicates {resource:?}"
            ),
            Self::NonmonotonicCycle { release } => write!(
                formatter,
                "graph deployment replay release {release} is not strictly later"
            ),
            Self::Arithmetic => {
                formatter.write_str("graph deployment replay arithmetic overflowed")
            }
            Self::EvidenceTooLarge => {
                formatter.write_str("graph deployment replay evidence exceeds byte policy")
            }
            Self::EvidenceDigestMismatch => formatter
                .write_str("graph deployment replay evidence digest does not match its bytes"),
            Self::EvidenceMalformed(aspect) => write!(
                formatter,
                "graph deployment replay evidence has invalid {aspect}"
            ),
            Self::EvidenceIdentityMismatch(aspect) => write!(
                formatter,
                "graph deployment replay evidence has foreign {aspect} identity"
            ),
            Self::EvidenceLimitEscalation(aspect) => write!(
                formatter,
                "graph deployment replay evidence exceeds caller {aspect} admission"
            ),
            Self::EvidenceReplayMismatch => formatter
                .write_str("graph deployment replay evidence was not reproduced byte for byte"),
            Self::Actor(error) => write!(
                formatter,
                "graph deployment firmware actor rejected: {error:?}"
            ),
        }
    }
}

impl std::error::Error for GraphDeploymentReplayError {}

/// Run one Realtime-only lowered package through the actual portable firmware actors.
///
/// The caller supplies the exact static image opcode/resource palette plus a
/// nonzero transaction/run/start identity. A terminal execution fault is a
/// successful replay result and stops later requested frames; admission or
/// lifecycle failures are returned as [`GraphDeploymentReplayError`].
#[allow(
    clippy::too_many_arguments,
    reason = "exact package, target, image palette, run identity, frames, and independent bounds are distinct replay authorities"
)]
pub fn replay_realtime_graph_deployment(
    deployment: &GraphDeploymentReport,
    target: GraphDeploymentTarget,
    opcodes: &'static [GraphOpcodeDescriptor],
    resources: &'static [GraphResourceDescriptor],
    transaction_id: u64,
    run_id: u64,
    start_cycle: DeviceCycle,
    inputs: &[GraphDeploymentReplayInput],
    limits: GraphDeploymentReplayLimits,
) -> Result<GraphDeploymentReplay, GraphDeploymentReplayError> {
    limits.validate()?;
    let package = deployment.package();
    if package.header().service_schedule.node_count != 0 {
        return Err(GraphDeploymentReplayError::ServiceDomainPresent);
    }
    if package.header().realtime_schedule.node_count == 0 {
        return Err(GraphDeploymentReplayError::RealtimeDomainAbsent);
    }
    validate_inputs(inputs, limits)?;

    let content_digest = graph_ir_content_digest(package.bytes());
    let run = GraphRunIdentity {
        transaction_id,
        run_id,
        content_digest,
        package_digest: package.digest(),
        start_cycle,
    };
    let authority = GraphRuntimeAuthority {
        device_id: target.device_id,
        capability_digest: target.capability_digest,
        config_digest: target.config_digest,
        implementation_digest: deployment.implementation_digest(),
    };
    let runtime_limits = GraphRuntimeLimits::fixed_with_capabilities::<
        REPLAY_SERVICE_STATE_BYTES,
        REPLAY_REALTIME_STATE_BYTES,
        REPLAY_SERVICE_CHANNEL_BYTES,
        REPLAY_REALTIME_CHANNEL_BYTES,
        REPLAY_BRIDGE_BYTES,
    >(opcodes, resources);
    let bridge = ReloadableGraphBridge::<REPLAY_BRIDGE_BYTES>::new();
    let mut service = ReplayServiceActor::new(&bridge);
    let mut realtime = ReplayRealtimeActor::new(&bridge);
    service
        .install(
            package.bytes(),
            transaction_id,
            content_digest,
            package.digest(),
            authority,
            runtime_limits,
            true,
        )
        .map_err(GraphDeploymentReplayError::Actor)?;
    realtime
        .install(
            package.bytes(),
            transaction_id,
            content_digest,
            package.digest(),
            authority,
            runtime_limits,
            true,
        )
        .map_err(GraphDeploymentReplayError::Actor)?;
    service
        .prepare_start(run, true)
        .map_err(GraphDeploymentReplayError::Actor)?;
    realtime
        .prepare_start(run, true)
        .map_err(GraphDeploymentReplayError::Actor)?;
    realtime
        .activate(run)
        .map_err(GraphDeploymentReplayError::Actor)?;
    service
        .observe_realtime_started(run)
        .map_err(GraphDeploymentReplayError::Actor)?;

    let mut releases = Vec::with_capacity(inputs.len());
    for (release_index, input) in inputs.iter().enumerate() {
        let mut reads = Vec::with_capacity(limits.maximum_reads_per_release);
        let mut read_overflow = false;
        let result = realtime.release(input.cycle, input.safety_authorized, |resource| {
            if reads.len() < limits.maximum_reads_per_release {
                reads.push(resource);
            } else {
                read_overflow = true;
            }
            input
                .resources
                .iter()
                .find(|sample| sample.resource == resource)
                .and_then(|sample| sample.value)
        });
        if read_overflow {
            return Err(GraphDeploymentReplayError::LimitExceeded {
                aspect: "resource-read count",
                release: Some(release_index),
            });
        }
        match result {
            Ok(report) => releases.push(GraphDeploymentReplayRelease {
                cycle: input.cycle,
                reads,
                outcome: GraphDeploymentReplayReleaseOutcome::Completed(report),
            }),
            Err(GraphLiveError::Execution(error)) => {
                releases.push(GraphDeploymentReplayRelease {
                    cycle: input.cycle,
                    reads,
                    outcome: GraphDeploymentReplayReleaseOutcome::Faulted(error),
                });
                break;
            }
            Err(error) => return Err(GraphDeploymentReplayError::Actor(error)),
        }
    }

    let evidence = canonical_replay_evidence(target, deployment, run, inputs, &releases, limits)?;
    Ok(GraphDeploymentReplay {
        run,
        implementation_digest: deployment.implementation_digest(),
        evidence,
        requested_releases: inputs.len(),
        releases,
    })
}

/// Independently rebuild a canonical replay artifact through fresh actors.
///
/// The artifact's declared limits must fit within `admission_limits`. Exact
/// deployment identities and static image palettes remain caller supplied;
/// this function grants no device or runtime authority.
pub fn replay_graph_deployment_evidence(
    evidence: &CanonicalGraphDeploymentReplayEvidence1,
    deployment: &GraphDeploymentReport,
    target: GraphDeploymentTarget,
    opcodes: &'static [GraphOpcodeDescriptor],
    resources: &'static [GraphResourceDescriptor],
    admission_limits: GraphDeploymentReplayLimits,
) -> Result<GraphDeploymentReplay, GraphDeploymentReplayError> {
    verify_graph_deployment_evidence_bytes(
        evidence.encoded(),
        evidence.digest(),
        deployment,
        target,
        opcodes,
        resources,
        admission_limits,
    )
}

/// Verify raw canonical evidence before reproducing it through fresh actors.
///
/// Hash, size, canonical input encoding, exact deployment identity, and caller
/// limit admission are checked before actor execution. Success additionally
/// requires the fresh transcript to match every supplied byte.
#[allow(
    clippy::too_many_arguments,
    reason = "raw identity, deployment, target, image palettes, and caller replay admission are independent verification inputs"
)]
pub fn verify_graph_deployment_evidence_bytes(
    encoded: &[u8],
    expected_digest: Digest,
    deployment: &GraphDeploymentReport,
    target: GraphDeploymentTarget,
    opcodes: &'static [GraphOpcodeDescriptor],
    resources: &'static [GraphResourceDescriptor],
    admission_limits: GraphDeploymentReplayLimits,
) -> Result<GraphDeploymentReplay, GraphDeploymentReplayError> {
    admission_limits.validate()?;
    if encoded.len() > MAX_GRAPH_DEPLOYMENT_REPLAY_EVIDENCE_BYTES {
        return Err(GraphDeploymentReplayError::EvidenceTooLarge);
    }
    if sha256(encoded).digest != expected_digest {
        return Err(GraphDeploymentReplayError::EvidenceDigestMismatch);
    }
    let decoded = decode_replay_evidence_inputs(encoded, deployment, target, admission_limits)?;
    let replay = replay_realtime_graph_deployment(
        deployment,
        target,
        opcodes,
        resources,
        decoded.transaction_id,
        decoded.run_id,
        decoded.start_cycle,
        &decoded.inputs,
        decoded.limits,
    )?;
    if replay.evidence().encoded() != encoded || replay.evidence_digest() != expected_digest {
        return Err(GraphDeploymentReplayError::EvidenceReplayMismatch);
    }
    Ok(replay)
}

struct DecodedReplayEvidenceInputs {
    transaction_id: u64,
    run_id: u64,
    start_cycle: DeviceCycle,
    inputs: Vec<GraphDeploymentReplayInput>,
    limits: GraphDeploymentReplayLimits,
}

fn decode_replay_evidence_inputs(
    encoded: &[u8],
    deployment: &GraphDeploymentReport,
    target: GraphDeploymentTarget,
    admission_limits: GraphDeploymentReplayLimits,
) -> Result<DecodedReplayEvidenceInputs, GraphDeploymentReplayError> {
    let mut reader = ReplayEvidenceReader::new(encoded);
    if reader.fixed::<8>()? != GRAPH_DEPLOYMENT_REPLAY_MAGIC {
        return Err(GraphDeploymentReplayError::EvidenceMalformed("magic"));
    }
    if DeviceId(reader.fixed::<16>()?) != target.device_id {
        return Err(GraphDeploymentReplayError::EvidenceIdentityMismatch(
            "device",
        ));
    }
    if Digest(reader.fixed::<32>()?) != target.capability_digest {
        return Err(GraphDeploymentReplayError::EvidenceIdentityMismatch(
            "capability",
        ));
    }
    if Digest(reader.fixed::<32>()?) != target.config_digest {
        return Err(GraphDeploymentReplayError::EvidenceIdentityMismatch(
            "configuration",
        ));
    }
    if Digest(reader.fixed::<32>()?) != deployment.implementation_digest() {
        return Err(GraphDeploymentReplayError::EvidenceIdentityMismatch(
            "implementation",
        ));
    }
    let package = deployment.package();
    if Digest(reader.fixed::<32>()?) != graph_ir_content_digest(package.bytes()) {
        return Err(GraphDeploymentReplayError::EvidenceIdentityMismatch(
            "content",
        ));
    }
    if Digest(reader.fixed::<32>()?) != package.digest() {
        return Err(GraphDeploymentReplayError::EvidenceIdentityMismatch(
            "package",
        ));
    }

    let transaction_id = reader.u64()?;
    let run_id = reader.u64()?;
    let start_cycle = DeviceCycle(reader.u64()?);
    let limits = GraphDeploymentReplayLimits {
        maximum_releases: reader.usize()?,
        maximum_inputs_per_release: reader.usize()?,
        maximum_reads_per_release: reader.usize()?,
    };
    if limits.validate().is_err() {
        return Err(GraphDeploymentReplayError::EvidenceMalformed("limits"));
    }
    require_admitted_limits(limits, admission_limits)?;

    let input_count = reader.usize()?;
    if input_count == 0 || input_count > limits.maximum_releases {
        return Err(GraphDeploymentReplayError::EvidenceMalformed(
            "release count",
        ));
    }
    let mut inputs = Vec::with_capacity(input_count);
    for _ in 0..input_count {
        let cycle = DeviceCycle(reader.u64()?);
        let safety_authorized = match reader.u8()? {
            0 => false,
            1 => true,
            _ => {
                return Err(GraphDeploymentReplayError::EvidenceMalformed(
                    "safety value",
                ));
            }
        };
        let sample_count = reader.usize()?;
        if sample_count > limits.maximum_inputs_per_release {
            return Err(GraphDeploymentReplayError::EvidenceMalformed(
                "resource-input count",
            ));
        }
        let mut samples = Vec::with_capacity(sample_count);
        let mut previous = None;
        for _ in 0..sample_count {
            let resource = decode_resource_id(&reader.fixed::<4>()?)
                .map_err(|_| GraphDeploymentReplayError::EvidenceMalformed("resource"))?;
            if previous.is_some_and(|previous| previous >= resource) {
                return Err(GraphDeploymentReplayError::EvidenceMalformed(
                    "resource order",
                ));
            }
            previous = Some(resource);
            let value = match reader.u8()? {
                0 => None,
                1 => Some(false),
                2 => Some(true),
                _ => {
                    return Err(GraphDeploymentReplayError::EvidenceMalformed(
                        "resource value",
                    ));
                }
            };
            samples.push(GraphDeploymentResourceSample::new(resource, value));
        }
        inputs.push(GraphDeploymentReplayInput::new(
            cycle,
            safety_authorized,
            samples,
        ));
    }
    if reader.remaining() < 4 {
        return Err(GraphDeploymentReplayError::EvidenceMalformed(
            "release results",
        ));
    }
    Ok(DecodedReplayEvidenceInputs {
        transaction_id,
        run_id,
        start_cycle,
        inputs,
        limits,
    })
}

fn require_admitted_limits(
    declared: GraphDeploymentReplayLimits,
    admitted: GraphDeploymentReplayLimits,
) -> Result<(), GraphDeploymentReplayError> {
    for (aspect, declared, admitted) in [
        (
            "release-count",
            declared.maximum_releases,
            admitted.maximum_releases,
        ),
        (
            "resource-input",
            declared.maximum_inputs_per_release,
            admitted.maximum_inputs_per_release,
        ),
        (
            "resource-read",
            declared.maximum_reads_per_release,
            admitted.maximum_reads_per_release,
        ),
    ] {
        if declared > admitted {
            return Err(GraphDeploymentReplayError::EvidenceLimitEscalation(aspect));
        }
    }
    Ok(())
}

struct ReplayEvidenceReader<'a> {
    encoded: &'a [u8],
    cursor: usize,
}

impl<'a> ReplayEvidenceReader<'a> {
    const fn new(encoded: &'a [u8]) -> Self {
        Self { encoded, cursor: 0 }
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], GraphDeploymentReplayError> {
        let end = self
            .cursor
            .checked_add(N)
            .ok_or(GraphDeploymentReplayError::Arithmetic)?;
        let source = self
            .encoded
            .get(self.cursor..end)
            .ok_or(GraphDeploymentReplayError::EvidenceMalformed("length"))?;
        let mut value = [0_u8; N];
        value.copy_from_slice(source);
        self.cursor = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, GraphDeploymentReplayError> {
        Ok(self.fixed::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, GraphDeploymentReplayError> {
        Ok(u32::from_le_bytes(self.fixed::<4>()?))
    }

    fn u64(&mut self) -> Result<u64, GraphDeploymentReplayError> {
        Ok(u64::from_le_bytes(self.fixed::<8>()?))
    }

    fn usize(&mut self) -> Result<usize, GraphDeploymentReplayError> {
        usize::try_from(self.u32()?).map_err(|_| GraphDeploymentReplayError::Arithmetic)
    }

    fn remaining(&self) -> usize {
        self.encoded.len().saturating_sub(self.cursor)
    }
}

fn validate_inputs(
    inputs: &[GraphDeploymentReplayInput],
    limits: GraphDeploymentReplayLimits,
) -> Result<(), GraphDeploymentReplayError> {
    if inputs.is_empty() {
        return Err(GraphDeploymentReplayError::Empty);
    }
    if inputs.len() > limits.maximum_releases {
        return Err(GraphDeploymentReplayError::LimitExceeded {
            aspect: "release count",
            release: None,
        });
    }
    let mut previous = None;
    for (release, input) in inputs.iter().enumerate() {
        if previous.is_some_and(|cycle: DeviceCycle| input.cycle.0 <= cycle.0) {
            return Err(GraphDeploymentReplayError::NonmonotonicCycle { release });
        }
        previous = Some(input.cycle);
        if input.resources.len() > limits.maximum_inputs_per_release {
            return Err(GraphDeploymentReplayError::LimitExceeded {
                aspect: "resource-input count",
                release: Some(release),
            });
        }
        for (index, sample) in input.resources.iter().enumerate() {
            if input.resources[..index]
                .iter()
                .any(|prior| prior.resource == sample.resource)
            {
                return Err(GraphDeploymentReplayError::DuplicateResource {
                    release,
                    resource: sample.resource,
                });
            }
        }
    }
    Ok(())
}

fn canonical_replay_evidence(
    target: GraphDeploymentTarget,
    deployment: &GraphDeploymentReport,
    run: GraphRunIdentity,
    inputs: &[GraphDeploymentReplayInput],
    releases: &[GraphDeploymentReplayRelease],
    limits: GraphDeploymentReplayLimits,
) -> Result<CanonicalGraphDeploymentReplayEvidence1, GraphDeploymentReplayError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&GRAPH_DEPLOYMENT_REPLAY_MAGIC);
    bytes.extend_from_slice(&target.device_id.0);
    bytes.extend_from_slice(&target.capability_digest.0);
    bytes.extend_from_slice(&target.config_digest.0);
    bytes.extend_from_slice(&deployment.implementation_digest().0);
    bytes.extend_from_slice(&run.content_digest.0);
    bytes.extend_from_slice(&run.package_digest.0);
    put_u64(&mut bytes, run.transaction_id);
    put_u64(&mut bytes, run.run_id);
    put_u64(&mut bytes, run.start_cycle.0);
    put_usize(&mut bytes, limits.maximum_releases)?;
    put_usize(&mut bytes, limits.maximum_inputs_per_release)?;
    put_usize(&mut bytes, limits.maximum_reads_per_release)?;
    put_usize(&mut bytes, inputs.len())?;
    for input in inputs {
        put_u64(&mut bytes, input.cycle.0);
        bytes.push(u8::from(input.safety_authorized));
        put_usize(&mut bytes, input.resources.len())?;
        let mut samples = input.resources.clone();
        samples.sort_unstable_by_key(|sample| sample.resource);
        for sample in samples {
            bytes.extend_from_slice(&encode_resource_id(sample.resource));
            bytes.push(match sample.value {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            });
        }
    }
    put_usize(&mut bytes, releases.len())?;
    for release in releases {
        put_u64(&mut bytes, release.cycle.0);
        put_usize(&mut bytes, release.reads.len())?;
        for resource in &release.reads {
            bytes.extend_from_slice(&encode_resource_id(*resource));
        }
        match release.outcome {
            GraphDeploymentReplayReleaseOutcome::Completed(report) => {
                bytes.push(1);
                put_u64(&mut bytes, report.cycle.0);
                put_u64(&mut bytes, report.release_tick);
                bytes.extend_from_slice(&report.nodes_executed.to_le_bytes());
                bytes.extend_from_slice(&report.items_consumed.to_le_bytes());
                bytes.extend_from_slice(&report.items_emitted.to_le_bytes());
                bytes.extend_from_slice(&report.sink_items.to_le_bytes());
                bytes.push(match report.last_sink_value {
                    None => 0,
                    Some(false) => 1,
                    Some(true) => 2,
                });
            }
            GraphDeploymentReplayReleaseOutcome::Faulted(error) => {
                bytes.push(2);
                bytes.extend_from_slice(&error.observation.generation.to_le_bytes());
                bytes.push(error.observation.fault as u8);
                bytes.push(error.observation.detail);
                put_optional_cycle(&mut bytes, error.expected_cycle);
                put_optional_cycle(&mut bytes, error.received_cycle);
            }
        }
    }
    if bytes.len() > MAX_GRAPH_DEPLOYMENT_REPLAY_EVIDENCE_BYTES {
        return Err(GraphDeploymentReplayError::EvidenceTooLarge);
    }
    let digest = sha256(&bytes).digest;
    Ok(CanonicalGraphDeploymentReplayEvidence1 {
        encoded: bytes,
        digest,
    })
}

fn put_usize(bytes: &mut Vec<u8>, value: usize) -> Result<(), GraphDeploymentReplayError> {
    let value = u32::try_from(value).map_err(|_| GraphDeploymentReplayError::Arithmetic)?;
    bytes.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_optional_cycle(bytes: &mut Vec<u8>, value: Option<DeviceCycle>) {
    match value {
        None => bytes.push(0),
        Some(value) => {
            bytes.push(1);
            put_u64(bytes, value.0);
        }
    }
}
