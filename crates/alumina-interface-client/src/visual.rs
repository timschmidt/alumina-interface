//! Retry-safe acquisition of immutable capability-declared visual assets.

use core::fmt;

use alumina_capability::{
    MAX_VISUAL_ASSET_CHUNK_BYTES, VisualAssetIdentity, VisualAssetReadRequest,
    VisualAssetReadResponse,
};
use alumina_protocol::{Digest, Operation, StatusCode};
use alumina_storage::sha256;

use crate::Response;

/// One side-effect-free visual range ready for authenticated native framing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualAssetOperation {
    /// Always [`Operation::CapabilityVisualGet`].
    pub operation: Operation,
    /// Canonical fixed range-request body.
    pub body: Vec<u8>,
}

/// Exact acquisition phase retained across ambiguous network outcomes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualAssetDownloadPhase {
    /// No authenticated response has established the complete byte length.
    Discovering,
    /// A stable identity is known and a contiguous prefix is retained.
    Downloading,
    /// Every byte was received and independently matched the declared SHA-256.
    Complete,
}

/// Bounded progress facts safe to project into worker diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualAssetDownloadProgress {
    /// Exact acquisition phase.
    pub phase: VisualAssetDownloadPhase,
    /// Contiguous transport prefix already retained.
    pub received_bytes: u32,
    /// Stable complete asset identity after the first accepted range.
    pub identity: Option<VisualAssetIdentity>,
}

/// Retry-safe contiguous downloader for one exact capability visual digest.
#[derive(Debug)]
pub struct VisualAssetDownloadMachine {
    capability_digest: Digest,
    expected_asset_digest: Digest,
    maximum_asset_bytes: u32,
    identity: Option<VisualAssetIdentity>,
    bytes: Vec<u8>,
    received_bytes: u32,
    complete: bool,
    pending: Option<VisualAssetReadRequest>,
}

impl VisualAssetDownloadMachine {
    /// Creates an empty downloader under an explicit per-asset memory bound.
    ///
    /// # Errors
    ///
    /// Rejects zero identities, a zero byte bound, or a bound not representable
    /// by this client process.
    pub fn new(
        capability_digest: Digest,
        expected_asset_digest: Digest,
        maximum_asset_bytes: u32,
    ) -> Result<Self, VisualAssetDownloadError> {
        if capability_digest.is_zero()
            || expected_asset_digest.is_zero()
            || maximum_asset_bytes == 0
            || usize::try_from(maximum_asset_bytes).is_err()
        {
            return Err(VisualAssetDownloadError::Limit);
        }
        Ok(Self {
            capability_digest,
            expected_asset_digest,
            maximum_asset_bytes,
            identity: None,
            bytes: Vec::new(),
            received_bytes: 0,
            complete: false,
            pending: None,
        })
    }

    /// Exact capability digest that granted this visual read.
    pub const fn capability_digest(&self) -> Digest {
        self.capability_digest
    }

    /// Exact asset digest selected from that capability.
    pub const fn expected_asset_digest(&self) -> Digest {
        self.expected_asset_digest
    }

    /// Current acquisition phase.
    pub const fn phase(&self) -> VisualAssetDownloadPhase {
        if self.complete {
            VisualAssetDownloadPhase::Complete
        } else if self.identity.is_some() {
            VisualAssetDownloadPhase::Downloading
        } else {
            VisualAssetDownloadPhase::Discovering
        }
    }

    /// Current contiguous progress and pinned complete identity.
    pub const fn progress(&self) -> VisualAssetDownloadProgress {
        VisualAssetDownloadProgress {
            phase: self.phase(),
            received_bytes: self.received_bytes,
            identity: self.identity,
        }
    }

    /// Complete independently hash-verified bytes.
    pub fn asset(&self) -> Option<&[u8]> {
        self.complete.then_some(self.bytes.as_slice())
    }

