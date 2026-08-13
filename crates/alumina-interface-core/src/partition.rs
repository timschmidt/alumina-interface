//! Canonical per-MCU block partitioning and content-addressed cache artifacts.
//!
//! This module begins only after exact geometry has become canonical integer
//! machine segments. It uses the firmware workspace's real block, storage, and
//! job schemas, then independently decodes and validates the complete result.
//! No renderer value or duplicate wire representation exists here.

use std::error::Error as StdError;
use std::fmt;

use alumina_job::{
    DescriptorError, JOB_DESCRIPTOR_WIRE_BYTES, JobDescriptor, JobDescriptorWireError,
};
use alumina_machine_ir::{
    BlockError, BlockExpectation, BlockValidationLimits, EXECUTION_BLOCK_BYTES, ExecutionBlock,
    MAX_EXECUTION_AXES, MotionStreamProgress, MotionStreamValidator, StreamId, StreamTick,
    ValidationLimits, maximum_motion_segments_per_block,
};
use alumina_protocol::Digest;
use alumina_storage::{
    CacheLimits, ChunkUploadHeader, ContentId, Error as StorageError, ManifestHasher, ObjectKind,
    PublishedObject, StoredObject, UploadId, UploadPlan, sha256,
};

use crate::compiler::CanonicalPathProgram2;
use crate::motion_schedule::CanonicalScheduledProgram2;

/// Result type for canonical per-MCU packaging.
pub type MachinePartitionResult<T> = Result<T, MachinePartitionError>;

/// Exact identities and bounded firmware/storage limits for one MCU partition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MachinePartitionPolicy2 {
    stream_id: StreamId,
    capability_digest: Digest,
    config_digest: Digest,
    block_limits: BlockValidationLimits,
    upload_id: UploadId,
    storage_chunk_bytes: u32,
    cache_limits: CacheLimits,
}

impl MachinePartitionPolicy2 {
    /// Validates every identity and nonzero firmware/storage bound before bytes
    /// can be allocated or hashed.
    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is an independent canonical policy fact"
    )]
    pub fn try_new(
        stream_id: [u8; 16],
        capability_digest: Digest,
        config_digest: Digest,
        block_limits: BlockValidationLimits,
        upload_id: UploadId,
        storage_chunk_bytes: u32,
        cache_limits: CacheLimits,
    ) -> MachinePartitionResult<Self> {
        let stream_id = StreamId::new(stream_id).map_err(MachinePartitionError::Machine)?;
        if capability_digest.is_zero() {
            return Err(MachinePartitionError::InvalidPolicy(
                "capability digest must be nonzero",
            ));
        }
        if config_digest.is_zero() {
            return Err(MachinePartitionError::InvalidPolicy(
                "configuration digest must be nonzero",
            ));
        }
        if block_limits.maximum_block_ticks == 0
            || block_limits.segment.maximum_segment_ticks == 0
            || block_limits.segment.maximum_steps_per_segment == 0
        {
            return Err(MachinePartitionError::InvalidPolicy(
                "all block and segment limits must be nonzero",
            ));
        }
        if upload_id.0 == 0 {
            return Err(MachinePartitionError::InvalidPolicy(
                "upload identity must be nonzero",
            ));
        }
        if cache_limits.maximum_object_bytes == 0
            || cache_limits.maximum_chunk_bytes == 0
            || cache_limits.maximum_chunks == 0
        {
            return Err(MachinePartitionError::InvalidPolicy(
                "all cache limits must be nonzero",
            ));
        }
        if storage_chunk_bytes == 0 || storage_chunk_bytes > cache_limits.maximum_chunk_bytes {
            return Err(MachinePartitionError::InvalidPolicy(
                "storage chunk bytes exceed the cache policy",
            ));
        }
        Ok(Self {
            stream_id,
            capability_digest,
            config_digest,
            block_limits,
            upload_id,
            storage_chunk_bytes,
            cache_limits,
        })
    }

    /// Return the per-partition stream identity repeated by every block.
    pub const fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Return the exact board capability identity used by the compiler.
    pub const fn capability_digest(&self) -> Digest {
        self.capability_digest
    }

    /// Return the exact active machine-configuration identity.
    pub const fn config_digest(&self) -> Digest {
        self.config_digest
    }

    /// Return the firmware's block/segment admission bounds.
    pub const fn block_limits(&self) -> BlockValidationLimits {
        self.block_limits
    }

    /// Return the resumable storage transaction identity.
    pub const fn upload_id(&self) -> UploadId {
        self.upload_id
    }

    /// Return the exact independently hashed storage chunk size.
    pub const fn storage_chunk_bytes(&self) -> u32 {
        self.storage_chunk_bytes
    }

    /// Return the device cache admission budget used to construct the upload.
    pub const fn cache_limits(&self) -> CacheLimits {
        self.cache_limits
    }
}

