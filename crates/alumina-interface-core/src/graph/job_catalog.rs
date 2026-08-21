//! Cache-derived immutable job references for graph authoring.
//!
//! A canonical global job describes the exact per-MCU partitions that should
//! exist. Cache delivery separately proves which exact partition and shared
//! manifest publications were observed on each participant. This module joins
//! those two authorities into a bounded, canonical editor catalog. It does not
//! authenticate a transport, start a job, allocate a resource, or make a graph
//! executable.

use core::fmt;

use alumina_job::MachineJobParticipant;
use alumina_protocol::{DeviceId, Digest};
use alumina_storage::{ObjectKind, PublishedObject};

use crate::CanonicalGlobalJob2;

use super::{
    CanonicalGraphWorkspaceEncoding, GraphAnalysisError, GraphNodeId, GraphNodeRegistry,
    GraphSchema, GraphSchemaError, GraphTypeId, GraphValue, GraphWorkspaceDocument,
    GraphWorkspaceError, JobGraphHandle, TypeKind, TypedGraphValue, analyze_graph_draft,
    encode_graph_workspace,
};

/// Caller-owned bound for one cache-derived job catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphCachedJobCatalogLimits {
    /// Maximum participant-local handles retained for one global job.
    pub maximum_entries: usize,
}

impl GraphCachedJobCatalogLimits {
    /// Bounded first interactive policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_entries: 4_096,
        }
    }

    fn validate(self) -> Result<(), GraphCachedJobCatalogError> {
        if self.maximum_entries == 0 {
            Err(GraphCachedJobCatalogError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphCachedJobCatalogLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Data-only proof that one MCU reported both immutable cache publications.
///
/// Construction validates only the self-contained identity shape. Callers must
/// obtain the publications from a reconciled, authenticated cache-delivery
/// state machine. [`derive_graph_cached_job_catalog`] then requires exact
/// equality with a canonical compiled job before producing any handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphCachedJobPublicationEvidence {
    device_id: DeviceId,
    partition: PublishedObject,
    global_manifest: PublishedObject,
}

impl GraphCachedJobPublicationEvidence {
    /// Validate and retain one participant's observed cache publications.
    ///
    /// # Errors
    ///
    /// Rejects zero device/content identities, empty objects, wrong object
    /// kinds, or missing ordered-manifest identities.
    pub fn try_new(
        device_id: DeviceId,
        partition: PublishedObject,
        global_manifest: PublishedObject,
    ) -> Result<Self, GraphCachedJobCatalogError> {
        if device_id.0.iter().all(|byte| *byte == 0) {
            return Err(GraphCachedJobCatalogError::InvalidEvidence(
                "device identity",
            ));
        }
        validate_publication(
            partition,
            ObjectKind::MachineJobPartition,
            "partition publication",
        )?;
        validate_publication(
            global_manifest,
            ObjectKind::MachineJobManifest,
            "global-manifest publication",
        )?;
        Ok(Self {
            device_id,
            partition,
            global_manifest,
        })
    }

    /// Return the stable participant MCU identity.
    pub const fn device_id(self) -> DeviceId {
        self.device_id
    }

    /// Return the exact observed participant partition publication.
    pub const fn partition(self) -> PublishedObject {
        self.partition
    }

    /// Return the exact observed shared manifest publication.
    pub const fn global_manifest(self) -> PublishedObject {
        self.global_manifest
    }
}

/// One exact participant-local handle and the manifest facts needed by the UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphCachedJobCatalogEntry {
    handle: JobGraphHandle,
    participant: MachineJobParticipant,
    partition: PublishedObject,
    global_manifest: PublishedObject,
}

impl GraphCachedJobCatalogEntry {
    /// Return the inert graph value identity.
    pub const fn handle(self) -> JobGraphHandle {
        self.handle
    }

    /// Return the complete canonical participant record used by CAM.
    pub const fn participant(self) -> MachineJobParticipant {
        self.participant
    }

    /// Return the exact participant-local cached object publication.
    pub const fn partition(self) -> PublishedObject {
        self.partition
    }

    /// Return the exact shared cached manifest publication.
    pub const fn global_manifest(self) -> PublishedObject {
        self.global_manifest
    }

    /// Construct a schema-checked graph value without parsing an identity.
    ///
    /// # Errors
    ///
    /// Rejects an unknown or non-job root type and forwards bounded graph value
    /// validation errors.
    pub fn typed_value(
        self,
        schema: &GraphSchema,
        value_type: GraphTypeId,
    ) -> Result<TypedGraphValue, GraphCachedJobCatalogError> {
        match schema.value_type(value_type).map(|value| value.kind()) {
            Some(TypeKind::JobHandle) => {
                TypedGraphValue::try_new(schema, value_type, GraphValue::JobHandle(self.handle))
                    .map_err(GraphCachedJobCatalogError::GraphValue)
            }
            _ => Err(GraphCachedJobCatalogError::NotJobHandleType(value_type)),
        }
    }
}

/// Canonical participant-local choices for one fully cached global job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphCachedJobCatalog {
    global_job_digest: Digest,
    participant_set_digest: Digest,
    global_manifest: PublishedObject,
    entries: Vec<GraphCachedJobCatalogEntry>,
}

