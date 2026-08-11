//! Exact resumable cache-upload state machine independent of browser/native I/O.

use std::fmt;

use alumina_protocol::{Operation, StatusCode};
use alumina_storage::{
    CacheLimits, ChunkUploadHeader, Error as StorageError, FinalizeUploadRequest, PublishedObject,
    UploadPhase, UploadPlan, UploadProgress, WireError, sha256,
};

use crate::Response;

/// Zero-copy source of one canonical content-addressed upload.
pub trait UploadSource {
    /// Exact immutable upload declaration for this device transaction.
    fn upload_plan(&self) -> UploadPlan;

    /// Exact canonical header for one declared chunk.
    fn chunk_header(&self, index: u32) -> Option<ChunkUploadHeader>;

    /// Borrow the exact bytes named by [`Self::chunk_header`].
    fn chunk_bytes(&self, index: u32) -> Option<&[u8]>;
}

/// One storage operation body ready for native-protocol framing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadOperation {
    /// Exact firmware operation.
    pub operation: Operation,
    /// Canonical operation-specific body.
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UploadAction {
    Inspect,
    Begin,
    Chunk(u32),
    Finalize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UploadState {
    Inspect,
    Begin,
    Chunk {
        next_chunk: u32,
        accepted_bytes: u64,
    },
    Finalize,
    Complete,
}

/// User-facing exact upload/reconciliation phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheUploadPhase {
    /// Checking whether exact content is already published.
    Inspecting,
    /// Beginning or resuming the declared transaction.
    Resuming,
    /// Sending the first missing independently hashed chunk.
    Uploading {
        /// First chunk not durably acknowledged by the device.
        next_chunk: u32,
        /// Exact durably acknowledged object bytes.
        accepted_bytes: u64,
        /// Exact complete object bytes.
        total_bytes: u64,
    },
    /// Asking the device to verify aggregate identities and publish atomically.
    Finalizing,
    /// Exact object/manifest publication was observed or acknowledged.
    Complete,
}

/// Retry-safe upload coordinator whose only recovery edge is exact inspection.
#[derive(Debug)]
pub struct CacheUploadMachine {
    plan: UploadPlan,
    limits: CacheLimits,
    state: UploadState,
    pending: Option<UploadAction>,
}

impl CacheUploadMachine {
    /// Validates and binds one immutable source without retaining its bytes.
    pub fn new(source: &impl UploadSource, limits: CacheLimits) -> Result<Self, CacheUploadError> {
        let plan = source.upload_plan();
        plan.validate(limits)?;
        PublishedObject {
            object: plan.object,
            manifest: plan.manifest,
        }
        .validate(limits)?;
        Ok(Self {
            plan,
            limits,
            state: UploadState::Inspect,
            pending: None,
        })
    }

    /// Exact immutable transaction declaration bound at construction.
    pub const fn upload_plan(&self) -> UploadPlan {
        self.plan
    }

    /// Exact publication identity used for every reconciliation query.
    pub const fn publication(&self) -> PublishedObject {
        PublishedObject {
            object: self.plan.object,
            manifest: self.plan.manifest,
        }
    }

    /// Current durable progress interpretation.
    pub const fn phase(&self) -> CacheUploadPhase {
        match self.state {
            UploadState::Inspect => CacheUploadPhase::Inspecting,
            UploadState::Begin => CacheUploadPhase::Resuming,
            UploadState::Chunk {
                next_chunk,
                accepted_bytes,
            } => CacheUploadPhase::Uploading {
                next_chunk,
                accepted_bytes,
                total_bytes: self.plan.object.byte_len,
            },
            UploadState::Finalize => CacheUploadPhase::Finalizing,
            UploadState::Complete => CacheUploadPhase::Complete,
        }
    }

    /// Whether a request has been emitted but not accepted or abandoned.
    pub const fn has_pending_request(&self) -> bool {
        self.pending.is_some()
    }