/// One content-addressed slice of the immutable partition object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalPartitionChunk {
    offset: usize,
    byte_len: u32,
    content: ContentId,
}

impl CanonicalPartitionChunk {
    /// Return the byte offset in the complete partition object.
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

/// Complete immutable cache artifact for one two-axis MCU stream.
#[derive(Debug)]
pub struct CanonicalMachinePartition2 {
    policy: MachinePartitionPolicy2,
    bytes: Vec<u8>,
    chunks: Vec<CanonicalPartitionChunk>,
    upload_plan: UploadPlan,
    publication: PublishedObject,
    block_count: u32,
    maximum_segments_per_block: usize,
    maximum_observed_block_ticks: u64,
    local_timer_hz: u64,
    initial_position: [i64; 2],
    final_position: [i64; 2],
    terminal_progress: MotionStreamProgress<2>,
}

impl CanonicalMachinePartition2 {
    /// Borrow the exact identities and bounded board/cache policy.
    pub const fn policy(&self) -> &MachinePartitionPolicy2 {
        &self.policy
    }

    /// Borrow the byte-identical concatenation of canonical 512-byte blocks.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Borrow independently hashed chunks in upload order.
    pub fn chunks(&self) -> &[CanonicalPartitionChunk] {
        &self.chunks
    }

    /// Borrow exactly one chunk's bytes without copying.
    pub fn chunk_bytes(&self, index: usize) -> Option<&[u8]> {
        let chunk = self.chunks.get(index)?;
        let len = usize::try_from(chunk.byte_len).ok()?;
        self.bytes.get(chunk.offset..chunk.offset.checked_add(len)?)
    }

    /// Return the validated `StorageBeginUpload` declaration.
    pub const fn upload_plan(&self) -> UploadPlan {
        self.upload_plan
    }

    /// Return one canonical storage chunk prefix for the retained transaction.
    pub fn chunk_upload_header(&self, index: usize) -> Option<ChunkUploadHeader> {
        let chunk = *self.chunks.get(index)?;
        Some(ChunkUploadHeader {
            upload_id: self.policy.upload_id,
            index: u32::try_from(index).ok()?,
            byte_len: chunk.byte_len,
            content: chunk.content,
        })
    }

    /// Return the object/manifest identity expected after atomic publication.
    pub const fn publication(&self) -> PublishedObject {
        self.publication
    }

    /// Return the number of independently owned execution blocks.
    pub const fn block_count(&self) -> u32 {
        self.block_count
    }

    /// Return the canonical record capacity queried from `alumina-machine-ir`.
    pub const fn maximum_segments_per_block(&self) -> usize {
        self.maximum_segments_per_block
    }

    /// Return the longest block horizon observed in this partition.
    pub const fn maximum_observed_block_ticks(&self) -> u64 {
        self.maximum_observed_block_ticks
    }

    /// Return the exact timer frequency used to compile stream-relative ticks.
    pub const fn local_timer_hz(&self) -> u64 {
        self.local_timer_hz
    }

    /// Return the exact absolute machine-lattice start position.
    pub const fn initial_position(&self) -> [i64; 2] {
        self.initial_position
    }

    /// Return the exact absolute machine-lattice terminal position.
    pub const fn final_position(&self) -> [i64; 2] {
        self.final_position
    }

    /// Return the independently replayed terminal chain/tick/displacement facts.
    pub const fn terminal_progress(&self) -> MotionStreamProgress<2> {
        self.terminal_progress
    }

