//! Zero-copy upload adapters from authoritative compiler artifacts to the Wi-Fi client.

use std::fmt;

use alumina_interface_client::Response;
use alumina_interface_client::upload::{
    CacheUploadError, CacheUploadMachine, CacheUploadPhase, UploadOperation, UploadSource,
};
use alumina_interface_core::{
    CanonicalGlobalJob2, CanonicalMachinePartition2, GlobalJobCompileError,
};
use alumina_protocol::DeviceId;
use alumina_storage::{ChunkUploadHeader, PublishedObject, UploadId, UploadPlan};

/// Zero-copy source for one canonical per-MCU execution partition.
#[derive(Clone, Copy, Debug)]
pub struct PartitionUploadSource<'a> {
    partition: &'a CanonicalMachinePartition2,
}

impl<'a> PartitionUploadSource<'a> {
    /// Borrows one independently replayed partition artifact.
    #[must_use]
    pub const fn new(partition: &'a CanonicalMachinePartition2) -> Self {
        Self { partition }
    }
}

impl UploadSource for PartitionUploadSource<'_> {
    fn upload_plan(&self) -> UploadPlan {
        self.partition.upload_plan()
    }

    fn chunk_header(&self, index: u32) -> Option<ChunkUploadHeader> {
        self.partition
            .chunk_upload_header(usize::try_from(index).ok()?)
    }

    fn chunk_bytes(&self, index: u32) -> Option<&[u8]> {
        self.partition.chunk_bytes(usize::try_from(index).ok()?)
    }
}

/// Zero-copy source for the identical global manifest under one device-local transaction ID.
#[derive(Clone, Copy, Debug)]
pub struct GlobalManifestUploadSource<'a> {
    job: &'a CanonicalGlobalJob2,
    plan: UploadPlan,
}

impl<'a> GlobalManifestUploadSource<'a> {
    /// Binds content identity to one nonzero per-device resumable transaction.
    ///
    /// # Errors
    ///
    /// Returns the shared global-job policy error when the device-local upload
    /// identity is zero or cannot satisfy the retained cache limits.
    pub fn try_new(
        job: &'a CanonicalGlobalJob2,
        upload_id: UploadId,
    ) -> Result<Self, GlobalJobCompileError> {
        Ok(Self {
            job,
            plan: job.upload_plan_for(upload_id)?,
        })
    }
}

impl UploadSource for GlobalManifestUploadSource<'_> {
    fn upload_plan(&self) -> UploadPlan {
        self.plan
    }

    fn chunk_header(&self, index: u32) -> Option<ChunkUploadHeader> {
        let chunk = self
            .job
            .manifest_chunks()
            .get(usize::try_from(index).ok()?)?;
        Some(ChunkUploadHeader {
            upload_id: self.plan.upload_id,
            index,
            byte_len: chunk.byte_len(),
            content: chunk.content(),
        })
    }

    fn chunk_bytes(&self, index: u32) -> Option<&[u8]> {
        self.job.manifest_chunk_bytes(usize::try_from(index).ok()?)
    }
}

/// Which immutable artifact is being reconciled for one participant MCU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParticipantCachePhase {
    /// Device-local executable partition is being inspected or uploaded.
    Partition(CacheUploadPhase),
    /// Identical global job manifest is being inspected or uploaded afterward.
    GlobalManifest(CacheUploadPhase),
    /// Both exact publications have been observed on this MCU.
    Complete,
}

/// Ordered, retry-safe partition-then-global-manifest delivery for one MCU.
#[derive(Debug)]
pub struct ParticipantCacheDelivery {
    participant_index: usize,
    device_id: DeviceId,
    manifest_upload_id: UploadId,
    partition: CacheUploadMachine,
    manifest: CacheUploadMachine,
}

