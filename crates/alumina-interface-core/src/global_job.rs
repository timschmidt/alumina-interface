//! Authoritative global manifest construction over owned per-MCU cache artifacts.

use std::error::Error as StdError;
use std::fmt;

use alumina_job::{
    DecodedMachineJobManifest, JobNetworkPolicy, MachineJobGlobalFacts, MachineJobManifest,
    MachineJobManifestError, MachineJobParticipant,
};
use alumina_machine_ir::{MAX_EXECUTION_AXES, StreamTick};
use alumina_protocol::{DeviceId, Digest};
use alumina_storage::{
    CacheLimits, ChunkUploadHeader, ContentId, Error as StorageError, ManifestHasher, ObjectKind,
    PublishedObject, StoredObject, UploadId, UploadPlan, sha256,
};

use crate::compiler::CanonicalPathProgram2;
use crate::partition::{
    CanonicalMachinePartition2, MachinePartitionError, package_canonical_program,
    representative_partition_policy_for,
};

/// Result type for global multi-MCU job compilation.
pub type GlobalJobCompileResult<T> = Result<T, GlobalJobCompileError>;

/// Global identities plus storage policy for the shared manifest object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalJobCompilePolicy {
    global: MachineJobGlobalFacts,
    upload_id: UploadId,
    storage_chunk_bytes: u32,
    cache_limits: CacheLimits,
}

impl GlobalJobCompilePolicy {
    /// Retain global compile facts and validate the manifest upload budget.
    pub fn try_new(
        global: MachineJobGlobalFacts,
        upload_id: UploadId,
        storage_chunk_bytes: u32,
        cache_limits: CacheLimits,
    ) -> GlobalJobCompileResult<Self> {
        if upload_id.0 == 0 {
            return Err(GlobalJobCompileError::InvalidPolicy(
                "manifest upload identity must be nonzero",
            ));
        }
        if storage_chunk_bytes == 0
            || cache_limits.maximum_object_bytes == 0
            || cache_limits.maximum_chunk_bytes == 0
            || cache_limits.maximum_chunks == 0
            || storage_chunk_bytes > cache_limits.maximum_chunk_bytes
        {
            return Err(GlobalJobCompileError::InvalidPolicy(
                "manifest cache limits and chunk size must be nonzero and bounded",
            ));
        }
        Ok(Self {
            global,
            upload_id,
            storage_chunk_bytes,
            cache_limits,
        })
    }

    /// Return shared source/compiler/time/safety facts.
    pub const fn global(&self) -> MachineJobGlobalFacts {
        self.global
    }

    /// Return the default resumable manifest-upload transaction identity.
    pub const fn upload_id(&self) -> UploadId {
        self.upload_id
    }

    /// Return the exact independently hashed storage chunk size.
    pub const fn storage_chunk_bytes(&self) -> u32 {
        self.storage_chunk_bytes
    }

    /// Return the cache admission budget for the manifest object.
    pub const fn cache_limits(&self) -> CacheLimits {
        self.cache_limits
    }
}

/// One owned local partition plus the global-manifest facts not already
/// repeated in its canonical execution blocks.
#[derive(Debug)]
pub struct MachineJobParticipantPackage2 {
    device_id: DeviceId,
    board_package_digest: Digest,
    resource_set_digest: Digest,
    error_evidence_digest: Digest,
    safety_envelope_digest: Digest,
    partition: CanonicalMachinePartition2,
}

impl MachineJobParticipantPackage2 {
    /// Bind one compiled partition to its physical board and retained evidence.
    pub const fn new(
        device_id: DeviceId,
        board_package_digest: Digest,
        resource_set_digest: Digest,
        error_evidence_digest: Digest,
        safety_envelope_digest: Digest,
        partition: CanonicalMachinePartition2,
    ) -> Self {
        Self {
            device_id,
            board_package_digest,
            resource_set_digest,
            error_evidence_digest,
            safety_envelope_digest,
            partition,
        }
    }

    /// Return the stable physical MCU identity.
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Borrow the complete local cached partition artifact.
    pub const fn partition(&self) -> &CanonicalMachinePartition2 {
        &self.partition
    }
}

/// One content-addressed slice of the global manifest object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalGlobalManifestChunk {
    offset: usize,
    byte_len: u32,
    content: ContentId,
}

impl CanonicalGlobalManifestChunk {
    /// Return the byte offset in the complete manifest object.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Return the exact bytes in this storage chunk.
    pub const fn byte_len(&self) -> u32 {
        self.byte_len
    }