    /// Construct the real firmware `JobPrepare` descriptor only after a live
    /// boot-local correlation has been selected.
    pub fn job_descriptor(&self, prepare_id: u64) -> MachinePartitionResult<JobDescriptor> {
        let mut initial_position = [0_i64; MAX_EXECUTION_AXES];
        initial_position[..2].copy_from_slice(&self.initial_position);
        let descriptor = JobDescriptor {
            prepare_id,
            partition: self.publication,
            stream_id: self.policy.stream_id,
            capability_digest: self.policy.capability_digest,
            config_digest: self.policy.config_digest,
            axis_count: 2,
            block_count: self.block_count,
            first_tick: StreamTick(0),
            initial_position,
            limits: self.policy.block_limits,
        };
        descriptor
            .validate::<2>()
            .map_err(MachinePartitionError::Descriptor)?;
        Ok(descriptor)
    }

    /// Encode the exact body sent by a later authenticated `JobPrepare` call.
    pub fn job_prepare_body(
        &self,
        prepare_id: u64,
    ) -> MachinePartitionResult<[u8; JOB_DESCRIPTOR_WIRE_BYTES]> {
        self.job_descriptor(prepare_id)?
            .encode::<2>()
            .map_err(MachinePartitionError::DescriptorWire)
    }
}

/// Partition one certified canonical program into independently admitted
/// blocks, storage chunks, and a resumable content-addressed upload.
pub fn package_canonical_program(
    program: &CanonicalPathProgram2,
    policy: MachinePartitionPolicy2,
) -> MachinePartitionResult<CanonicalMachinePartition2> {
    if program
        .policy()
        .machine_configuration_digest()
        .is_some_and(|digest| digest != policy.config_digest)
        || program
            .policy()
            .capability_digest()
            .is_some_and(|digest| digest != policy.capability_digest)
    {
        return Err(MachinePartitionError::ProgramIdentityMismatch);
    }
    let first_point = program
        .points()
        .first()
        .ok_or(MachinePartitionError::EmptyProgram)?;
    let last_point = program
        .points()
        .last()
        .ok_or(MachinePartitionError::EmptyProgram)?;
    package_canonical_segments(
        program.segments(),
        [first_point.steps()[0].get(), first_point.steps()[1].get()],
        [last_point.steps()[0].get(), last_point.steps()[1].get()],
        program.policy().timer_ticks_per_second(),
        policy,
    )
}

/// Package a machine-bound, jerk-schedule-derived program only when its
/// configuration and board identities exactly match the target partition.
pub fn package_canonical_scheduled_program(
    program: &CanonicalScheduledProgram2,
    policy: MachinePartitionPolicy2,
) -> MachinePartitionResult<CanonicalMachinePartition2> {
    if program.configuration_digest() != policy.config_digest
        || program.capability_digest() != policy.capability_digest
    {
        return Err(MachinePartitionError::ProgramIdentityMismatch);
    }
    let first_point = program
        .points()
        .first()
        .ok_or(MachinePartitionError::EmptyProgram)?;
    let last_point = program
        .points()
        .last()
        .ok_or(MachinePartitionError::EmptyProgram)?;
    let initial_position = [first_point.steps()[0].get(), first_point.steps()[1].get()];
    let final_position = [last_point.steps()[0].get(), last_point.steps()[1].get()];
    let preflight = program.executor_preflight();
    if preflight.position != final_position
        || preflight.end_tick
            != program
                .segments()
                .last()
                .ok_or(MachinePartitionError::EmptyProgram)?
                .end_tick
    {
        return Err(MachinePartitionError::TerminalMismatch);
    }
    package_canonical_segments(
        program.segments(),
        initial_position,
        final_position,
        program.timer_ticks_per_second(),
        policy,
    )
}

fn package_canonical_segments(
    segments: &[alumina_machine_ir::ExecutionSegment<2>],
    initial_position: [i64; 2],
    expected_final: [i64; 2],
    local_timer_hz: u64,
    policy: MachinePartitionPolicy2,
) -> MachinePartitionResult<CanonicalMachinePartition2> {
    let first = segments
        .first()
        .ok_or(MachinePartitionError::EmptyProgram)?;
    if first.start_tick != StreamTick(0) {
        return Err(MachinePartitionError::ProgramMustStartAtZero);
    }

    let maximum_segments_per_block = maximum_motion_segments_per_block::<2>()?;
    if maximum_segments_per_block == 0 {
        return Err(MachinePartitionError::Machine(BlockError::PayloadLength));
    }

    let mut bytes = Vec::new();
    let mut cursor = 0_usize;
    let mut sequence = 0_u32;
    let mut previous_digest = Digest::ZERO;
    let mut maximum_observed_block_ticks = 0_u64;
    while cursor < segments.len() {
        let block_start_tick = segments[cursor].start_tick;
        let mut end = cursor;
        while end < segments.len() && end - cursor < maximum_segments_per_block {
            let candidate_ticks = segments[end]
                .end_tick
                .0
                .checked_sub(block_start_tick.0)
                .ok_or(MachinePartitionError::ProgramTiming { segment: end })?;
            if end > cursor && candidate_ticks > policy.block_limits.maximum_block_ticks {
                break;
            }
            end += 1;
            if candidate_ticks > policy.block_limits.maximum_block_ticks {
                break;
            }
        }

        let block = ExecutionBlock::encode_motion(
            policy.stream_id,
            policy.capability_digest,
            policy.config_digest,
            sequence,
            previous_digest,
            &segments[cursor..end],
        )?;
        let header = block.header();
        let block_ticks = header
            .end_tick
            .0
            .checked_sub(header.start_tick.0)
            .ok_or(MachinePartitionError::ProgramTiming { segment: cursor })?;
        maximum_observed_block_ticks = maximum_observed_block_ticks.max(block_ticks);
        bytes
            .try_reserve_exact(EXECUTION_BLOCK_BYTES)
            .map_err(|_| MachinePartitionError::AllocationOverflow)?;
        bytes.extend_from_slice(block.as_bytes());
        previous_digest = header.block_digest;
        sequence = sequence
            .checked_add(1)
            .ok_or(MachinePartitionError::Machine(BlockError::SequenceOverflow))?;
        cursor = end;
    }
    let block_count = sequence;

    let mut validator = MotionStreamValidator::<2>::new(
        block_count,
        BlockExpectation {
            stream_id: policy.stream_id,
            capability_digest: policy.capability_digest,
            config_digest: policy.config_digest,
            sequence: 0,
            start_tick: StreamTick(0),
            previous_digest: Digest::ZERO,
        },
        policy.block_limits,
    )?;
    for encoded in bytes.chunks_exact(EXECUTION_BLOCK_BYTES) {
        let mut owned = [0_u8; EXECUTION_BLOCK_BYTES];
        owned.copy_from_slice(encoded);
        let block = ExecutionBlock::decode(owned)?;
        validator.accept(&block)?;
    }
    let terminal_progress = validator.finish()?;

    let final_position = [
        initial_position[0]
            .checked_add(terminal_progress.position[0])
            .ok_or(MachinePartitionError::TerminalMismatch)?,
        initial_position[1]
            .checked_add(terminal_progress.position[1])
            .ok_or(MachinePartitionError::TerminalMismatch)?,
    ];
    let expected_end_tick = segments
        .last()
        .ok_or(MachinePartitionError::EmptyProgram)?
        .end_tick;
    if final_position != expected_final
        || terminal_progress.end_tick != expected_end_tick
        || terminal_progress.block_digest != previous_digest
    {
        return Err(MachinePartitionError::TerminalMismatch);
    }

    let object = StoredObject {
        kind: ObjectKind::MachineJobPartition,
        content: sha256(&bytes),
        byte_len: u64::try_from(bytes.len()).map_err(|_| MachinePartitionError::CounterOverflow)?,
    };
    let chunk_bytes = usize::try_from(policy.storage_chunk_bytes)
        .map_err(|_| MachinePartitionError::CounterOverflow)?;
    let chunk_count = u32::try_from(bytes.len().div_ceil(chunk_bytes))
        .map_err(|_| MachinePartitionError::CounterOverflow)?;
    let mut manifest = ManifestHasher::new(
        object,
        policy.storage_chunk_bytes,
        chunk_count,
        policy.cache_limits,
    )?;
    let mut chunks = Vec::new();
    chunks
        .try_reserve_exact(
            usize::try_from(chunk_count).map_err(|_| MachinePartitionError::CounterOverflow)?,
        )
        .map_err(|_| MachinePartitionError::AllocationOverflow)?;
    let mut offset = 0_usize;
    for (index, chunk) in bytes.chunks(chunk_bytes).enumerate() {
        let index = u32::try_from(index).map_err(|_| MachinePartitionError::CounterOverflow)?;
        let byte_len =
            u32::try_from(chunk.len()).map_err(|_| MachinePartitionError::CounterOverflow)?;
        let content = sha256(chunk);
        manifest.push(index, content, byte_len)?;
        chunks.push(CanonicalPartitionChunk {
            offset,
            byte_len,
            content,
        });
        offset = offset
            .checked_add(chunk.len())
            .ok_or(MachinePartitionError::CounterOverflow)?;
    }
    let manifest = manifest.finalize()?;
    let upload_plan = UploadPlan {
        upload_id: policy.upload_id,
        object,
        manifest,
        chunk_bytes: policy.storage_chunk_bytes,
        chunk_count,
    };
    upload_plan.validate(policy.cache_limits)?;
    let publication = PublishedObject { object, manifest };

    Ok(CanonicalMachinePartition2 {
        policy,
        bytes,
        chunks,
        upload_plan,
        publication,
        block_count,
        maximum_segments_per_block,
        maximum_observed_block_ticks,
        local_timer_hz,
        initial_position,
        final_position,
        terminal_progress,
    })
}

/// Construct the deterministic cache/block policy used by the window-free
/// line/arc/cubic checkpoint.
///
/// These identities are fixtures, not discovered board/configuration facts.
/// Production callers must replace them with authenticated capability and
/// active-configuration identities before compiling executable work.
pub fn representative_partition_policy() -> MachinePartitionResult<MachinePartitionPolicy2> {
    representative_partition_policy_for(1)
}

/// Construct a distinct deterministic participant policy for global-job
/// fixtures. Participant numbers begin at one.
pub fn representative_partition_policy_for(
    participant: u8,
) -> MachinePartitionResult<MachinePartitionPolicy2> {
    if participant == 0 {
        return Err(MachinePartitionError::InvalidPolicy(
            "fixture participant number must be nonzero",
        ));
    }
    let stream = 0x10_u8
        .checked_add(participant)
        .ok_or(MachinePartitionError::CounterOverflow)?;
    let capability = 0x21_u8
        .checked_add(participant)
        .ok_or(MachinePartitionError::CounterOverflow)?;
    let config = 0x32_u8
        .checked_add(participant)
        .ok_or(MachinePartitionError::CounterOverflow)?;
    let upload_id = 0x0102_0304_0506_0708_u64
        .checked_add(u64::from(participant - 1))
        .ok_or(MachinePartitionError::CounterOverflow)?;
    MachinePartitionPolicy2::try_new(
        [stream; 16],
        Digest([capability; 32]),
        Digest([config; 32]),
        BlockValidationLimits {
            maximum_block_ticks: 450_000,
            segment: ValidationLimits {
                maximum_segment_ticks: 450_000,
                maximum_steps_per_segment: 1_000,
            },
        },
        UploadId(upload_id),
        700,
        CacheLimits {
            maximum_object_bytes: 4 * 1024 * 1024,
            maximum_chunk_bytes: 1_024,
            maximum_chunks: 10_000,
        },
    )
}

/// Failure while turning canonical segments into a cached firmware partition.
#[derive(Debug)]
pub enum MachinePartitionError {
    /// One identity, admission bound, or storage bound was invalid.
    InvalidPolicy(&'static str),
    /// Program configuration/capability identity did not match the partition target.
    ProgramIdentityMismatch,
    /// No canonical segment was available to package.
    EmptyProgram,
    /// Full cached partitions begin at stream-relative tick zero.
    ProgramMustStartAtZero,
    /// A canonical segment had reversed or unrepresentable timing.
    ProgramTiming {
        /// Zero-based canonical segment index.
        segment: usize,
    },
    /// Allocation or reservation could not represent the partition.
    AllocationOverflow,
    /// A byte/chunk/block counter could not fit the canonical representation.
    CounterOverflow,
    /// Independent block replay did not reproduce source terminal facts.
    TerminalMismatch,
    /// The canonical execution-block schema rejected construction or replay.
    Machine(BlockError),
    /// The canonical storage schema rejected object or manifest construction.
    Storage(StorageError),
    /// The runtime job descriptor rejected compiled identities or limits.
    Descriptor(DescriptorError),
    /// The canonical `JobPrepare` body could not be encoded.
    DescriptorWire(JobDescriptorWireError),
}

impl fmt::Display for MachinePartitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy(reason) => write!(formatter, "invalid partition policy: {reason}"),
            Self::ProgramIdentityMismatch => formatter
                .write_str("canonical program identity does not match the partition target"),
            Self::EmptyProgram => formatter.write_str("canonical program contains no segments"),
            Self::ProgramMustStartAtZero => {
                formatter.write_str("cached partition must begin at stream tick zero")
            }
            Self::ProgramTiming { segment } => {
                write!(formatter, "canonical segment {segment} has invalid timing")
            }
            Self::AllocationOverflow => {
                formatter.write_str("partition storage could not be reserved")
            }
            Self::CounterOverflow => {
                formatter.write_str("partition counter does not fit its canonical field")
            }
            Self::TerminalMismatch => formatter.write_str(
                "independent partition replay diverged from canonical program terminal facts",
            ),
            Self::Machine(source) => write!(formatter, "machine block rejected: {source:?}"),
            Self::Storage(source) => write!(formatter, "storage artifact rejected: {source:?}"),
            Self::Descriptor(source) => {
                write!(formatter, "job descriptor rejected: {source:?}")
            }
            Self::DescriptorWire(source) => {
                write!(formatter, "job descriptor encoding failed: {source:?}")
            }
        }
    }
}