impl ParticipantCacheDelivery {
    /// Binds both immutable sources before any network request can be emitted.
    ///
    /// # Errors
    ///
    /// Returns an exact compiler/storage error if the participant does not
    /// exist or either source cannot satisfy its retained cache policy.
    pub fn try_new(
        job: &CanonicalGlobalJob2,
        participant_index: usize,
        manifest_upload_id: UploadId,
    ) -> Result<Self, ParticipantCacheDeliveryError> {
        let participant = job
            .participants()
            .get(participant_index)
            .ok_or(ParticipantCacheDeliveryError::Participant)?;
        let partition_source = PartitionUploadSource::new(participant.partition());
        let partition = CacheUploadMachine::new(
            &partition_source,
            participant.partition().policy().cache_limits(),
        )?;
        let manifest_source = GlobalManifestUploadSource::try_new(job, manifest_upload_id)?;
        let manifest = CacheUploadMachine::new(&manifest_source, job.policy().cache_limits())?;
        Ok(Self {
            participant_index,
            device_id: participant.device_id(),
            manifest_upload_id,
            partition,
            manifest,
        })
    }

    /// Stable target MCU identity from the sorted global manifest.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Current durable interpretation, never inferred from transport success alone.
    #[must_use]
    pub const fn phase(&self) -> ParticipantCachePhase {
        match (self.partition.phase(), self.manifest.phase()) {
            (CacheUploadPhase::Complete, CacheUploadPhase::Complete) => {
                ParticipantCachePhase::Complete
            }
            (CacheUploadPhase::Complete, phase) => ParticipantCachePhase::GlobalManifest(phase),
            (phase, _) => ParticipantCachePhase::Partition(phase),
        }
    }

    /// Exact partition and global-manifest identities required on this MCU.
    #[must_use]
    pub const fn publications(&self) -> (PublishedObject, PublishedObject) {
        (self.partition.publication(), self.manifest.publication())
    }

    /// Emits the next partition request, then the next global-manifest request.
    ///
    /// # Errors
    ///
    /// Fails closed if the caller substitutes another job/participant or either
    /// underlying reconciliation machine rejects its source or state.
    pub fn next_request(
        &mut self,
        job: &CanonicalGlobalJob2,
    ) -> Result<Option<UploadOperation>, ParticipantCacheDeliveryError> {
        let participant = self.participant(job)?;
        if self.partition.phase() != CacheUploadPhase::Complete {
            return self
                .partition
                .next_request(&PartitionUploadSource::new(participant.partition()))
                .map_err(Into::into);
        }
        if self.manifest.phase() != CacheUploadPhase::Complete {
            let source = GlobalManifestUploadSource::try_new(job, self.manifest_upload_id)?;
            return self.manifest.next_request(&source).map_err(Into::into);
        }
        Ok(None)
    }

    /// Applies one authenticated, correlated response to the unique pending artifact.
    ///
    /// # Errors
    ///
    /// Returns a state error when neither or both artifact machines are pending,
    /// or forwards the exact upload rejection.
    pub fn accept_response(
        &mut self,
        response: &Response,
    ) -> Result<(), ParticipantCacheDeliveryError> {
        match (
            self.partition.has_pending_request(),
            self.manifest.has_pending_request(),
        ) {
            (true, false) => self.partition.accept_response(response)?,
            (false, true) => self.manifest.accept_response(response)?,
            _ => return Err(ParticipantCacheDeliveryError::Pending),
        }
        Ok(())
    }

    /// Abandons an ambiguous I/O result and forces exact content inspection next.
    pub fn abandon_pending(&mut self) -> bool {
        let partition = self.partition.abandon_pending();
        let manifest = self.manifest.abandon_pending();
        partition || manifest
    }

    fn participant<'a>(
        &self,
        job: &'a CanonicalGlobalJob2,
    ) -> Result<
        &'a alumina_interface_core::MachineJobParticipantPackage2,
        ParticipantCacheDeliveryError,
    > {
        let participant = job
            .participants()
            .get(self.participant_index)
            .ok_or(ParticipantCacheDeliveryError::Participant)?;
        if participant.device_id() != self.device_id {
            return Err(ParticipantCacheDeliveryError::Participant);
        }
        Ok(participant)
    }
}