impl GraphCachedJobCatalog {
    /// Return the exact shared global job identity.
    pub const fn global_job_digest(&self) -> Digest {
        self.global_job_digest
    }

    /// Return the exact canonical participant-set identity.
    pub const fn participant_set_digest(&self) -> Digest {
        self.participant_set_digest
    }

    /// Return the shared manifest publication observed on every participant.
    pub const fn global_manifest(&self) -> PublishedObject {
        self.global_manifest
    }

    /// Borrow participant-local entries in stable device order.
    pub fn entries(&self) -> &[GraphCachedJobCatalogEntry] {
        &self.entries
    }

    /// Resolve an exact catalog-owned immutable handle.
    pub fn entry_index_for_handle(&self, handle: JobGraphHandle) -> Option<usize> {
        self.entries
            .binary_search_by_key(&handle.device_id, |entry| entry.handle.device_id)
            .ok()
            .filter(|index| self.entries[*index].handle == handle)
    }
}

/// Rejection while deriving cache-authoritative graph choices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphCachedJobCatalogError {
    /// A caller-owned bound was zero.
    ZeroLimit,
    /// The complete participant set exceeds caller policy.
    EntryLimitExceeded,
    /// Retaining the already bounded catalog could not reserve memory.
    AllocationOverflow,
    /// Evidence did not contain exactly one item per canonical participant.
    ParticipantCountMismatch {
        /// Participants named by the canonical job.
        expected: usize,
        /// Observed cache-ready evidence items.
        received: usize,
    },
    /// Evidence repeated one device identity.
    DuplicateDevice(DeviceId),
    /// A self-contained evidence identity was invalid.
    InvalidEvidence(&'static str),
    /// Canonical package and manifest participant records contradicted each other.
    CanonicalParticipantMismatch(usize),
    /// Evidence named a device outside the canonical participant position.
    ParticipantIdentityMismatch {
        /// Canonical stable-order position.
        index: usize,
        /// Device required at that position.
        expected: DeviceId,
        /// Device supplied by evidence.
        received: DeviceId,
    },
    /// The observed partition publication was not the compiled participant object.
    PartitionPublicationMismatch(DeviceId),
    /// The observed global manifest was not the compiled shared object.
    GlobalManifestPublicationMismatch(DeviceId),
    /// A requested graph type is not a root job handle.
    NotJobHandleType(GraphTypeId),
    /// A derived graph value contradicted the supplied schema.
    GraphValue(GraphSchemaError),
}