impl StdError for MachinePartitionError {}

impl From<BlockError> for MachinePartitionError {
    fn from(value: BlockError) -> Self {
        Self::Machine(value)
    }
}

impl From<StorageError> for MachinePartitionError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

#[cfg(test)]
mod tests {
    use alumina_job::JobDescriptor;
    use alumina_machine_ir::PartitionAssembler;
    use alumina_storage::{MutationContext, UploadCoordinator, UploadPlan};

    use super::*;
    use crate::compiler::compile_representative_program;

    fn policy() -> MachinePartitionPolicy2 {
        representative_partition_policy().unwrap()
    }

    #[test]
    fn representative_program_packages_deterministically_into_real_schemas() {
        let program = compile_representative_program().unwrap();
        let first = package_canonical_program(&program, policy()).unwrap();
        let second = package_canonical_program(&program, policy()).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.chunks(), second.chunks());
        assert_eq!(first.upload_plan(), second.upload_plan());
        assert_eq!(first.publication(), second.publication());
        assert_eq!(first.maximum_segments_per_block(), 10);
        assert_eq!(
            first.bytes().len(),
            usize::try_from(first.block_count()).unwrap() * EXECUTION_BLOCK_BYTES
        );
        assert!(first.maximum_observed_block_ticks() <= 450_000);
        assert_eq!(first.initial_position(), [0, 0]);
        assert_eq!(first.final_position(), [960, 0]);
        assert_eq!(first.terminal_progress().position, [960, 0]);
        assert_eq!(
            first.terminal_progress().end_tick,
            program.segments().last().unwrap().end_tick
        );