/// Atomically binds one independent delivery state machine per sorted participant.
///
/// # Errors
///
/// Requires exactly one device-local manifest upload ID per participant and
/// validates every machine before returning any of them.
pub fn prepare_global_cache_delivery(
    job: &CanonicalGlobalJob2,
    manifest_upload_ids: &[UploadId],
) -> Result<Vec<ParticipantCacheDelivery>, ParticipantCacheDeliveryError> {
    if manifest_upload_ids.len() != job.participants().len() {
        return Err(ParticipantCacheDeliveryError::Participant);
    }
    manifest_upload_ids
        .iter()
        .copied()
        .enumerate()
        .map(|(index, upload_id)| ParticipantCacheDelivery::try_new(job, index, upload_id))
        .collect()
}

/// Failure before a participant may be reported cache-ready.
#[derive(Debug)]
pub enum ParticipantCacheDeliveryError {
    /// Participant index/identity does not match the bound canonical job.
    Participant,
    /// Pending response ownership was absent or ambiguous.
    Pending,
    /// Global-manifest source construction failed.
    GlobalJob(GlobalJobCompileError),
    /// Exact upload/reconciliation state rejected the source or response.
    Upload(CacheUploadError),
}

impl From<GlobalJobCompileError> for ParticipantCacheDeliveryError {
    fn from(value: GlobalJobCompileError) -> Self {
        Self::GlobalJob(value)
    }
}

impl From<CacheUploadError> for ParticipantCacheDeliveryError {
    fn from(value: CacheUploadError) -> Self {
        Self::Upload(value)
    }
}

impl fmt::Display for ParticipantCacheDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Participant => formatter.write_str("participant does not match canonical job"),
            Self::Pending => {
                formatter.write_str("participant response has no unique pending owner")
            }
            Self::GlobalJob(error) => write!(formatter, "global job rejected: {error}"),
            Self::Upload(error) => write!(formatter, "cache upload rejected: {error}"),
        }
    }
}

impl std::error::Error for ParticipantCacheDeliveryError {}

#[cfg(test)]
mod tests {
    use alumina_interface_client::upload::CacheUploadMachine;
    use alumina_interface_core::{
        compile_representative_global_job, compile_representative_program,
    };
    use alumina_protocol::{Operation, StatusCode};
    use alumina_storage::{UploadPhase, UploadProgress};

    use super::*;

    #[test]
    fn authoritative_partition_and_global_artifacts_bind_without_copying() {
        let program = compile_representative_program().unwrap();
        let job = compile_representative_global_job(&program).unwrap();
        let participant = &job.participants()[0];
        let partition = PartitionUploadSource::new(participant.partition());
        let partition_machine =
            CacheUploadMachine::new(&partition, participant.partition().policy().cache_limits())
                .unwrap();
        assert_eq!(
            partition_machine.publication(),
            participant.partition().publication()
        );

        let manifest =
            GlobalManifestUploadSource::try_new(&job, UploadId(0x5152_5354_5556_5758)).unwrap();
        let manifest_machine =
            CacheUploadMachine::new(&manifest, job.policy().cache_limits()).unwrap();
        assert_eq!(manifest_machine.publication(), job.publication());
        assert_ne!(
            manifest.upload_plan().upload_id,
            job.upload_plan().upload_id
        );
        assert_eq!(manifest.upload_plan().object, job.upload_plan().object);
        assert_eq!(manifest.upload_plan().manifest, job.upload_plan().manifest);
    }