impl fmt::Display for GraphCachedJobCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph cached-job catalog limit is zero"),
            Self::EntryLimitExceeded => {
                formatter.write_str("cached-job participants exceed graph catalog policy")
            }
            Self::AllocationOverflow => formatter.write_str("cached-job catalog allocation failed"),
            Self::ParticipantCountMismatch { expected, received } => write!(
                formatter,
                "cached-job evidence count {received} does not match {expected} participants"
            ),
            Self::DuplicateDevice(device) => {
                write!(formatter, "cached-job evidence repeats device {device:?}")
            }
            Self::InvalidEvidence(aspect) => {
                write!(formatter, "cached-job evidence has invalid {aspect}")
            }
            Self::CanonicalParticipantMismatch(index) => write!(
                formatter,
                "canonical cached-job participant {index} contradicts its package"
            ),
            Self::ParticipantIdentityMismatch {
                index,
                expected,
                received,
            } => write!(
                formatter,
                "cached-job evidence {index} names {received:?}, expected {expected:?}"
            ),
            Self::PartitionPublicationMismatch(device) => write!(
                formatter,
                "cached partition publication does not match participant {device:?}"
            ),
            Self::GlobalManifestPublicationMismatch(device) => write!(
                formatter,
                "cached global manifest publication does not match participant {device:?}"
            ),
            Self::NotJobHandleType(value_type) => {
                write!(formatter, "graph type {value_type:?} is not a job handle")
            }
            Self::GraphValue(error) => write!(formatter, "cached job handle is invalid: {error}"),
        }
    }
}

impl std::error::Error for GraphCachedJobCatalogError {}

/// Rejection while replacing one cache-derived graph job reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphCachedJobHandleSelectionError {
    /// The selected catalog item does not exist.
    UnknownEntry(usize),
    /// The selected graph node does not exist.
    UnknownNode(GraphNodeId),
    /// The selected node parameter does not exist.
    UnknownParameter {
        /// Exact graph node.
        node: GraphNodeId,
        /// Node-local parameter ID.
        parameter: u32,
    },
    /// The selected parameter is not a root job-handle value.
    NotJobHandleParameter {
        /// Exact graph node.
        node: GraphNodeId,
        /// Node-local parameter ID.
        parameter: u32,
    },
    /// The existing value was not supplied by this exact cache-derived catalog.
    ParameterNotCatalogBound {
        /// Exact graph node.
        node: GraphNodeId,
        /// Node-local parameter ID.
        parameter: u32,
    },
    /// The requested entry is already selected; no revision is created.
    AlreadySelected {
        /// Exact graph node.
        node: GraphNodeId,
        /// Node-local parameter ID.
        parameter: u32,
        /// Exact catalog entry.
        entry: usize,
    },
    /// Catalog value construction contradicted the workspace schema.
    Catalog(GraphCachedJobCatalogError),
    /// Structural workspace mutation or canonical encoding failed.
    Workspace(GraphWorkspaceError),
    /// The complete candidate failed reviewed graph semantics.
    Analysis(GraphAnalysisError),
}

impl fmt::Display for GraphCachedJobHandleSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEntry(entry) => {
                write!(formatter, "graph cached-job entry {entry} is unavailable")
            }
            Self::UnknownNode(node) => write!(formatter, "graph node {node:?} is unavailable"),
            Self::UnknownParameter { node, parameter } => write!(
                formatter,
                "graph node {node:?} has no parameter {parameter}"
            ),
            Self::NotJobHandleParameter { node, parameter } => write!(
                formatter,
                "graph node {node:?} parameter {parameter} is not a job handle"
            ),
            Self::ParameterNotCatalogBound { node, parameter } => write!(
                formatter,
                "graph node {node:?} parameter {parameter} is not bound by this exact cached-job catalog"
            ),
            Self::AlreadySelected {
                node,
                parameter,
                entry,
            } => write!(
                formatter,
                "graph node {node:?} parameter {parameter} already carries cached-job entry {entry}"
            ),
            Self::Catalog(error) => write!(formatter, "job selection failed: {error}"),
            Self::Workspace(error) => write!(formatter, "job selection failed: {error}"),
            Self::Analysis(error) => {
                write!(formatter, "job selection semantic analysis failed: {error}")
            }
        }
    }
}