    /// Return the independently verified SHA-256 content identity.
    pub const fn content(&self) -> ContentId {
        self.content
    }
}

/// Complete global manifest plus every owned per-MCU partition it names.
#[derive(Debug)]
pub struct CanonicalGlobalJob2 {
    policy: GlobalJobCompilePolicy,
    participants: Vec<MachineJobParticipantPackage2>,
    participant_records: Vec<MachineJobParticipant>,
    manifest_bytes: Vec<u8>,
    manifest_chunks: Vec<CanonicalGlobalManifestChunk>,
    upload_plan: UploadPlan,
    publication: PublishedObject,
    participant_set_digest: Digest,
    global_job_digest: Digest,
}

impl CanonicalGlobalJob2 {
    /// Borrow global compile and cache policy.
    pub const fn policy(&self) -> &GlobalJobCompilePolicy {
        &self.policy
    }

    /// Borrow owned per-MCU packages in canonical stable-device order.
    pub fn participants(&self) -> &[MachineJobParticipantPackage2] {
        &self.participants
    }

    /// Borrow canonical participant records in the same order.
    pub fn participant_records(&self) -> &[MachineJobParticipant] {
        &self.participant_records
    }

    /// Borrow the exact global manifest bytes.
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }

    /// Borrow independently hashed global-manifest storage chunks.
    pub fn manifest_chunks(&self) -> &[CanonicalGlobalManifestChunk] {
        &self.manifest_chunks
    }

    /// Borrow one manifest storage chunk without copying.
    pub fn manifest_chunk_bytes(&self, index: usize) -> Option<&[u8]> {
        let chunk = self.manifest_chunks.get(index)?;
        let len = usize::try_from(chunk.byte_len).ok()?;
        self.manifest_bytes
            .get(chunk.offset..chunk.offset.checked_add(len)?)
    }

    /// Return the fixture/default `StorageBeginUpload` declaration.
    pub const fn upload_plan(&self) -> UploadPlan {
        self.upload_plan
    }

    /// Construct an equivalent upload declaration for another participant's
    /// independent storage transaction. The upload ID is not content authority.
    pub fn upload_plan_for(&self, upload_id: UploadId) -> GlobalJobCompileResult<UploadPlan> {
        let plan = UploadPlan {
            upload_id,
            object: self.publication.object,
            manifest: self.publication.manifest,
            chunk_bytes: self.upload_plan.chunk_bytes,
            chunk_count: self.upload_plan.chunk_count,
        };
        plan.validate(self.policy.cache_limits)?;
        Ok(plan)
    }

    /// Return one canonical manifest chunk prefix for a selected participant's
    /// upload transaction.
    pub fn manifest_chunk_upload_header(
        &self,
        upload_id: UploadId,
        index: usize,
    ) -> GlobalJobCompileResult<Option<ChunkUploadHeader>> {
        self.upload_plan_for(upload_id)?;
        let Some(chunk) = self.manifest_chunks.get(index).copied() else {
            return Ok(None);
        };
        Ok(Some(ChunkUploadHeader {
            upload_id,
            index: u32::try_from(index).map_err(|_| GlobalJobCompileError::CounterOverflow)?,
            byte_len: chunk.byte_len,
            content: chunk.content,
        }))
    }

    /// Return the typed object/ordered-chunk-manifest publication.
    pub const fn publication(&self) -> PublishedObject {
        self.publication
    }

    /// Return SHA-256 of complete ordered participant records.
    pub const fn participant_set_digest(&self) -> Digest {
        self.participant_set_digest
    }

    /// Return SHA-256 of the complete canonical global manifest bytes.
    pub const fn global_job_digest(&self) -> Digest {
        self.global_job_digest
    }
}