    /// Emits the unique next contiguous range request.
    pub fn next_request(
        &mut self,
    ) -> Result<Option<VisualAssetOperation>, VisualAssetDownloadError> {
        if self.pending.is_some() {
            return Err(VisualAssetDownloadError::RequestPending);
        }
        if self.complete {
            return Ok(None);
        }
        let remaining = self.identity.map_or(
            u32::try_from(MAX_VISUAL_ASSET_CHUNK_BYTES).expect("visual chunk limit fits u32"),
            |identity| identity.byte_len.saturating_sub(self.received_bytes),
        );
        if remaining == 0 {
            return Err(VisualAssetDownloadError::Range);
        }
        let maximum_bytes = u16::try_from(remaining.min(
            u32::try_from(MAX_VISUAL_ASSET_CHUNK_BYTES).expect("visual chunk limit fits u32"),
        ))
        .expect("bounded visual range fits u16");
        let request = VisualAssetReadRequest {
            capability_digest: self.capability_digest,
            asset_digest: self.expected_asset_digest,
            offset: self.received_bytes,
            maximum_bytes,
        };
        self.pending = Some(request);
        Ok(Some(VisualAssetOperation {
            operation: Operation::CapabilityVisualGet,
            body: request
                .encode()
                .map_err(VisualAssetDownloadError::Wire)?
                .to_vec(),
        }))
    }

    /// Applies one already authenticated and correlated range response.
    pub fn accept_response(&mut self, response: &Response) -> Result<(), VisualAssetDownloadError> {
        let request = self
            .pending
            .take()
            .ok_or(VisualAssetDownloadError::NoPendingRequest)?;
        if response.status != StatusCode::Ok {
            if response.body.is_empty() {
                return Err(VisualAssetDownloadError::DeviceStatus(response.status));
            }
            return Err(VisualAssetDownloadError::ResponseBody);
        }
        let (metadata, chunk) = VisualAssetReadResponse::decode_body(&response.body)
            .map_err(VisualAssetDownloadError::Wire)?;
        if metadata.capability_digest != self.capability_digest
            || metadata.asset.digest != self.expected_asset_digest
            || metadata.offset != request.offset
            || metadata.offset != self.received_bytes
            || metadata.chunk_len > request.maximum_bytes
        {
            return Err(VisualAssetDownloadError::Identity);
        }
        if metadata.asset.byte_len > self.maximum_asset_bytes {
            return Err(VisualAssetDownloadError::Limit);
        }
        if let Some(identity) = self.identity
            && metadata.asset != identity
        {
            return Err(VisualAssetDownloadError::Identity);
        }

        let total = usize::try_from(metadata.asset.byte_len)
            .map_err(|_| VisualAssetDownloadError::Limit)?;
        if self.identity.is_none() {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(total)
                .map_err(|_| VisualAssetDownloadError::Allocation)?;
            bytes.resize(total, 0);
            self.bytes = bytes;
            self.identity = Some(metadata.asset);
        }
        let start =
            usize::try_from(self.received_bytes).map_err(|_| VisualAssetDownloadError::Range)?;
        let end = start
            .checked_add(chunk.len())
            .filter(|end| *end <= self.bytes.len())
            .ok_or(VisualAssetDownloadError::Range)?;
        self.bytes[start..end].copy_from_slice(chunk);
        let received_bytes = u32::try_from(end).map_err(|_| VisualAssetDownloadError::Range)?;

        if metadata.complete {
            if received_bytes != metadata.asset.byte_len {
                return Err(VisualAssetDownloadError::Range);
            }
            if sha256(&self.bytes).digest != self.expected_asset_digest {
                return Err(VisualAssetDownloadError::Digest);
            }
            self.received_bytes = received_bytes;
            self.complete = true;
        } else {
            if received_bytes >= metadata.asset.byte_len {
                return Err(VisualAssetDownloadError::Range);
            }
            self.received_bytes = received_bytes;
        }
        Ok(())
    }

    /// Makes an ambiguous side-effect-free range eligible for exact retry.
    pub fn abandon_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }
}

/// Visual range, identity, bound, allocation, or content rejection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisualAssetDownloadError {
    /// Caller policy or a device-declared total cannot be represented.
    Limit,
    /// One request is already in flight.
    RequestPending,
    /// A response arrived without a pending request.
    NoPendingRequest,
    /// Canonical request or response decoding failed.
    Wire(alumina_capability::CapabilityWireError),
    /// Device returned a typed non-success status with no body.
    DeviceStatus(StatusCode),
    /// A non-success response carried an ambiguous body.
    ResponseBody,
    /// Capability, asset, total, offset, or request identity changed.
    Identity,
    /// Offset, length, or completion facts disagreed.
    Range,
    /// Bounded browser/native allocation failed.
    Allocation,
    /// Complete bytes did not match the capability-declared SHA-256.
    Digest,
}