    /// Emits the unique next operation and retains its correlation state.
    pub fn next_request(
        &mut self,
        source: &impl UploadSource,
    ) -> Result<Option<UploadOperation>, CacheUploadError> {
        if self.pending.is_some() {
            return Err(CacheUploadError::RequestPending);
        }
        if source.upload_plan() != self.plan {
            return Err(CacheUploadError::SourceChanged);
        }
        let (action, request) = match self.state {
            UploadState::Inspect => (
                UploadAction::Inspect,
                UploadOperation {
                    operation: Operation::StorageInspect,
                    body: self.publication().encode().to_vec(),
                },
            ),
            UploadState::Begin => (
                UploadAction::Begin,
                UploadOperation {
                    operation: Operation::StorageBeginUpload,
                    body: self.plan.encode().to_vec(),
                },
            ),
            UploadState::Chunk { next_chunk, .. } => {
                let header = source
                    .chunk_header(next_chunk)
                    .ok_or(CacheUploadError::MissingChunk(next_chunk))?;
                let bytes = source
                    .chunk_bytes(next_chunk)
                    .ok_or(CacheUploadError::MissingChunk(next_chunk))?;
                self.validate_chunk(next_chunk, header, bytes)?;
                let mut body = Vec::new();
                body.try_reserve_exact(ChunkUploadHeader::WIRE_LEN + bytes.len())
                    .map_err(|_| CacheUploadError::AllocationOverflow)?;
                body.extend_from_slice(&header.encode());
                body.extend_from_slice(bytes);
                (
                    UploadAction::Chunk(next_chunk),
                    UploadOperation {
                        operation: Operation::StoragePutChunk,
                        body,
                    },
                )
            }
            UploadState::Finalize => (
                UploadAction::Finalize,
                UploadOperation {
                    operation: Operation::StorageFinalize,
                    body: FinalizeUploadRequest {
                        upload_id: self.plan.upload_id,
                    }
                    .encode()
                    .to_vec(),
                },
            ),
            UploadState::Complete => return Ok(None),
        };
        self.pending = Some(action);
        Ok(Some(request))
    }

    /// Applies one already authenticated, correlated native response.
    pub fn accept_response(&mut self, response: &Response) -> Result<(), CacheUploadError> {
        let action = self
            .pending
            .take()
            .ok_or(CacheUploadError::NoPendingRequest)?;
        let result = (|| match action {
            UploadAction::Inspect => self.accept_inspection(response),
            UploadAction::Begin => {
                require_ok(response)?;
                let progress = UploadProgress::decode(&response.body)?;
                self.state = self.state_from_progress(progress)?;
                Ok(())
            }
            UploadAction::Chunk(index) => {
                require_ok(response)?;
                let progress = UploadProgress::decode(&response.body)?;
                let expected_next = index
                    .checked_add(1)
                    .ok_or(CacheUploadError::CounterOverflow)?;
                if progress.next_chunk != expected_next {
                    return Err(CacheUploadError::ProgressMismatch);
                }
                self.state = self.state_from_progress(progress)?;
                Ok(())
            }
            UploadAction::Finalize => {
                require_ok(response)?;
                if !response.body.is_empty() {
                    return Err(CacheUploadError::ResponseBody);
                }
                self.state = UploadState::Complete;
                Ok(())
            }
        })();
        if result.is_err() {
            self.state = UploadState::Inspect;
        }
        result
    }

    /// Abandons an ambiguous I/O result and returns to content inspection.
    pub fn abandon_pending(&mut self) -> bool {
        let abandoned = self.pending.take().is_some();
        if abandoned {
            self.state = UploadState::Inspect;
        }
        abandoned
    }

    fn accept_inspection(&mut self, response: &Response) -> Result<(), CacheUploadError> {
        match response.status {
            StatusCode::Ok => {
                let observed = PublishedObject::decode(&response.body, self.limits)?;
                if observed != self.publication() {
                    return Err(CacheUploadError::PublicationMismatch);
                }
                self.state = UploadState::Complete;
                Ok(())
            }
            StatusCode::NotFound if response.body.is_empty() => {
                self.state = UploadState::Begin;
                Ok(())
            }
            StatusCode::NotFound => Err(CacheUploadError::ResponseBody),
            status => {
                if response.body.is_empty() {
                    Err(CacheUploadError::DeviceStatus(status))
                } else {
                    Err(CacheUploadError::ResponseBody)
                }
            }
        }
    }

    fn state_from_progress(
        &self,
        progress: UploadProgress,
    ) -> Result<UploadState, CacheUploadError> {
        if progress.upload_id != self.plan.upload_id
            || progress.total_bytes != self.plan.object.byte_len
            || progress.next_chunk > self.plan.chunk_count
        {
            return Err(CacheUploadError::ProgressMismatch);
        }
        let expected_bytes = expected_prefix_bytes(self.plan, progress.next_chunk)?;
        if progress.accepted_bytes != expected_bytes {
            return Err(CacheUploadError::ProgressMismatch);
        }
        if progress.next_chunk < self.plan.chunk_count {
            if progress.phase != UploadPhase::Receiving {
                return Err(CacheUploadError::ProgressMismatch);
            }
            Ok(UploadState::Chunk {
                next_chunk: progress.next_chunk,
                accepted_bytes: progress.accepted_bytes,
            })
        } else if matches!(
            progress.phase,
            UploadPhase::Receiving | UploadPhase::PublishPending
        ) {
            Ok(UploadState::Finalize)
        } else {
            Err(CacheUploadError::ProgressMismatch)
        }
    }