/// Construct a canonical global manifest and retain every local partition it
/// names. Discovery order is erased by stable `DeviceId` sorting.
pub fn compile_global_job(
    policy: GlobalJobCompilePolicy,
    mut participants: Vec<MachineJobParticipantPackage2>,
) -> GlobalJobCompileResult<CanonicalGlobalJob2> {
    participants.sort_unstable_by_key(MachineJobParticipantPackage2::device_id);

    let mut participant_records = Vec::new();
    participant_records
        .try_reserve_exact(participants.len())
        .map_err(|_| GlobalJobCompileError::AllocationOverflow)?;
    for participant in &participants {
        participant_records.push(participant_record(participant));
    }

    let manifest = MachineJobManifest::new(policy.global, &participant_records)?;
    let wire_len = manifest.wire_len()?;
    let participant_set_digest = manifest.participant_set_digest();
    let global_job_digest = manifest.global_job_digest()?;
    let mut manifest_bytes = Vec::new();
    manifest_bytes
        .try_reserve_exact(wire_len)
        .map_err(|_| GlobalJobCompileError::AllocationOverflow)?;
    manifest_bytes.resize(wire_len, 0);
    manifest.encode_into(&mut manifest_bytes)?;
    if sha256(&manifest_bytes).digest != global_job_digest {
        return Err(GlobalJobCompileError::DigestMismatch);
    }

    let decoded = DecodedMachineJobManifest::decode(&manifest_bytes)?;
    if decoded.global() != policy.global
        || decoded.participant_count() != participant_records.len()
        || decoded.participant_set_digest() != participant_set_digest
        || decoded.global_job_digest() != global_job_digest
    {
        return Err(GlobalJobCompileError::ReplayMismatch);
    }
    for (index, expected) in participant_records.iter().copied().enumerate() {
        if decoded.participant(index)? != Some(expected) {
            return Err(GlobalJobCompileError::ReplayMismatch);
        }
    }

    let object = StoredObject {
        kind: ObjectKind::MachineJobManifest,
        content: sha256(&manifest_bytes),
        byte_len: u64::try_from(manifest_bytes.len())
            .map_err(|_| GlobalJobCompileError::CounterOverflow)?,
    };
    if object.content.digest != global_job_digest {
        return Err(GlobalJobCompileError::DigestMismatch);
    }
    let chunk_bytes = usize::try_from(policy.storage_chunk_bytes)
        .map_err(|_| GlobalJobCompileError::CounterOverflow)?;
    let chunk_count = u32::try_from(manifest_bytes.len().div_ceil(chunk_bytes))
        .map_err(|_| GlobalJobCompileError::CounterOverflow)?;
    let mut manifest_hasher = ManifestHasher::new(
        object,
        policy.storage_chunk_bytes,
        chunk_count,
        policy.cache_limits,
    )?;
    let mut manifest_chunks = Vec::new();
    manifest_chunks
        .try_reserve_exact(
            usize::try_from(chunk_count).map_err(|_| GlobalJobCompileError::CounterOverflow)?,
        )
        .map_err(|_| GlobalJobCompileError::AllocationOverflow)?;
    let mut offset = 0_usize;
    for (index, chunk) in manifest_bytes.chunks(chunk_bytes).enumerate() {
        let index = u32::try_from(index).map_err(|_| GlobalJobCompileError::CounterOverflow)?;
        let byte_len =
            u32::try_from(chunk.len()).map_err(|_| GlobalJobCompileError::CounterOverflow)?;
        let content = sha256(chunk);
        manifest_hasher.push(index, content, byte_len)?;
        manifest_chunks.push(CanonicalGlobalManifestChunk {
            offset,
            byte_len,
            content,
        });
        offset = offset
            .checked_add(chunk.len())
            .ok_or(GlobalJobCompileError::CounterOverflow)?;
    }
    let chunk_manifest = manifest_hasher.finalize()?;
    let upload_plan = UploadPlan {
        upload_id: policy.upload_id,
        object,
        manifest: chunk_manifest,
        chunk_bytes: policy.storage_chunk_bytes,
        chunk_count,
    };
    upload_plan.validate(policy.cache_limits)?;
    let publication = PublishedObject {
        object,
        manifest: chunk_manifest,
    };

    Ok(CanonicalGlobalJob2 {
        policy,
        participants,
        participant_records,
        manifest_bytes,
        manifest_chunks,
        upload_plan,
        publication,
        participant_set_digest,
        global_job_digest,
    })
}

