//! Retry-safe authenticated assembly of one canonical board capability document.

use core::fmt;

use alumina_capability::{
    BoardCapabilityLimits, BoardCapabilityView, CAPABILITY_DOCUMENT_HEADER_BYTES,
    CapabilityDocumentError, CapabilityIdentity, CapabilityReadRequest, CapabilityReadResponse,
    CapabilityWireError, MAX_CAPABILITY_CHUNK_BYTES, decode_board_capability,
};
use alumina_protocol::{Digest, Operation, StatusCode};

use crate::Response;

/// One side-effect-free capability range ready for authenticated native framing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityOperation {
    /// Always [`Operation::CapabilitiesGet`].
    pub operation: Operation,
    /// Canonical fixed range-request body.
    pub body: Vec<u8>,
}

/// Exact acquisition phase retained across ambiguous network outcomes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityDownloadPhase {
    /// No authenticated range has established the document identity yet.
    Discovering,
    /// A stable identity is known and a contiguous prefix is retained.
    Downloading,
    /// Every byte passed canonical decoding and complete SHA-256 identity validation.
    Complete,
}

/// Bounded progress facts safe to project into worker diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityDownloadProgress {
    /// Exact acquisition phase.
    pub phase: CapabilityDownloadPhase,
    /// Contiguous validated transport prefix already retained.
    pub received_bytes: u32,
    /// Stable complete identity after the first accepted range.
    pub identity: Option<CapabilityIdentity>,
}

/// Retry-safe, contiguous capability document assembler.
#[derive(Debug)]
pub struct CapabilityDownloadMachine {
    limits: BoardCapabilityLimits,
    identity: Option<CapabilityIdentity>,
    document: Vec<u8>,
    received_bytes: u32,
    complete: bool,
    pending: Option<CapabilityReadRequest>,
}

impl CapabilityDownloadMachine {
    /// Creates an empty downloader with explicit untrusted-document bounds.
    ///
    /// # Errors
    ///
    /// Rejects a policy that cannot admit even the fixed capability header or
    /// whose document bound cannot be represented by this client process.
    pub fn new(limits: BoardCapabilityLimits) -> Result<Self, CapabilityDownloadError> {
        let minimum = u32::try_from(CAPABILITY_DOCUMENT_HEADER_BYTES)
            .expect("fixed capability header length fits u32");
        if limits.maximum_document_bytes < minimum
            || usize::try_from(limits.maximum_document_bytes).is_err()
        {
            return Err(CapabilityDownloadError::Limit);
        }
        Ok(Self {
            limits,
            identity: None,
            document: Vec::new(),
            received_bytes: 0,
            complete: false,
            pending: None,
        })
    }

    /// Current exact acquisition phase.
    pub const fn phase(&self) -> CapabilityDownloadPhase {
        if self.complete {
            CapabilityDownloadPhase::Complete
        } else if self.identity.is_some() {
            CapabilityDownloadPhase::Downloading
        } else {
            CapabilityDownloadPhase::Discovering
        }
    }

    /// Stable byte identity after the first authenticated range.
    pub const fn identity(&self) -> Option<CapabilityIdentity> {
        self.identity
    }

    /// Contiguous transport prefix and stable identity facts.
    pub const fn progress(&self) -> CapabilityDownloadProgress {
        CapabilityDownloadProgress {
            phase: self.phase(),
            received_bytes: self.received_bytes,
            identity: self.identity,
        }
    }

    /// Complete independently validated bytes, unavailable before completion.
    pub fn document(&self) -> Option<&[u8]> {
        self.complete.then_some(self.document.as_slice())
    }