    fn validate_chunk(
        &self,
        index: u32,
        header: ChunkUploadHeader,
        bytes: &[u8],
    ) -> Result<(), CacheUploadError> {
        let expected_len = expected_chunk_bytes(self.plan, index)?;
        let received_len =
            u32::try_from(bytes.len()).map_err(|_| CacheUploadError::Chunk(index))?;
        if header.upload_id != self.plan.upload_id
            || header.index != index
            || header.byte_len != expected_len
            || received_len != expected_len
            || header.content != sha256(bytes)
        {
            return Err(CacheUploadError::Chunk(index));
        }
        Ok(())
    }
}

fn require_ok(response: &Response) -> Result<(), CacheUploadError> {
    if response.status != StatusCode::Ok {
        return if response.body.is_empty() {
            Err(CacheUploadError::DeviceStatus(response.status))
        } else {
            Err(CacheUploadError::ResponseBody)
        };
    }
    Ok(())
}

fn expected_prefix_bytes(plan: UploadPlan, next_chunk: u32) -> Result<u64, CacheUploadError> {
    if next_chunk > plan.chunk_count {
        return Err(CacheUploadError::ProgressMismatch);
    }
    if next_chunk == plan.chunk_count {
        Ok(plan.object.byte_len)
    } else {
        u64::from(plan.chunk_bytes)
            .checked_mul(u64::from(next_chunk))
            .ok_or(CacheUploadError::CounterOverflow)
    }
}

fn expected_chunk_bytes(plan: UploadPlan, index: u32) -> Result<u32, CacheUploadError> {
    if index >= plan.chunk_count {
        return Err(CacheUploadError::MissingChunk(index));
    }
    if index + 1 < plan.chunk_count {
        return Ok(plan.chunk_bytes);
    }
    let preceding = u64::from(plan.chunk_bytes)
        .checked_mul(u64::from(index))
        .ok_or(CacheUploadError::CounterOverflow)?;
    u32::try_from(
        plan.object
            .byte_len
            .checked_sub(preceding)
            .ok_or(CacheUploadError::CounterOverflow)?,
    )
    .map_err(|_| CacheUploadError::CounterOverflow)
}

/// Failure before upload progress may become authoritative UI state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheUploadError {
    /// Source upload declaration changed after the state machine was bound.
    SourceChanged,
    /// One request must be accepted or abandoned before another is emitted.
    RequestPending,
    /// No emitted request exists for the supplied response.
    NoPendingRequest,
    /// State requested a chunk the source did not expose.
    MissingChunk(u32),
    /// Chunk header, length, upload identity, or SHA-256 did not match source bytes.
    Chunk(u32),
    /// A small request body allocation could not be reserved.
    AllocationOverflow,
    /// Checked storage counter/length arithmetic overflowed.
    CounterOverflow,
    /// Device returned a typed application rejection without a response body.
    DeviceStatus(StatusCode),
    /// Response body was present/absent contrary to the selected operation/status.
    ResponseBody,
    /// Durable upload progress disagreed with the exact local plan prefix.
    ProgressMismatch,
    /// Successful inspection returned another publication identity.
    PublicationMismatch,
    /// Shared storage policy rejected the bound plan/publication.
    Storage(StorageError),
    /// Shared storage wire decoder rejected a response body.
    Wire(WireError),
}

impl From<StorageError> for CacheUploadError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<WireError> for CacheUploadError {
    fn from(value: WireError) -> Self {
        Self::Wire(value)
    }
}

impl fmt::Display for CacheUploadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceChanged => formatter.write_str("upload source changed after binding"),
            Self::RequestPending => formatter.write_str("an upload request is already pending"),
            Self::NoPendingRequest => formatter.write_str("no upload request is pending"),
            Self::MissingChunk(index) => write!(formatter, "upload chunk {index} is absent"),
            Self::Chunk(index) => write!(formatter, "upload chunk {index} failed local identity"),
            Self::AllocationOverflow => formatter.write_str("upload request allocation failed"),
            Self::CounterOverflow => formatter.write_str("upload counter overflowed"),
            Self::DeviceStatus(status) => write!(formatter, "device rejected upload: {status:?}"),
            Self::ResponseBody => formatter.write_str("upload response body is not canonical"),
            Self::ProgressMismatch => {
                formatter.write_str("device upload progress disagrees with local content")
            }
            Self::PublicationMismatch => {
                formatter.write_str("inspected publication disagrees with local content")
            }
            Self::Storage(error) => write!(formatter, "upload plan rejected: {error:?}"),
            Self::Wire(error) => write!(formatter, "upload response rejected: {error:?}"),
        }
    }
}