impl fmt::Display for VisualAssetDownloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "visual asset download rejected: {self:?}")
    }
}

impl std::error::Error for VisualAssetDownloadError {}

#[cfg(test)]
mod tests {
    use alumina_capability::{VISUAL_ASSET_READ_RESPONSE_PREFIX_BYTES, VisualAssetReadResponse};

    use super::*;

    fn response(request: &VisualAssetOperation, complete_bytes: &[u8], offset: usize) -> Response {
        let request = VisualAssetReadRequest::decode(&request.body).unwrap();
        let end = offset
            .saturating_add(usize::from(request.maximum_bytes))
            .min(complete_bytes.len());
        let chunk = &complete_bytes[offset..end];
        let metadata = VisualAssetReadResponse {
            capability_digest: request.capability_digest,
            asset: VisualAssetIdentity {
                byte_len: u32::try_from(complete_bytes.len()).unwrap(),
                digest: sha256(complete_bytes).digest,
            },
            offset: u32::try_from(offset).unwrap(),
            chunk_len: u16::try_from(chunk.len()).unwrap(),
            complete: end == complete_bytes.len(),
        };
        let mut body = vec![0; VISUAL_ASSET_READ_RESPONSE_PREFIX_BYTES + chunk.len()];
        body[..VISUAL_ASSET_READ_RESPONSE_PREFIX_BYTES]
            .copy_from_slice(&metadata.encode().unwrap());
        body[VISUAL_ASSET_READ_RESPONSE_PREFIX_BYTES..].copy_from_slice(chunk);
        Response {
            status: StatusCode::Ok,
            body,
        }
    }

    #[test]
    fn contiguous_ranges_retry_and_hash_before_publication() {
        let bytes = vec![0x5a; MAX_VISUAL_ASSET_CHUNK_BYTES + 17];
        let digest = sha256(&bytes).digest;
        let mut download =
            VisualAssetDownloadMachine::new(Digest([0x44; 32]), digest, 1_024).unwrap();
        let first = download.next_request().unwrap().unwrap();
        assert!(download.abandon_pending());
        let retry = download.next_request().unwrap().unwrap();
        assert_eq!(retry, first);
        download
            .accept_response(&response(&retry, &bytes, 0))
            .unwrap();
        assert_eq!(download.phase(), VisualAssetDownloadPhase::Downloading);
        assert!(download.asset().is_none());
        let second = download.next_request().unwrap().unwrap();
        download
            .accept_response(&response(&second, &bytes, MAX_VISUAL_ASSET_CHUNK_BYTES))
            .unwrap();
        assert_eq!(download.phase(), VisualAssetDownloadPhase::Complete);
        assert_eq!(download.asset(), Some(bytes.as_slice()));
        assert!(download.next_request().unwrap().is_none());
    }

    #[test]
    fn substitution_and_complete_content_tamper_fail_closed() {
        let bytes = b"capability-bound visual";
        let digest = sha256(bytes).digest;
        let mut download =
            VisualAssetDownloadMachine::new(Digest([0x22; 32]), digest, 1_024).unwrap();
        let request = download.next_request().unwrap().unwrap();
        let mut wrong = response(&request, bytes, 0);
        wrong.body[32] ^= 1;
        assert_eq!(
            download.accept_response(&wrong),
            Err(VisualAssetDownloadError::Identity)
        );

        let mut download =
            VisualAssetDownloadMachine::new(Digest([0x22; 32]), digest, 1_024).unwrap();
        let request = download.next_request().unwrap().unwrap();
        let mut tampered = response(&request, bytes, 0);
        *tampered.body.last_mut().unwrap() ^= 1;
        assert_eq!(
            download.accept_response(&tampered),
            Err(VisualAssetDownloadError::Digest)
        );
        assert!(download.asset().is_none());
    }
}