    /// Re-decodes and borrows the complete canonical board document.
    ///
    /// # Errors
    ///
    /// Returns `Incomplete` before completion and preserves all decoder errors
    /// if retained memory has been corrupted after admission.
    pub fn capability(&self) -> Result<BoardCapabilityView<'_>, CapabilityDownloadError> {
        if !self.complete {
            return Err(CapabilityDownloadError::Incomplete);
        }
        let view = decode_board_capability(&self.document, self.limits)
            .map_err(CapabilityDownloadError::Document)?;
        if Some(view.identity()) != self.identity {
            return Err(CapabilityDownloadError::Identity);
        }
        Ok(view)
    }

    /// Emits the unique next contiguous range request.
    pub fn next_request(&mut self) -> Result<Option<CapabilityOperation>, CapabilityDownloadError> {
        if self.pending.is_some() {
            return Err(CapabilityDownloadError::RequestPending);
        }
        if self.complete {
            return Ok(None);
        }
        let remaining = self.identity.map_or(
            u32::try_from(MAX_CAPABILITY_CHUNK_BYTES).expect("capability chunk limit fits u32"),
            |identity| identity.byte_len.saturating_sub(self.received_bytes),
        );
        if remaining == 0 {
            return Err(CapabilityDownloadError::Range);
        }
        let maximum_bytes = u16::try_from(remaining.min(
            u32::try_from(MAX_CAPABILITY_CHUNK_BYTES).expect("capability chunk limit fits u32"),
        ))
        .expect("bounded capability range fits u16");
        let request = CapabilityReadRequest {
            expected_digest: self
                .identity
                .map_or(Digest::ZERO, |identity| identity.digest),
            offset: self.received_bytes,
            maximum_bytes,
        };
        let body = request
            .encode()
            .map_err(CapabilityDownloadError::Wire)?
            .to_vec();
        self.pending = Some(request);
        Ok(Some(CapabilityOperation {
            operation: Operation::CapabilitiesGet,
            body,
        }))
    }

    /// Applies one already authenticated and correlated range response.
    pub fn accept_response(&mut self, response: &Response) -> Result<(), CapabilityDownloadError> {
        let request = self
            .pending
            .take()
            .ok_or(CapabilityDownloadError::NoPendingRequest)?;
        if response.status != StatusCode::Ok {
            if response.body.is_empty() {
                return Err(CapabilityDownloadError::DeviceStatus(response.status));
            }
            return Err(CapabilityDownloadError::ResponseBody);
        }
        if response.body.is_empty() {
            return Err(CapabilityDownloadError::ResponseBody);
        }
        let (metadata, chunk) = CapabilityReadResponse::decode_body(&response.body)
            .map_err(CapabilityDownloadError::Wire)?;
        if metadata.offset != request.offset
            || metadata.offset != self.received_bytes
            || metadata.chunk_len > request.maximum_bytes
        {
            return Err(CapabilityDownloadError::Range);
        }
        if metadata.identity.byte_len > self.limits.maximum_document_bytes {
            return Err(CapabilityDownloadError::Limit);
        }
        if let Some(identity) = self.identity {
            if metadata.identity != identity || request.expected_digest != identity.digest {
                return Err(CapabilityDownloadError::Identity);
            }
        } else if !request.expected_digest.is_zero() {
            return Err(CapabilityDownloadError::Identity);
        }

        let total = usize::try_from(metadata.identity.byte_len)
            .map_err(|_| CapabilityDownloadError::Limit)?;
        if self.identity.is_none() {
            let mut document = Vec::new();
            document
                .try_reserve_exact(total)
                .map_err(|_| CapabilityDownloadError::Allocation)?;
            document.resize(total, 0);
            self.document = document;
            self.identity = Some(metadata.identity);
        }
        let start =
            usize::try_from(self.received_bytes).map_err(|_| CapabilityDownloadError::Range)?;
        let end = start
            .checked_add(chunk.len())
            .filter(|end| *end <= self.document.len())
            .ok_or(CapabilityDownloadError::Range)?;
        self.document[start..end].copy_from_slice(chunk);
        let received_bytes = u32::try_from(end).map_err(|_| CapabilityDownloadError::Range)?;

        if metadata.complete {
            if received_bytes != metadata.identity.byte_len {
                return Err(CapabilityDownloadError::Range);
            }
            let view = decode_board_capability(&self.document, self.limits)
                .map_err(CapabilityDownloadError::Document)?;
            if view.identity() != metadata.identity {
                return Err(CapabilityDownloadError::Identity);
            }
            self.received_bytes = received_bytes;
            self.complete = true;
        } else {
            if received_bytes >= metadata.identity.byte_len {
                return Err(CapabilityDownloadError::Range);
            }
            self.received_bytes = received_bytes;
        }
        Ok(())
    }

    /// Makes an ambiguous side-effect-free range eligible for exact retry.
    pub fn abandon_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }

    /// Invalidates every retained byte and restarts digest discovery.
    pub fn reset(&mut self) {
        self.identity = None;
        self.document.clear();
        self.received_bytes = 0;
        self.complete = false;
        self.pending = None;
    }
}

/// Capability acquisition, response, identity, allocation, or decode failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilityDownloadError {
    /// Caller policy cannot admit the declared document.
    Limit,
    /// Browser/native allocation for the bounded document failed.
    Allocation,
    /// Canonical request or response bytes were malformed.
    Wire(CapabilityWireError),
    /// The complete document failed independent canonical decoding.
    Document(CapabilityDocumentError),
    /// Device returned an explicit non-success status with no body.
    DeviceStatus(StatusCode),
    /// Success omitted its body or failure carried ambiguous bytes.
    ResponseBody,
    /// Digest or complete byte identity changed during acquisition.
    Identity,
    /// Offset, chunk length, completion, or contiguous progress diverged.
    Range,
    /// A request is already awaiting acceptance or abandonment.
    RequestPending,
    /// No request was pending when a response arrived.
    NoPendingRequest,
    /// Complete bytes were requested before acquisition completed.
    Incomplete,
}

impl fmt::Display for CapabilityDownloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "capability download rejected: {self:?}")
    }
}

impl std::error::Error for CapabilityDownloadError {}

#[cfg(test)]
mod tests {
    use alumina_capability::{
        CAPABILITY_READ_RESPONSE_PREFIX_BYTES, calculate_identity, read_verified_range,
    };

    use super::*;
    use crate::{ProtocolClient, SimulatedResponse, SimulatorTransport};