impl std::error::Error for CacheUploadError {}

#[cfg(test)]
mod tests {
    use alumina_protocol::Digest;
    use alumina_storage::{
        ContentId, ManifestHasher, MutationContext, ObjectKind, StoredObject, UploadCoordinator,
        UploadId,
    };

    use super::*;

    const LIMITS: CacheLimits = CacheLimits {
        maximum_object_bytes: 1_024,
        maximum_chunk_bytes: 4,
        maximum_chunks: 256,
    };

    struct FixtureSource {
        plan: UploadPlan,
        chunks: Vec<Vec<u8>>,
        headers: Vec<ChunkUploadHeader>,
    }

    impl UploadSource for FixtureSource {
        fn upload_plan(&self) -> UploadPlan {
            self.plan
        }

        fn chunk_header(&self, index: u32) -> Option<ChunkUploadHeader> {
            self.headers.get(usize::try_from(index).ok()?).copied()
        }

        fn chunk_bytes(&self, index: u32) -> Option<&[u8]> {
            self.chunks
                .get(usize::try_from(index).ok()?)
                .map(Vec::as_slice)
        }
    }

    fn source() -> FixtureSource {
        let chunks = vec![b"abcd".to_vec(), b"efgh".to_vec(), b"ij".to_vec()];
        let object = StoredObject {
            kind: ObjectKind::MachineJobPartition,
            content: sha256(b"abcdefghij"),
            byte_len: 10,
        };
        let mut manifest = ManifestHasher::new(object, 4, 3, LIMITS).unwrap();
        let mut headers = Vec::new();
        for (index, bytes) in chunks.iter().enumerate() {
            let index = u32::try_from(index).unwrap();
            let content = sha256(bytes);
            let byte_len = u32::try_from(bytes.len()).unwrap();
            manifest.push(index, content, byte_len).unwrap();
            headers.push(ChunkUploadHeader {
                upload_id: UploadId(7),
                index,
                byte_len,
                content,
            });
        }
        FixtureSource {
            plan: UploadPlan {
                upload_id: UploadId(7),
                object,
                manifest: manifest.finalize().unwrap(),
                chunk_bytes: 4,
                chunk_count: 3,
            },
            chunks,
            headers,
        }
    }

    struct Device {
        upload: UploadCoordinator,
        published: Option<PublishedObject>,
        begin_count: u32,
        finalize_count: u32,
    }

    impl Device {
        fn new() -> Self {
            Self {
                upload: UploadCoordinator::new(),
                published: None,
                begin_count: 0,
                finalize_count: 0,
            }
        }

        fn dispatch(&mut self, request: &UploadOperation) -> Response {
            match request.operation {
                Operation::StorageInspect => {
                    let expected = PublishedObject::decode(&request.body, LIMITS).unwrap();
                    match self.published {
                        Some(publication) if publication == expected => Response {
                            status: StatusCode::Ok,
                            body: publication.encode().to_vec(),
                        },
                        _ => Response {
                            status: StatusCode::NotFound,
                            body: Vec::new(),
                        },
                    }
                }
                Operation::StorageBeginUpload => {
                    self.begin_count += 1;
                    let plan = UploadPlan::decode(&request.body, LIMITS).unwrap();
                    let progress = self
                        .upload
                        .begin(plan, LIMITS, MutationContext::DISARMED_IDLE)
                        .unwrap();
                    Response {
                        status: StatusCode::Ok,
                        body: progress.encode().to_vec(),
                    }
                }
                Operation::StoragePutChunk => {
                    let header =
                        ChunkUploadHeader::decode(&request.body[..ChunkUploadHeader::WIRE_LEN])
                            .unwrap();
                    let bytes = &request.body[ChunkUploadHeader::WIRE_LEN..];
                    let verified = self
                        .upload
                        .verify_chunk(header.upload_id, header.index, header.content, bytes)
                        .unwrap();
                    let progress = self
                        .upload
                        .record_chunk(verified, MutationContext::DISARMED_IDLE)
                        .unwrap();
                    Response {
                        status: StatusCode::Ok,
                        body: progress.encode().to_vec(),
                    }
                }
                Operation::StorageFinalize => {
                    self.finalize_count += 1;
                    let request = FinalizeUploadRequest::decode(&request.body).unwrap();
                    let plan = self.upload.checkpoint().unwrap().plan;
                    let token = self
                        .upload
                        .finalize(
                            request.upload_id,
                            plan.object.content,
                            plan.manifest,
                            MutationContext::DISARMED_IDLE,
                        )
                        .unwrap();
                    self.published = Some(
                        self.upload
                            .record_published(token, MutationContext::DISARMED_IDLE)
                            .unwrap(),
                    );
                    Response {
                        status: StatusCode::Ok,
                        body: Vec::new(),
                    }
                }
                operation => panic!("unexpected operation {operation:?}"),
            }
        }
    }