        let encoded_plan = first.upload_plan().encode();
        assert_eq!(
            UploadPlan::decode(&encoded_plan, policy().cache_limits()),
            Ok(first.upload_plan())
        );
        let prepare = first.job_prepare_body(0x8877_6655_4433_2211).unwrap();
        assert_eq!(
            JobDescriptor::decode::<2>(&prepare).unwrap(),
            first.job_descriptor(0x8877_6655_4433_2211).unwrap()
        );
    }

    #[test]
    fn arbitrary_storage_chunks_reassemble_every_canonical_block() {
        let program = compile_representative_program().unwrap();
        let artifact = package_canonical_program(&program, policy()).unwrap();
        let mut assembler =
            PartitionAssembler::new(artifact.upload_plan().object.byte_len).unwrap();
        let mut upload = UploadCoordinator::new();
        upload
            .begin(
                artifact.upload_plan(),
                policy().cache_limits(),
                MutationContext::DISARMED_IDLE,
            )
            .unwrap();
        let mut blocks = 0_u32;

        for (index, chunk) in artifact.chunks().iter().enumerate() {
            let header = artifact.chunk_upload_header(index).unwrap();
            let encoded = header.encode();
            assert_eq!(ChunkUploadHeader::decode(&encoded), Ok(header));
            header
                .validate_body_len(
                    u32::try_from(ChunkUploadHeader::WIRE_LEN).unwrap() + chunk.byte_len(),
                )
                .unwrap();

            let mut remaining = artifact.chunk_bytes(index).unwrap();
            let verified = upload
                .verify_chunk(header.upload_id, header.index, header.content, remaining)
                .unwrap();
            upload
                .record_chunk(verified, MutationContext::DISARMED_IDLE)
                .unwrap();
            while !remaining.is_empty() {
                match assembler.push(remaining).unwrap() {
                    alumina_machine_ir::AssembleOutcome::NeedMore { consumed } => {
                        remaining = &remaining[consumed..];
                    }
                    alumina_machine_ir::AssembleOutcome::Block { consumed, .. } => {
                        blocks += 1;
                        remaining = &remaining[consumed..];
                    }
                }
            }
        }
        assert_eq!(assembler.finish(), Ok(artifact.block_count()));
        assert_eq!(blocks, artifact.block_count());
        let publication = upload
            .finalize(
                artifact.upload_plan().upload_id,
                artifact.publication().object.content,
                artifact.publication().manifest,
                MutationContext::DISARMED_IDLE,
            )
            .unwrap();
        assert_eq!(
            upload
                .record_published(publication, MutationContext::DISARMED_IDLE)
                .unwrap(),
            artifact.publication()
        );
    }

    #[test]
    fn firmware_block_horizon_is_replayed_before_artifact_release() {
        let program = compile_representative_program().unwrap();
        let base = policy();
        let too_short = MachinePartitionPolicy2::try_new(
            base.stream_id().0,
            base.capability_digest(),
            base.config_digest(),
            BlockValidationLimits {
                maximum_block_ticks: 399_999,
                segment: base.block_limits().segment,
            },
            base.upload_id(),
            base.storage_chunk_bytes(),
            base.cache_limits(),
        )
        .unwrap();
        assert!(matches!(
            package_canonical_program(&program, too_short),
            Err(MachinePartitionError::Machine(BlockError::BlockTooLong {
                duration: 400_000,
                maximum: 399_999
            }))
        ));
    }

    #[test]
    fn missing_identities_and_storage_bounds_fail_before_packaging() {
        let limits = BlockValidationLimits {
            maximum_block_ticks: 1,
            segment: ValidationLimits {
                maximum_segment_ticks: 1,
                maximum_steps_per_segment: 1,
            },
        };
        let cache = CacheLimits {
            maximum_object_bytes: 512,
            maximum_chunk_bytes: 512,
            maximum_chunks: 1,
        };
        assert!(matches!(
            MachinePartitionPolicy2::try_new(
                [0; 16],
                Digest([1; 32]),
                Digest([2; 32]),
                limits,
                UploadId(1),
                512,
                cache,
            ),
            Err(MachinePartitionError::Machine(BlockError::MissingStreamId))
        ));
        assert!(matches!(
            MachinePartitionPolicy2::try_new(
                [1; 16],
                Digest::ZERO,
                Digest([2; 32]),
                limits,
                UploadId(1),
                512,
                cache,
            ),
            Err(MachinePartitionError::InvalidPolicy(_))
        ));
        assert!(matches!(
            MachinePartitionPolicy2::try_new(
                [1; 16],
                Digest([1; 32]),
                Digest([2; 32]),
                limits,
                UploadId(1),
                513,
                cache,
            ),
            Err(MachinePartitionError::InvalidPolicy(_))
        ));
    }
}