    fn simulated_response(body: &[u8]) -> SimulatedResponse {
        let request = CapabilityReadRequest::decode(body).unwrap();
        let mut chunk = [0_u8; MAX_CAPABILITY_CHUNK_BYTES];
        let read = read_verified_range(
            &board_mks_tinybee::PACKAGE,
            request.offset,
            &mut chunk[..usize::from(request.maximum_bytes)],
        )
        .unwrap();
        let prefix = CapabilityReadResponse {
            identity: read.identity,
            offset: read.offset,
            chunk_len: read.byte_len,
            complete: read.complete,
        }
        .encode()
        .unwrap();
        let mut response = Vec::from(prefix);
        response.extend_from_slice(&chunk[..usize::from(read.byte_len)]);
        SimulatedResponse {
            status: StatusCode::Ok,
            body: response,
        }
    }

    #[test]
    fn authentic_ranges_reassemble_and_decode_the_exact_tinybee_document() {
        let transport = SimulatorTransport::new(|operation, body: &[u8]| {
            assert_eq!(operation, Operation::CapabilitiesGet);
            simulated_response(body)
        });
        let mut protocol = ProtocolClient::new(transport, Digest::ZERO);
        let mut download =
            CapabilityDownloadMachine::new(BoardCapabilityLimits::interactive()).unwrap();

        while let Some(operation) = download.next_request().unwrap() {
            let response = protocol
                .request(operation.operation, &operation.body)
                .unwrap();
            download.accept_response(&response).unwrap();
        }

        assert_eq!(download.phase(), CapabilityDownloadPhase::Complete);
        assert_eq!(
            download.identity(),
            Some(calculate_identity(&board_mks_tinybee::PACKAGE).unwrap())
        );
        assert_eq!(download.capability().unwrap().board_id(), "mks-tinybee-v1");
        assert!(protocol.transport().request_count() > 1);
    }

    #[test]
    fn ambiguous_read_retries_the_identical_side_effect_free_range() {
        let mut download =
            CapabilityDownloadMachine::new(BoardCapabilityLimits::interactive()).unwrap();
        let first = download.next_request().unwrap().unwrap();
        assert!(download.abandon_pending());
        let retry = download.next_request().unwrap().unwrap();
        assert_eq!(retry, first);
    }

    #[test]
    fn identity_substitution_cannot_advance_a_contiguous_prefix() {
        let mut download =
            CapabilityDownloadMachine::new(BoardCapabilityLimits::interactive()).unwrap();
        let first = download.next_request().unwrap().unwrap();
        let first_response = simulated_response(&first.body);
        download
            .accept_response(&Response {
                status: first_response.status,
                body: first_response.body,
            })
            .unwrap();
        let progress = download.progress();
        let second = download.next_request().unwrap().unwrap();
        let request = CapabilityReadRequest::decode(&second.body).unwrap();
        let foreign = CapabilityReadResponse {
            identity: CapabilityIdentity {
                byte_len: progress.identity.unwrap().byte_len,
                digest: Digest([0xa5; 32]),
            },
            offset: request.offset,
            chunk_len: 1,
            complete: false,
        };
        let mut body = Vec::from(foreign.encode().unwrap());
        body.push(0);

        assert_eq!(
            download.accept_response(&Response {
                status: StatusCode::Ok,
                body,
            }),
            Err(CapabilityDownloadError::Identity)
        );
        assert_eq!(download.progress(), progress);
    }

    #[test]
    fn declared_document_limit_is_checked_before_allocation() {
        let mut limits = BoardCapabilityLimits::interactive();
        limits.maximum_document_bytes = 512;
        let mut download = CapabilityDownloadMachine::new(limits).unwrap();
        let _ = download.next_request().unwrap().unwrap();
        let prefix = CapabilityReadResponse {
            identity: CapabilityIdentity {
                byte_len: 513,
                digest: Digest([7; 32]),
            },
            offset: 0,
            chunk_len: 1,
            complete: false,
        }
        .encode()
        .unwrap();
        let mut body = Vec::with_capacity(CAPABILITY_READ_RESPONSE_PREFIX_BYTES + 1);
        body.extend_from_slice(&prefix);
        body.push(0);

        assert_eq!(
            download.accept_response(&Response {
                status: StatusCode::Ok,
                body,
            }),
            Err(CapabilityDownloadError::Limit)
        );
        assert_eq!(download.phase(), CapabilityDownloadPhase::Discovering);
    }

    #[test]
    fn device_failure_must_have_an_empty_body() {
        let mut download =
            CapabilityDownloadMachine::new(BoardCapabilityLimits::interactive()).unwrap();
        let _ = download.next_request().unwrap().unwrap();
        assert_eq!(
            download.accept_response(&Response {
                status: StatusCode::Unsupported,
                body: Vec::new(),
            }),
            Err(CapabilityDownloadError::DeviceStatus(
                StatusCode::Unsupported
            ))
        );

        let _ = download.next_request().unwrap().unwrap();
        assert_eq!(
            download.accept_response(&Response {
                status: StatusCode::Unsupported,
                body: vec![1],
            }),
            Err(CapabilityDownloadError::ResponseBody)
        );
    }
}