    fn step(
        machine: &mut CacheUploadMachine,
        source: &FixtureSource,
        device: &mut Device,
    ) -> Operation {
        let request = machine.next_request(source).unwrap().unwrap();
        let operation = request.operation;
        let response = device.dispatch(&request);
        machine.accept_response(&response).unwrap();
        operation
    }

    #[test]
    fn lost_chunk_response_resumes_and_lost_finalize_reconciles_by_content() {
        let source = source();
        let mut machine = CacheUploadMachine::new(&source, LIMITS).unwrap();
        let mut device = Device::new();

        assert_eq!(
            step(&mut machine, &source, &mut device),
            Operation::StorageInspect
        );
        assert_eq!(
            step(&mut machine, &source, &mut device),
            Operation::StorageBeginUpload
        );

        let first_chunk = machine.next_request(&source).unwrap().unwrap();
        assert_eq!(first_chunk.operation, Operation::StoragePutChunk);
        let _lost = device.dispatch(&first_chunk);
        assert!(machine.abandon_pending());
        assert_eq!(machine.phase(), CacheUploadPhase::Inspecting);

        assert_eq!(
            step(&mut machine, &source, &mut device),
            Operation::StorageInspect
        );
        assert_eq!(
            step(&mut machine, &source, &mut device),
            Operation::StorageBeginUpload
        );
        assert_eq!(device.begin_count, 2);
        assert!(matches!(
            machine.phase(),
            CacheUploadPhase::Uploading {
                next_chunk: 1,
                accepted_bytes: 4,
                total_bytes: 10,
            }
        ));

        while machine.phase() != CacheUploadPhase::Finalizing {
            assert_eq!(
                step(&mut machine, &source, &mut device),
                Operation::StoragePutChunk
            );
        }
        let finalize = machine.next_request(&source).unwrap().unwrap();
        assert_eq!(finalize.operation, Operation::StorageFinalize);
        let _lost = device.dispatch(&finalize);
        assert!(machine.abandon_pending());

        assert_eq!(
            step(&mut machine, &source, &mut device),
            Operation::StorageInspect
        );
        assert_eq!(machine.phase(), CacheUploadPhase::Complete);
        assert_eq!(device.finalize_count, 1);
        assert!(machine.next_request(&source).unwrap().is_none());
    }

    #[test]
    fn impossible_progress_and_local_chunk_tamper_fail_closed() {
        let mut source = source();
        let mut machine = CacheUploadMachine::new(&source, LIMITS).unwrap();
        let inspect = machine.next_request(&source).unwrap().unwrap();
        assert_eq!(inspect.operation, Operation::StorageInspect);
        machine
            .accept_response(&Response {
                status: StatusCode::NotFound,
                body: Vec::new(),
            })
            .unwrap();
        let begin = machine.next_request(&source).unwrap().unwrap();
        assert_eq!(begin.operation, Operation::StorageBeginUpload);
        let impossible = UploadProgress {
            upload_id: UploadId(8),
            phase: UploadPhase::Receiving,
            next_chunk: 0,
            accepted_bytes: 0,
            total_bytes: 10,
        };
        assert_eq!(
            machine.accept_response(&Response {
                status: StatusCode::Ok,
                body: impossible.encode().to_vec(),
            }),
            Err(CacheUploadError::ProgressMismatch)
        );
        assert_eq!(machine.phase(), CacheUploadPhase::Inspecting);

        let mut machine = CacheUploadMachine::new(&source, LIMITS).unwrap();
        let mut device = Device::new();
        step(&mut machine, &source, &mut device);
        step(&mut machine, &source, &mut device);
        source.chunks[0][0] ^= 1;
        assert_eq!(
            machine.next_request(&source),
            Err(CacheUploadError::Chunk(0))
        );
    }

    #[test]
    fn zero_publication_identity_is_rejected_before_any_request() {
        let mut source = source();
        source.plan.manifest = ContentId::from_sha256(Digest::ZERO);
        assert!(matches!(
            CacheUploadMachine::new(&source, LIMITS),
            Err(CacheUploadError::Storage(
                StorageError::MissingDigest { .. }
            ))
        ));
    }
}