fn participant_record(package: &MachineJobParticipantPackage2) -> MachineJobParticipant {
    let partition = &package.partition;
    let mut initial_position = [0_i64; MAX_EXECUTION_AXES];
    let mut final_position = [0_i64; MAX_EXECUTION_AXES];
    initial_position[..2].copy_from_slice(&partition.initial_position());
    final_position[..2].copy_from_slice(&partition.final_position());
    MachineJobParticipant {
        device_id: package.device_id,
        stream_id: partition.policy().stream_id(),
        board_package_digest: package.board_package_digest,
        capability_digest: partition.policy().capability_digest(),
        config_digest: partition.policy().config_digest(),
        partition_digest: partition.publication().object.content.digest,
        partition_manifest_digest: partition.publication().manifest.digest,
        terminal_block_digest: partition.terminal_progress().block_digest,
        resource_set_digest: package.resource_set_digest,
        error_evidence_digest: package.error_evidence_digest,
        safety_envelope_digest: package.safety_envelope_digest,
        partition_byte_len: partition.publication().object.byte_len,
        block_count: partition.block_count(),
        axis_count: 2,
        local_timer_hz: partition.local_timer_hz(),
        first_tick: StreamTick(0),
        end_tick: partition.terminal_progress().end_tick,
        initial_position,
        final_position,
    }
}

/// Compile the deterministic two-participant cached-job fixture.
pub fn compile_representative_global_job(
    program: &CanonicalPathProgram2,
) -> GlobalJobCompileResult<CanonicalGlobalJob2> {
    let first = package_canonical_program(program, representative_partition_policy_for(1)?)?;
    let second = package_canonical_program(program, representative_partition_policy_for(2)?)?;
    let end_tick = program
        .segments()
        .last()
        .ok_or(GlobalJobCompileError::InvalidPolicy(
            "representative program must contain a terminal segment",
        ))?
        .end_tick
        .0;
    let global = MachineJobGlobalFacts {
        network_policy: JobNetworkPolicy::NetworkAttended,
        global_timebase_hz: program.policy().timer_ticks_per_second(),
        duration_ticks: end_tick,
        source_digest: Digest([0x41; 32]),
        compiler_digest: Digest([0x42; 32]),
        interface_digest: Digest([0x43; 32]),
        policy_digest: Digest([0x44; 32]),
        machine_digest: Digest([0x45; 32]),
        coordinate_epoch_digest: Digest([0x46; 32]),
        safety_policy_digest: Digest([0x47; 32]),
        synchronization_digest: Digest([0x48; 32]),
    };
    let policy = GlobalJobCompilePolicy::try_new(
        global,
        UploadId(0x2122_2324_2526_2728),
        700,
        CacheLimits {
            maximum_object_bytes: 4 * 1024 * 1024,
            maximum_chunk_bytes: 1_024,
            maximum_chunks: 10_000,
        },
    )?;
    let first = MachineJobParticipantPackage2::new(
        DeviceId([1; 16]),
        Digest([0x61; 32]),
        Digest([0x71; 32]),
        Digest([0x81; 32]),
        Digest([0x91; 32]),
        first,
    );
    let second = MachineJobParticipantPackage2::new(
        DeviceId([2; 16]),
        Digest([0x62; 32]),
        Digest([0x72; 32]),
        Digest([0x82; 32]),
        Digest([0x92; 32]),
        second,
    );
    // Deliberately reverse discovery order; canonical compilation sorts by ID.
    compile_global_job(policy, vec![second, first])
}

/// Failure while binding local cached partitions into a global job.
#[derive(Debug)]
pub enum GlobalJobCompileError {
    /// One global storage or fixture policy was invalid.
    InvalidPolicy(&'static str),
    /// Manifest or participant storage could not be reserved.
    AllocationOverflow,
    /// A canonical length/index field overflowed.
    CounterOverflow,
    /// Global manifest bytes did not hash to the streaming canonical identity.
    DigestMismatch,
    /// Independent manifest decode did not reproduce compiler facts.
    ReplayMismatch,
    /// One local partition failed before global binding.
    Partition(MachinePartitionError),
    /// The shared firmware manifest schema rejected global or participant facts.
    Manifest(MachineJobManifestError),
    /// The shared storage schema rejected the manifest object or chunk layout.
    Storage(StorageError),
}

impl fmt::Display for GlobalJobCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy(reason) => write!(formatter, "invalid global job policy: {reason}"),
            Self::AllocationOverflow => {
                formatter.write_str("global job storage could not be reserved")
            }
            Self::CounterOverflow => {
                formatter.write_str("global job counter overflowed its canonical field")
            }
            Self::DigestMismatch => {
                formatter.write_str("global manifest streaming and byte digests diverged")
            }
            Self::ReplayMismatch => {
                formatter.write_str("decoded global manifest diverged from compiler facts")
            }
            Self::Partition(source) => write!(formatter, "participant partition failed: {source}"),
            Self::Manifest(source) => write!(formatter, "global manifest rejected: {source:?}"),
            Self::Storage(source) => {
                write!(formatter, "global manifest storage rejected: {source:?}")
            }
        }
    }
}