    #[test]
    fn every_participant_reconciles_partition_before_identical_global_manifest() {
        let program = compile_representative_program().unwrap();
        let job = compile_representative_global_job(&program).unwrap();
        let ids = [
            UploadId(0x1112_1314_1516_1718),
            UploadId(0x2122_2324_2526_2728),
        ];
        let mut deliveries = prepare_global_cache_delivery(&job, &ids).unwrap();
        assert_eq!(deliveries.len(), job.participants().len());

        for delivery in &mut deliveries {
            let expected_publications = delivery.publications();
            let mut observed_operations = Vec::new();
            while delivery.phase() != ParticipantCachePhase::Complete {
                let phase = delivery.phase();
                let operation = delivery.next_request(&job).unwrap().unwrap();
                observed_operations.push(operation.operation);
                let response = match phase {
                    ParticipantCachePhase::Partition(CacheUploadPhase::Inspecting)
                    | ParticipantCachePhase::GlobalManifest(CacheUploadPhase::Inspecting) => {
                        Response {
                            status: StatusCode::NotFound,
                            body: Vec::new(),
                        }
                    }
                    ParticipantCachePhase::Partition(CacheUploadPhase::Resuming)
                    | ParticipantCachePhase::GlobalManifest(CacheUploadPhase::Resuming) => {
                        let plan = plan_for_phase(delivery, phase);
                        Response {
                            status: StatusCode::Ok,
                            body: UploadProgress {
                                upload_id: plan.upload_id,
                                phase: UploadPhase::Receiving,
                                next_chunk: 0,
                                accepted_bytes: 0,
                                total_bytes: plan.object.byte_len,
                            }
                            .encode()
                            .to_vec(),
                        }
                    }
                    ParticipantCachePhase::Partition(CacheUploadPhase::Uploading {
                        next_chunk,
                        accepted_bytes,
                        ..
                    })
                    | ParticipantCachePhase::GlobalManifest(CacheUploadPhase::Uploading {
                        next_chunk,
                        accepted_bytes,
                        ..
                    }) => {
                        let plan = plan_for_phase(delivery, phase);
                        let header = ChunkUploadHeader::decode(
                            &operation.body[..ChunkUploadHeader::WIRE_LEN],
                        )
                        .unwrap();
                        Response {
                            status: StatusCode::Ok,
                            body: UploadProgress {
                                upload_id: plan.upload_id,
                                phase: UploadPhase::Receiving,
                                next_chunk: next_chunk + 1,
                                accepted_bytes: accepted_bytes + u64::from(header.byte_len),
                                total_bytes: plan.object.byte_len,
                            }
                            .encode()
                            .to_vec(),
                        }
                    }
                    ParticipantCachePhase::Partition(CacheUploadPhase::Finalizing)
                    | ParticipantCachePhase::GlobalManifest(CacheUploadPhase::Finalizing) => {
                        Response {
                            status: StatusCode::Ok,
                            body: Vec::new(),
                        }
                    }
                    ParticipantCachePhase::Partition(CacheUploadPhase::Complete)
                    | ParticipantCachePhase::GlobalManifest(CacheUploadPhase::Complete)
                    | ParticipantCachePhase::Complete => unreachable!(),
                };
                delivery.accept_response(&response).unwrap();
            }
            let manifest_inspect = observed_operations
                .iter()
                .rposition(|operation| *operation == Operation::StorageInspect)
                .unwrap();
            let partition_finalize = observed_operations
                .iter()
                .position(|operation| *operation == Operation::StorageFinalize)
                .unwrap();
            assert!(partition_finalize < manifest_inspect);
            assert_eq!(delivery.publications(), expected_publications);
        }
    }

    fn plan_for_phase(
        delivery: &ParticipantCacheDelivery,
        phase: ParticipantCachePhase,
    ) -> UploadPlan {
        match phase {
            ParticipantCachePhase::Partition(_) => delivery.partition.upload_plan(),
            ParticipantCachePhase::GlobalManifest(_) => delivery.manifest.upload_plan(),
            ParticipantCachePhase::Complete => unreachable!(),
        }
    }
}