impl std::error::Error for GraphCachedJobHandleSelectionError {}

impl From<GraphCachedJobCatalogError> for GraphCachedJobHandleSelectionError {
    fn from(value: GraphCachedJobCatalogError) -> Self {
        Self::Catalog(value)
    }
}

impl From<GraphWorkspaceError> for GraphCachedJobHandleSelectionError {
    fn from(value: GraphWorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl From<GraphAnalysisError> for GraphCachedJobHandleSelectionError {
    fn from(value: GraphAnalysisError) -> Self {
        Self::Analysis(value)
    }
}

/// Join a canonical compiled job with exact per-MCU cache-ready observations.
///
/// Evidence discovery order is erased by stable `DeviceId` sorting. Every
/// partition and every copy of the shared manifest must compare byte-identity
/// metadata exactly with the compiler artifact before any entry is returned.
pub fn derive_graph_cached_job_catalog(
    job: &CanonicalGlobalJob2,
    evidence: &[GraphCachedJobPublicationEvidence],
    limits: GraphCachedJobCatalogLimits,
) -> Result<GraphCachedJobCatalog, GraphCachedJobCatalogError> {
    limits.validate()?;
    let packages = job.participants();
    let records = job.participant_records();
    if packages.len() > limits.maximum_entries {
        return Err(GraphCachedJobCatalogError::EntryLimitExceeded);
    }
    if evidence.len() != packages.len() {
        return Err(GraphCachedJobCatalogError::ParticipantCountMismatch {
            expected: packages.len(),
            received: evidence.len(),
        });
    }
    if records.len() != packages.len() {
        return Err(GraphCachedJobCatalogError::ParticipantCountMismatch {
            expected: packages.len(),
            received: records.len(),
        });
    }

    let mut observed = evidence.to_vec();
    observed.sort_unstable_by_key(|item| item.device_id);
    for pair in observed.windows(2) {
        if pair[0].device_id == pair[1].device_id {
            return Err(GraphCachedJobCatalogError::DuplicateDevice(
                pair[0].device_id,
            ));
        }
    }

    let global_manifest = job.publication();
    if global_manifest.object.content.digest != job.global_job_digest() {
        return Err(GraphCachedJobCatalogError::InvalidEvidence(
            "canonical global job identity",
        ));
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(packages.len())
        .map_err(|_| GraphCachedJobCatalogError::AllocationOverflow)?;
    for (index, ((package, participant), cached)) in
        packages.iter().zip(records).zip(&observed).enumerate()
    {
        if package.device_id() != participant.device_id
            || package.partition().publication().object.content.digest
                != participant.partition_digest
        {
            return Err(GraphCachedJobCatalogError::CanonicalParticipantMismatch(
                index,
            ));
        }
        if cached.device_id != participant.device_id {
            return Err(GraphCachedJobCatalogError::ParticipantIdentityMismatch {
                index,
                expected: participant.device_id,
                received: cached.device_id,
            });
        }
        let partition = package.partition().publication();
        if cached.partition != partition {
            return Err(GraphCachedJobCatalogError::PartitionPublicationMismatch(
                participant.device_id,
            ));
        }
        if cached.global_manifest != global_manifest {
            return Err(
                GraphCachedJobCatalogError::GlobalManifestPublicationMismatch(
                    participant.device_id,
                ),
            );
        }
        entries.push(GraphCachedJobCatalogEntry {
            handle: JobGraphHandle {
                device_id: participant.device_id,
                global_job_digest: job.global_job_digest(),
                partition_digest: participant.partition_digest,
            },
            participant: *participant,
            partition,
            global_manifest,
        });
    }

    Ok(GraphCachedJobCatalog {
        global_job_digest: job.global_job_digest(),
        participant_set_digest: job.participant_set_digest(),
        global_manifest,
        entries,
    })
}

/// Transactionally select another participant-local immutable job reference.
///
/// The current value must already be an exact member of `catalog`; raw or stale
/// identities therefore cannot acquire selector authority. The complete graph
/// candidate is structurally validated, semantically analyzed, and canonically
/// encoded before the caller's workspace changes. Duplicate references in
/// other nodes are deliberately allowed because a job handle is inert data,
/// not a resource lease or start capability.
pub fn select_graph_cached_job_handle(
    catalog: &GraphCachedJobCatalog,
    registry: &GraphNodeRegistry,
    workspace: &mut GraphWorkspaceDocument,
    node_id: GraphNodeId,
    parameter_id: u32,
    entry_index: usize,
) -> Result<CanonicalGraphWorkspaceEncoding, GraphCachedJobHandleSelectionError> {
    let entry = catalog.entries().get(entry_index).copied().ok_or(
        GraphCachedJobHandleSelectionError::UnknownEntry(entry_index),
    )?;
    let node = workspace
        .graph()
        .node(node_id)
        .ok_or(GraphCachedJobHandleSelectionError::UnknownNode(node_id))?;
    let parameter = node
        .parameters()
        .iter()
        .find(|parameter| parameter.id() == parameter_id)
        .ok_or(GraphCachedJobHandleSelectionError::UnknownParameter {
            node: node_id,
            parameter: parameter_id,
        })?;
    let GraphValue::JobHandle(current) = parameter.value().value() else {
        return Err(GraphCachedJobHandleSelectionError::NotJobHandleParameter {
            node: node_id,
            parameter: parameter_id,
        });
    };
    let Some(current_entry) = catalog.entry_index_for_handle(*current) else {
        return Err(
            GraphCachedJobHandleSelectionError::ParameterNotCatalogBound {
                node: node_id,
                parameter: parameter_id,
            },
        );
    };
    if current_entry == entry_index {
        return Err(GraphCachedJobHandleSelectionError::AlreadySelected {
            node: node_id,
            parameter: parameter_id,
            entry: entry_index,
        });
    }
    let selected = entry.typed_value(workspace.graph().schema(), parameter.value().value_type())?;

    let mut candidate = workspace.clone();
    candidate.set_parameter(node_id, parameter_id, selected)?;
    analyze_graph_draft(candidate.graph(), registry)?;
    let encoding = encode_graph_workspace(&candidate)?;
    *workspace = candidate;
    Ok(encoding)
}

fn validate_publication(
    publication: PublishedObject,
    expected_kind: ObjectKind,
    aspect: &'static str,
) -> Result<(), GraphCachedJobCatalogError> {
    if publication.object.kind != expected_kind
        || publication.object.byte_len == 0
        || !publication.object.content.is_valid()
        || !publication.manifest.is_valid()
    {
        Err(GraphCachedJobCatalogError::InvalidEvidence(aspect))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alumina_storage::{ObjectKind, StoredObject};

    use super::*;
    use crate::{compile_representative_global_job, compile_representative_program};

    use super::super::{
        ExecutionDomain, ExecutionDomainSet, GraphAnalysisLimits, GraphDocument, GraphLimits,
        GraphNodePrototype, GraphSchema, GraphWorkspaceLimits, NodeKind, NodeParameter,
        NodeParameterContract, NodeSchema, TypeDefinition,
    };

    const JOB: GraphTypeId = GraphTypeId::new(1);
    const BOOL: GraphTypeId = GraphTypeId::new(2);
    const KIND_NAME: &str = "alumina.job.cached-reference";

    fn canonical_job() -> CanonicalGlobalJob2 {
        let program = compile_representative_program().unwrap();
        compile_representative_global_job(&program).unwrap()
    }

    fn evidence(job: &CanonicalGlobalJob2) -> Vec<GraphCachedJobPublicationEvidence> {
        let mut result: Vec<_> = job
            .participants()
            .iter()
            .map(|participant| {
                GraphCachedJobPublicationEvidence::try_new(
                    participant.device_id(),
                    participant.partition().publication(),
                    job.publication(),
                )
                .unwrap()
            })
            .collect();
        result.reverse();
        result
    }

    fn catalog(job: &CanonicalGlobalJob2) -> GraphCachedJobCatalog {
        derive_graph_cached_job_catalog(
            job,
            &evidence(job),
            GraphCachedJobCatalogLimits::interactive(),
        )
        .unwrap()
    }

    fn registry() -> GraphNodeRegistry {
        let schema = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![
                TypeDefinition::new(JOB, "job.cached", TypeKind::JobHandle),
                TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
            ],
        )
        .unwrap();
        let context =
            GraphDocument::try_new(0, schema, Vec::new(), Vec::new(), Vec::new()).unwrap();
        GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            &context,
            vec![NodeSchema::new(
                NodeKind::new(KIND_NAME, 1),
                ExecutionDomainSet::HOST_EXACT,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![
                    NodeParameterContract::new(1, "job", JOB),
                    NodeParameterContract::new(2, "armed", BOOL),
                ],
                Vec::new(),
                Vec::new(),
                None,
            )],
        )
        .unwrap()
    }

    fn workspace(
        registry: &GraphNodeRegistry,
        catalog: &GraphCachedJobCatalog,
    ) -> (GraphWorkspaceDocument, GraphNodeId) {
        let graph = GraphDocument::try_new(
            0,
            registry.context_schema().clone(),
            registry.context_clocks().to_vec(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let mut workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            0,
            1,
            1,
            graph,
            Vec::new(),
        )
        .unwrap();
        let node = workspace
            .create_node(
                GraphNodePrototype::new(
                    NodeKind::new(KIND_NAME, 1),
                    "Cached participant",
                    ExecutionDomain::HostExact,
                    Vec::new(),
                    Vec::new(),
                    vec![
                        NodeParameter::new(
                            1,
                            "job",
                            catalog.entries()[0]
                                .typed_value(registry.context_schema(), JOB)
                                .unwrap(),
                        ),
                        NodeParameter::new(
                            2,
                            "armed",
                            TypedGraphValue::try_new(
                                registry.context_schema(),
                                BOOL,
                                GraphValue::Boolean(false),
                            )
                            .unwrap(),
                        ),
                    ],
                ),
                17,
                -23,
            )
            .unwrap();
        analyze_graph_draft(workspace.graph(), registry).unwrap();
        (workspace, node)
    }

    #[test]
    fn derivation_erases_discovery_order_and_retains_complete_manifest_facts() {
        let job = canonical_job();
        let catalog = catalog(&job);
        assert_eq!(catalog.entries().len(), job.participants().len());
        assert_eq!(catalog.global_job_digest(), job.global_job_digest());
        assert_eq!(
            catalog.participant_set_digest(),
            job.participant_set_digest()
        );
        assert_eq!(catalog.global_manifest(), job.publication());
        assert!(
            catalog
                .entries()
                .windows(2)
                .all(|pair| { pair[0].handle().device_id < pair[1].handle().device_id })
        );
        for (entry, record) in catalog.entries().iter().zip(job.participant_records()) {
            assert_eq!(entry.handle().device_id, record.device_id);
            assert_eq!(entry.handle().global_job_digest, job.global_job_digest());
            assert_eq!(entry.handle().partition_digest, record.partition_digest);
            assert_eq!(entry.participant(), *record);
            assert_eq!(entry.partition().object.byte_len, record.partition_byte_len);
            assert_eq!(entry.global_manifest(), job.publication());
        }
    }

    #[test]
    fn evidence_shape_and_exact_publications_fail_closed() {
        let job = canonical_job();
        let mut observed = evidence(&job);

        assert_eq!(
            derive_graph_cached_job_catalog(
                &job,
                &observed[..1],
                GraphCachedJobCatalogLimits::interactive(),
            ),
            Err(GraphCachedJobCatalogError::ParticipantCountMismatch {
                expected: 2,
                received: 1,
            })
        );

        observed[1].device_id = observed[0].device_id;
        assert_eq!(
            derive_graph_cached_job_catalog(
                &job,
                &observed,
                GraphCachedJobCatalogLimits::interactive(),
            ),
            Err(GraphCachedJobCatalogError::DuplicateDevice(
                observed[0].device_id
            ))
        );

        let mut observed = evidence(&job);
        observed[0].partition.object.byte_len += 1;
        assert!(matches!(
            derive_graph_cached_job_catalog(
                &job,
                &observed,
                GraphCachedJobCatalogLimits::interactive(),
            ),
            Err(GraphCachedJobCatalogError::PartitionPublicationMismatch(_))
        ));

        let mut observed = evidence(&job);
        observed[0].global_manifest.manifest = observed[0].partition.manifest;
        assert!(matches!(
            derive_graph_cached_job_catalog(
                &job,
                &observed,
                GraphCachedJobCatalogLimits::interactive(),
            ),
            Err(GraphCachedJobCatalogError::GlobalManifestPublicationMismatch(_))
        ));

        let participant = &job.participants()[0];
        let wrong_kind = PublishedObject {
            object: StoredObject {
                kind: ObjectKind::OpaqueData,
                ..participant.partition().publication().object
            },
            manifest: participant.partition().publication().manifest,
        };
        assert_eq!(
            GraphCachedJobPublicationEvidence::try_new(
                participant.device_id(),
                wrong_kind,
                job.publication(),
            ),
            Err(GraphCachedJobCatalogError::InvalidEvidence(
                "partition publication"
            ))
        );
        assert_eq!(
            derive_graph_cached_job_catalog(
                &job,
                &evidence(&job),
                GraphCachedJobCatalogLimits { maximum_entries: 1 },
            ),
            Err(GraphCachedJobCatalogError::EntryLimitExceeded)
        );
        assert_eq!(
            derive_graph_cached_job_catalog(
                &job,
                &evidence(&job),
                GraphCachedJobCatalogLimits { maximum_entries: 0 },
            ),
            Err(GraphCachedJobCatalogError::ZeroLimit)
        );
    }

    #[test]
    fn selector_preserves_identity_and_allows_duplicate_inert_references() {
        let job = canonical_job();
        let catalog = catalog(&job);
        let registry = registry();
        let (mut workspace, node) = workspace(&registry, &catalog);
        let duplicate = workspace
            .create_node(
                GraphNodePrototype::new(
                    NodeKind::new(KIND_NAME, 1),
                    "Second immutable reference",
                    ExecutionDomain::HostExact,
                    Vec::new(),
                    Vec::new(),
                    vec![
                        NodeParameter::new(
                            1,
                            "job",
                            catalog.entries()[0]
                                .typed_value(registry.context_schema(), JOB)
                                .unwrap(),
                        ),
                        NodeParameter::new(
                            2,
                            "armed",
                            TypedGraphValue::try_new(
                                registry.context_schema(),
                                BOOL,
                                GraphValue::Boolean(false),
                            )
                            .unwrap(),
                        ),
                    ],
                ),
                400,
                10,
            )
            .unwrap();
        analyze_graph_draft(workspace.graph(), &registry).unwrap();
        assert_ne!(node, duplicate);

        let before = encode_graph_workspace(&workspace).unwrap();
        let revision = workspace.revision();
        let graph_revision = workspace.graph().revision();
        let node_cursor = workspace.next_node_id();
        let wire_cursor = workspace.next_wire_id();
        let placement = workspace.placement(node);
        let selected =
            select_graph_cached_job_handle(&catalog, &registry, &mut workspace, node, 1, 1)
                .unwrap();
        assert_ne!(selected, before);
        assert_eq!(selected, encode_graph_workspace(&workspace).unwrap());
        assert_eq!(workspace.revision(), revision + 1);
        assert_eq!(workspace.graph().revision(), graph_revision + 1);
        assert_eq!(workspace.next_node_id(), node_cursor);
        assert_eq!(workspace.next_wire_id(), wire_cursor);
        assert_eq!(workspace.placement(node), placement);
        let GraphValue::JobHandle(handle) = workspace.graph().node(node).unwrap().parameters()[0]
            .value()
            .value()
        else {
            panic!("selected parameter was not a job handle")
        };
        assert_eq!(*handle, catalog.entries()[1].handle());
        let GraphValue::JobHandle(duplicate_handle) =
            workspace.graph().node(duplicate).unwrap().parameters()[0]
                .value()
                .value()
        else {
            panic!("duplicate parameter was not a job handle")
        };
        assert_eq!(*duplicate_handle, catalog.entries()[0].handle());
    }

    #[test]
    fn selector_rejects_noop_raw_wrong_parameter_and_semantics_atomically() {
        let job = canonical_job();
        let catalog = catalog(&job);
        let registry = registry();
        let (mut workspace, node) = workspace(&registry, &catalog);
        let retained = encode_graph_workspace(&workspace).unwrap();

        assert_eq!(
            catalog.entries()[0].typed_value(registry.context_schema(), BOOL),
            Err(GraphCachedJobCatalogError::NotJobHandleType(BOOL))
        );

        assert!(matches!(
            select_graph_cached_job_handle(&catalog, &registry, &mut workspace, node, 1, 0),
            Err(GraphCachedJobHandleSelectionError::AlreadySelected { .. })
        ));
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);
        assert!(matches!(
            select_graph_cached_job_handle(&catalog, &registry, &mut workspace, node, 2, 1),
            Err(GraphCachedJobHandleSelectionError::NotJobHandleParameter { .. })
        ));
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), retained);

        let raw = JobGraphHandle {
            global_job_digest: Digest([0x9a; 32]),
            ..catalog.entries()[0].handle()
        };
        workspace
            .set_parameter(
                node,
                1,
                TypedGraphValue::try_new(
                    registry.context_schema(),
                    JOB,
                    GraphValue::JobHandle(raw),
                )
                .unwrap(),
            )
            .unwrap();
        let raw_encoding = encode_graph_workspace(&workspace).unwrap();
        assert!(matches!(
            select_graph_cached_job_handle(&catalog, &registry, &mut workspace, node, 1, 1),
            Err(GraphCachedJobHandleSelectionError::ParameterNotCatalogBound { .. })
        ));
        assert_eq!(encode_graph_workspace(&workspace).unwrap(), raw_encoding);

        workspace
            .set_parameter(
                node,
                1,
                catalog.entries()[0]
                    .typed_value(registry.context_schema(), JOB)
                    .unwrap(),
            )
            .unwrap();
        let before_semantics = encode_graph_workspace(&workspace).unwrap();
        let context = GraphDocument::try_new(
            0,
            registry.context_schema().clone(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let empty_registry =
            GraphNodeRegistry::try_new(GraphAnalysisLimits::interactive(), &context, Vec::new())
                .unwrap();
        assert!(matches!(
            select_graph_cached_job_handle(&catalog, &empty_registry, &mut workspace, node, 1, 1,),
            Err(GraphCachedJobHandleSelectionError::Analysis(_))
        ));
        assert_eq!(
            encode_graph_workspace(&workspace).unwrap(),
            before_semantics
        );
    }
}