impl StdError for GlobalJobCompileError {}

impl From<MachinePartitionError> for GlobalJobCompileError {
    fn from(value: MachinePartitionError) -> Self {
        Self::Partition(value)
    }
}

impl From<MachineJobManifestError> for GlobalJobCompileError {
    fn from(value: MachineJobManifestError) -> Self {
        Self::Manifest(value)
    }
}

impl From<StorageError> for GlobalJobCompileError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

#[cfg(test)]
mod tests {
    use alumina_storage::{MutationContext, UploadCoordinator};

    use super::*;
    use crate::compiler::compile_representative_program;

    #[test]
    fn representative_global_job_is_sorted_deterministic_and_real_schema() {
        let program = compile_representative_program().unwrap();
        let first = compile_representative_global_job(&program).unwrap();
        let second = compile_representative_global_job(&program).unwrap();

        assert_eq!(first.manifest_bytes(), second.manifest_bytes());
        assert_eq!(first.manifest_chunks(), second.manifest_chunks());
        assert_eq!(first.upload_plan(), second.upload_plan());
        assert_eq!(first.publication(), second.publication());
        assert_eq!(first.participants().len(), 2);
        assert_eq!(first.participants()[0].device_id(), DeviceId([1; 16]));
        assert_eq!(first.participants()[1].device_id(), DeviceId([2; 16]));
        assert_eq!(first.manifest_bytes().len(), 1_312);
        assert_eq!(first.manifest_chunks().len(), 2);
        assert_eq!(
            first.global_job_digest(),
            first.publication().object.content.digest
        );

        let decoded = DecodedMachineJobManifest::decode(first.manifest_bytes()).unwrap();
        assert_eq!(decoded.participant_count(), 2);
        assert_eq!(
            decoded.participant_set_digest(),
            first.participant_set_digest()
        );
        assert_eq!(decoded.global_job_digest(), first.global_job_digest());
    }

    #[test]
    fn manifest_upload_replays_for_independent_participant_transactions() {
        let program = compile_representative_program().unwrap();
        let job = compile_representative_global_job(&program).unwrap();
        let upload_id = UploadId(0x3132_3334_3536_3738);
        let plan = job.upload_plan_for(upload_id).unwrap();
        let mut upload = UploadCoordinator::new();
        upload
            .begin(
                plan,
                job.policy().cache_limits(),
                MutationContext::DISARMED_IDLE,
            )
            .unwrap();
        for index in 0..job.manifest_chunks().len() {
            let header = job
                .manifest_chunk_upload_header(upload_id, index)
                .unwrap()
                .unwrap();
            let bytes = job.manifest_chunk_bytes(index).unwrap();
            let verified = upload
                .verify_chunk(header.upload_id, header.index, header.content, bytes)
                .unwrap();
            upload
                .record_chunk(verified, MutationContext::DISARMED_IDLE)
                .unwrap();
        }
        let publication = upload
            .finalize(
                upload_id,
                job.publication().object.content,
                job.publication().manifest,
                MutationContext::DISARMED_IDLE,
            )
            .unwrap();
        assert_eq!(
            upload
                .record_published(publication, MutationContext::DISARMED_IDLE)
                .unwrap(),
            job.publication()
        );
    }

    #[test]
    fn global_duration_mismatch_rejects_all_artifacts() {
        let program = compile_representative_program().unwrap();
        let partition =
            package_canonical_program(&program, representative_partition_policy_for(1).unwrap())
                .unwrap();
        let package = MachineJobParticipantPackage2::new(
            DeviceId([1; 16]),
            Digest([0x61; 32]),
            Digest([0x71; 32]),
            Digest([0x81; 32]),
            Digest([0x91; 32]),
            partition,
        );
        let mut global = compile_representative_global_job(&program)
            .unwrap()
            .policy()
            .global();
        global.duration_ticks -= 1;
        let policy = GlobalJobCompilePolicy::try_new(
            global,
            UploadId(1),
            700,
            CacheLimits {
                maximum_object_bytes: 4 * 1024 * 1024,
                maximum_chunk_bytes: 1_024,
                maximum_chunks: 10_000,
            },
        )
        .unwrap();
        assert!(matches!(
            compile_global_job(policy, vec![package]),
            Err(GlobalJobCompileError::Manifest(
                MachineJobManifestError::DurationMismatch { index: 0 }
            ))
        ));
    }
}
