//! Split-phase authenticated HTTP/native-protocol session for asynchronous Wi-Fi I/O.

use std::fmt;

use alumina_net::{
    AUTH_COUNTER_HEADER, AUTH_DISCOVERY_BODY_BYTES, AUTH_PROOF_HEADER, AUTH_RESPONSE_HEADER,
    AUTH_TAG_HEX_BYTES, AuthError, AuthenticatedMedia, BootNonce, CorsOrigin, HttpMethod,
    ResponseProof, parse_request_proof, sign_request, verify_response_proof, write_lower_hex,
};
use alumina_protocol::{Digest, Operation};
use serde::Deserialize;

use crate::{RequestEncodeError, Response, ResponseDecodeError, decode_response, encode_request};

/// Same-origin firmware route carrying one canonical native request/response.
pub const CONTROL_PATH: &str = "/api/v1/control";
/// Public firmware route returning the boot-scoped authentication challenge.
pub const AUTHENTICATION_PATH: &str = "/api/v1/auth";
/// Exact media type required by the firmware control route.
pub const NATIVE_FRAME_MEDIA_TYPE: &str = "application/vnd.alumina.frame";

const AUTHENTICATION_SCHEME: &str = "hmac-sha256-v2";

/// Validated boot-scoped firmware authentication discovery response.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AuthenticationChallenge {
    nonce: BootNonce,
}

impl AuthenticationChallenge {
    /// Public nonce mixed into each request and response proof for this boot.
    pub const fn nonce(self) -> BootNonce {
        self.nonce
    }
}

impl fmt::Debug for AuthenticationChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticationChallenge")
            .field("nonce", &"[redacted]")
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticationChallengeWire<'a> {
    scheme: &'a str,
    origin_bound: bool,
    boot_nonce: &'a str,
    counter_window: u8,
    rate_burst: u16,
    rate_per_second: u16,
    request_proof_header: &'a str,
    response_proof_header: &'a str,
}

/// Decodes the exact public authentication policy emitted by current firmware.
pub fn decode_authentication_challenge(
    bytes: &[u8],
) -> Result<AuthenticationChallenge, AuthenticationChallengeError> {
    if bytes.len() != AUTH_DISCOVERY_BODY_BYTES {
        return Err(AuthenticationChallengeError::Json);
    }
    let wire: AuthenticationChallengeWire<'_> =
        serde_json::from_slice(bytes).map_err(|_| AuthenticationChallengeError::Json)?;
    if wire.scheme != AUTHENTICATION_SCHEME || !wire.origin_bound {
        return Err(AuthenticationChallengeError::Scheme);
    }
    if wire.counter_window != 64
        || wire.rate_burst != 32
        || wire.rate_per_second != 50
        || wire.request_proof_header != AUTH_PROOF_HEADER
        || wire.response_proof_header != AUTH_RESPONSE_HEADER
    {
        return Err(AuthenticationChallengeError::Policy);
    }
    if wire.boot_nonce.len() != 32 {
        return Err(AuthenticationChallengeError::Nonce);
    }
    let mut nonce = [0_u8; 16];
    for (index, byte) in nonce.iter_mut().enumerate() {
        let high = decode_lower_hex(wire.boot_nonce.as_bytes()[index * 2])
            .ok_or(AuthenticationChallengeError::Nonce)?;
        let low = decode_lower_hex(wire.boot_nonce.as_bytes()[index * 2 + 1])
            .ok_or(AuthenticationChallengeError::Nonce)?;
        *byte = (high << 4) | low;
    }
    Ok(AuthenticationChallenge {
        nonce: BootNonce::new(nonce).map_err(|_| AuthenticationChallengeError::Nonce)?,
    })
}

const fn decode_lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Invalid or incompatible public authentication discovery response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthenticationChallengeError {
    /// Body was not the single current JSON schema.
    Json,
    /// Scheme or required origin binding did not match this client.
    Scheme,
    /// Replay/rate/header policy did not match the compiled client contract.
    Policy,
    /// Boot nonce was zero, incorrectly sized, or not lowercase hexadecimal.
    Nonce,
}

impl fmt::Display for AuthenticationChallengeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json => formatter.write_str("authentication challenge JSON is not canonical"),
            Self::Scheme => formatter.write_str("authentication challenge scheme is unsupported"),
            Self::Policy => formatter.write_str("authentication challenge policy is incompatible"),
            Self::Nonce => formatter.write_str("authentication challenge nonce is invalid"),
        }
    }
}

impl std::error::Error for AuthenticationChallengeError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingRequest {
    operation: Operation,
    sequence: u32,
    correlation: u32,
    http_counter: u64,
}

/// One fully authenticated HTTP request ready for native or browser async I/O.
pub struct AuthenticatedProtocolRequest {
    counter: u64,
    authorization: [u8; AUTH_TAG_HEX_BYTES],
    body: Vec<u8>,
}

impl AuthenticatedProtocolRequest {
    /// Canonical `POST` path covered by the request proof.
    pub const fn path(&self) -> &'static str {
        CONTROL_PATH
    }

    /// Exact request content type covered by server admission policy.
    pub const fn content_type(&self) -> &'static str {
        NATIVE_FRAME_MEDIA_TYPE
    }

    /// Header name carrying the canonical decimal request counter.
    pub const fn counter_header_name(&self) -> &'static str {
        AUTH_COUNTER_HEADER
    }

    /// Nonzero boot-scoped request counter.
    pub const fn counter(&self) -> u64 {
        self.counter
    }

    /// Canonical decimal counter text for an HTTP header value.
    pub fn counter_header_value(&self) -> String {
        self.counter.to_string()
    }

    /// Header name carrying the lowercase request HMAC.
    pub const fn authorization_header_name(&self) -> &'static str {
        AUTH_PROOF_HEADER
    }

    /// Exact lowercase request HMAC header value.
    pub fn authorization_header_value(&self) -> &str {
        std::str::from_utf8(&self.authorization)
            .expect("canonical lowercase hexadecimal proof is UTF-8")
    }

    /// Exact canonical native frame used as the HTTP body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

impl fmt::Debug for AuthenticatedProtocolRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedProtocolRequest")
            .field("counter", &self.counter)
            .field("authorization", &"[redacted]")
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Exact transport facts observed from one authenticated HTTP response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthenticatedProtocolResponse<'a> {
    /// HTTP response status covered by the response proof.
    pub http_status: u16,
    /// Parsed exact response representation covered by the response proof.
    pub media: AuthenticatedMedia,
    /// `X-Alumina-Counter` value observed without normalization.
    pub counter_header: &'a str,
    /// `X-Alumina-Response-Authorization` value observed without normalization.
    pub authorization_header: &'a str,
    /// Exact response body bytes.
    pub body: &'a [u8],
}

impl AuthenticatedProtocolResponse<'_> {
    /// Header name from which [`Self::counter_header`] must be read.
    pub const fn counter_header_name() -> &'static str {
        AUTH_COUNTER_HEADER
    }

    /// Header name from which [`Self::authorization_header`] must be read.
    pub const fn authorization_header_name() -> &'static str {
        AUTH_RESPONSE_HEADER
    }
}

/// Boot-scoped protocol/HMAC state independent of synchronous or asynchronous I/O.
pub struct AuthenticatedHttpSession {
    nonce: BootNonce,
    origin: CorsOrigin,
    next_http_counter: u64,
    next_sequence: u32,
    next_correlation: u32,
    config_digest: Digest,
    pending: Option<PendingRequest>,
}

impl AuthenticatedHttpSession {
    /// Starts a fresh boot-scoped session at counter/correlation one.
    pub const fn new(nonce: BootNonce, config_digest: Digest, origin: CorsOrigin) -> Self {
        Self {
            nonce,
            origin,
            next_http_counter: 1,
            next_sequence: 0,
            next_correlation: 1,
            config_digest,
            pending: None,
        }
    }

    /// Starts a fresh boot-scoped session at an explicit caller-selected counter.
    ///
    /// Browser workers use an epoch-prefixed seed so an ordinary page reload
    /// does not restart at counter one inside firmware's boot-global replay
    /// window. The counter is not a credential; HMAC authentication still
    /// covers it and every other canonical request fact.
    ///
    /// # Errors
    ///
    /// Rejects zero and `u64::MAX`, which cannot name a request and leave room
    /// for the next counter respectively.
    pub const fn starting_at(
        nonce: BootNonce,
        config_digest: Digest,
        origin: CorsOrigin,
        initial_counter: u64,
    ) -> Result<Self, HttpSessionError> {
        if initial_counter == 0 || initial_counter == u64::MAX {
            return Err(HttpSessionError::CounterSeed);
        }
        Ok(Self {
            nonce,
            origin,
            next_http_counter: initial_counter,
            next_sequence: 0,
            next_correlation: 1,
            config_digest,
            pending: None,
        })
    }

    /// Whether one request is currently awaiting a response or explicit abandonment.
    pub const fn has_pending_request(&self) -> bool {
        self.pending.is_some()
    }

    /// Exact calling-document origin bound into every request and response proof.
    pub const fn origin(&self) -> CorsOrigin {
        self.origin
    }

    /// Constructs and signs one request before browser/native I/O begins.
    ///
    /// The counter, sequence, and correlation are spent at construction. If I/O
    /// becomes ambiguous, call [`Self::abandon_pending`] and reconcile with a new
    /// request; never reuse the HMAC counter.
    pub fn begin_request(
        &mut self,
        operation: Operation,
        body: &[u8],
        secret: &[u8],
    ) -> Result<AuthenticatedProtocolRequest, HttpSessionError> {
        if self.pending.is_some() {
            return Err(HttpSessionError::RequestPending);
        }
        let counter = self.next_http_counter;
        let next_counter = counter
            .checked_add(1)
            .ok_or(HttpSessionError::CounterExhausted)?;
        let sequence = self.next_sequence;
        let correlation = self.next_correlation;
        let body = encode_request(operation, body, sequence, correlation, self.config_digest)?;
        let proof = sign_request(
            secret,
            self.nonce,
            counter,
            HttpMethod::Post,
            CONTROL_PATH,
            self.origin,
            &body,
        )?;
        let mut authorization = [0_u8; AUTH_TAG_HEX_BYTES];
        write_lower_hex(&proof.tag, &mut authorization)?;

        self.pending = Some(PendingRequest {
            operation,
            sequence,
            correlation,
            http_counter: counter,
        });
        self.next_http_counter = next_counter;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.next_correlation = self.next_correlation.wrapping_add(1).max(1);
        Ok(AuthenticatedProtocolRequest {
            counter,
            authorization,
            body,
        })
    }

    /// Authenticates and consumes one response before decoding its native frame.
    ///
    /// Unauthenticated input cannot mutate pending state. Once a response proof
    /// is valid, the request is consumed even if its HTTP media/status or native
    /// frame is malformed; callers then reconcile with a fresh request.
    pub fn accept_response(
        &mut self,
        response: AuthenticatedProtocolResponse<'_>,
        secret: &[u8],
    ) -> Result<Response, HttpSessionError> {
        let pending = self.pending.ok_or(HttpSessionError::NoPendingRequest)?;
        let parsed = parse_request_proof(response.counter_header, response.authorization_header)?;
        if parsed.counter != pending.http_counter {
            return Err(HttpSessionError::CounterMismatch {
                received: parsed.counter,
                expected: pending.http_counter,
            });
        }
        verify_response_proof(
            secret,
            self.nonce,
            ResponseProof {
                counter: parsed.counter,
                tag: parsed.tag,
            },
            response.http_status,
            response.media,
            self.origin,
            response.body,
        )?;

        self.pending = None;
        if response.http_status != 200 {
            return Err(HttpSessionError::HttpStatus(response.http_status));
        }
        if response.media != AuthenticatedMedia::NativeFrame {
            return Err(HttpSessionError::Media(response.media));
        }
        decode_response(
            response.body,
            pending.operation,
            pending.sequence,
            pending.correlation,
            self.config_digest,
        )
        .map_err(HttpSessionError::Response)
    }

    /// Resolves an ambiguous/failed transport without reusing its spent counter.
    pub fn abandon_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }

    /// Rebinds all boot/config-scoped state after fetching a new public challenge.
    pub fn rebind_boot(
        &mut self,
        nonce: BootNonce,
        config_digest: Digest,
    ) -> Result<(), HttpSessionError> {
        if self.pending.is_some() {
            return Err(HttpSessionError::RequestPending);
        }
        *self = Self::new(nonce, config_digest, self.origin);
        Ok(())
    }
}

impl fmt::Debug for AuthenticatedHttpSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedHttpSession")
            .field("nonce", &"[redacted]")
            .field("origin", &self.origin)
            .field("next_http_counter", &self.next_http_counter)
            .field("next_sequence", &self.next_sequence)
            .field("next_correlation", &self.next_correlation)
            .field("config_digest", &self.config_digest)
            .field("pending", &self.pending)
            .finish()
    }
}

/// Failure in the split-phase HTTP authentication/protocol boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpSessionError {
    /// Another request must be accepted or abandoned first.
    RequestPending,
    /// No request exists for the supplied response.
    NoPendingRequest,
    /// The boot-scoped request counter cannot advance without reuse.
    CounterExhausted,
    /// An explicit session seed was zero or left no room to advance.
    CounterSeed,
    /// Canonical native request encoding failed.
    Request(RequestEncodeError),
    /// Request/response HMAC syntax or verification failed.
    Authentication(AuthError),
    /// Signed response counter did not name the pending request.
    CounterMismatch {
        /// Counter carried by the signed response.
        received: u64,
        /// Counter spent by the pending request.
        expected: u64,
    },
    /// Authenticated transport status was not the native-control success status.
    HttpStatus(u16),
    /// Authenticated response used another representation.
    Media(AuthenticatedMedia),
    /// Authenticated native response validation failed.
    Response(ResponseDecodeError),
}

impl From<RequestEncodeError> for HttpSessionError {
    fn from(value: RequestEncodeError) -> Self {
        Self::Request(value)
    }
}

impl From<AuthError> for HttpSessionError {
    fn from(value: AuthError) -> Self {
        Self::Authentication(value)
    }
}

impl fmt::Display for HttpSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestPending => formatter.write_str("an HTTP request is already pending"),
            Self::NoPendingRequest => formatter.write_str("no HTTP request is pending"),
            Self::CounterExhausted => formatter.write_str("HTTP request counter is exhausted"),
            Self::CounterSeed => formatter.write_str("HTTP request counter seed is invalid"),
            Self::Request(error) => write!(formatter, "native request encoding failed: {error}"),
            Self::Authentication(error) => {
                write!(formatter, "HTTP authentication failed: {error:?}")
            }
            Self::CounterMismatch { received, expected } => write!(
                formatter,
                "HTTP response counter {received} does not match pending counter {expected}"
            ),
            Self::HttpStatus(status) => {
                write!(
                    formatter,
                    "authenticated HTTP response returned status {status}"
                )
            }
            Self::Media(media) => {
                write!(
                    formatter,
                    "authenticated HTTP response used {media:?} media"
                )
            }
            Self::Response(error) => write!(formatter, "native response rejected: {error}"),
        }
    }
}

impl std::error::Error for HttpSessionError {}

#[cfg(test)]
mod tests {
    use alumina_net::{RequestProof, sign_response, verify_request_proof};
    use alumina_protocol::StatusCode;

    use super::*;
    use crate::{SimulatedResponse, SimulatorTransport, Transport};

    const SECRET: &[u8] = b"correct horse battery staple";

    fn nonce(byte: u8) -> BootNonce {
        BootNonce::new([byte; 16]).unwrap()
    }

    fn origin() -> CorsOrigin {
        CorsOrigin::parse("http://alumina-ui.local").unwrap()
    }

    fn response_tag(
        nonce: BootNonce,
        counter: u64,
        status: u16,
        media: AuthenticatedMedia,
        body: &[u8],
    ) -> String {
        let proof = sign_response(SECRET, nonce, counter, status, media, origin(), body).unwrap();
        let mut encoded = [0_u8; AUTH_TAG_HEX_BYTES];
        write_lower_hex(&proof.tag, &mut encoded).unwrap();
        std::str::from_utf8(&encoded).unwrap().to_owned()
    }

    #[test]
    fn public_authentication_challenge_is_strict_and_boot_scoped() {
        let bytes = br#"{"scheme":"hmac-sha256-v2","origin_bound":true,"boot_nonce":"00112233445566778899aabbccddeeff","counter_window":64,"rate_burst":32,"rate_per_second":50,"request_proof_header":"X-Alumina-Authorization","response_proof_header":"X-Alumina-Response-Authorization"}"#;
        assert_eq!(bytes.len(), AUTH_DISCOVERY_BODY_BYTES);
        let challenge = decode_authentication_challenge(bytes).unwrap();
        assert_eq!(
            challenge.nonce().as_bytes(),
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ]
        );

        let nonce_start = bytes
            .windows(b"\"boot_nonce\":\"".len())
            .position(|window| window == b"\"boot_nonce\":\"")
            .unwrap()
            + b"\"boot_nonce\":\"".len();
        let mut uppercase = bytes.to_vec();
        uppercase[nonce_start + 20] = b'A';
        assert_eq!(
            decode_authentication_challenge(&uppercase),
            Err(AuthenticationChallengeError::Nonce)
        );

        let changed_policy = bytes
            .windows(b"\"counter_window\":64".len())
            .position(|window| window == b"\"counter_window\":64")
            .unwrap();
        let mut changed_policy_bytes = bytes.to_vec();
        changed_policy_bytes[changed_policy + b"\"counter_window\":".len()] = b'6';
        changed_policy_bytes[changed_policy + b"\"counter_window\":".len() + 1] = b'3';
        assert_eq!(
            decode_authentication_challenge(&changed_policy_bytes),
            Err(AuthenticationChallengeError::Policy)
        );
    }

    #[test]
    fn split_phase_request_and_response_share_exact_firmware_authentication() {
        let boot = nonce(0x31);
        let config = Digest([0x52; 32]);
        let mut session = AuthenticatedHttpSession::new(boot, config, origin());
        let request = session
            .begin_request(Operation::StorageInspect, b"publication", SECRET)
            .unwrap();

        let parsed = parse_request_proof(
            &request.counter_header_value(),
            request.authorization_header_value(),
        )
        .unwrap();
        verify_request_proof(
            SECRET,
            boot,
            RequestProof {
                counter: parsed.counter,
                tag: parsed.tag,
            },
            HttpMethod::Post,
            CONTROL_PATH,
            origin(),
            request.body(),
        )
        .unwrap();

        let mut transport = SimulatorTransport::new(|operation, body: &[u8]| {
            assert_eq!(operation, Operation::StorageInspect);
            assert_eq!(body, b"publication");
            SimulatedResponse {
                status: StatusCode::Ok,
                body: b"present".to_vec(),
            }
        });
        let native = transport.exchange(request.body()).unwrap();
        let tag = response_tag(
            boot,
            request.counter(),
            200,
            AuthenticatedMedia::NativeFrame,
            &native,
        );
        let response = session
            .accept_response(
                AuthenticatedProtocolResponse {
                    http_status: 200,
                    media: AuthenticatedMedia::NativeFrame,
                    counter_header: &request.counter_header_value(),
                    authorization_header: &tag,
                    body: &native,
                },
                SECRET,
            )
            .unwrap();
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.body, b"present");
        assert!(!session.has_pending_request());
    }

    #[test]
    fn unauthenticated_bytes_cannot_consume_pending_state_and_loss_spends_counter() {
        let boot = nonce(0x41);
        let mut session = AuthenticatedHttpSession::new(boot, Digest([0x62; 32]), origin());
        let first = session
            .begin_request(Operation::StorageFinalize, b"finalize", SECRET)
            .unwrap();
        let original = first.body().to_vec();
        let tag = response_tag(
            boot,
            first.counter(),
            200,
            AuthenticatedMedia::NativeFrame,
            &original,
        );
        let mut tampered = original;
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(
            session.accept_response(
                AuthenticatedProtocolResponse {
                    http_status: 200,
                    media: AuthenticatedMedia::NativeFrame,
                    counter_header: &first.counter_header_value(),
                    authorization_header: &tag,
                    body: &tampered,
                },
                SECRET,
            ),
            Err(HttpSessionError::Authentication(AuthError::Unauthorized))
        );
        assert!(session.has_pending_request());
        assert!(session.abandon_pending());

        let second = session
            .begin_request(Operation::StorageInspect, b"inspect", SECRET)
            .unwrap();
        assert_eq!(first.counter(), 1);
        assert_eq!(second.counter(), 2);
    }

    #[test]
    fn explicit_counter_seed_advances_across_browser_worker_replacement() {
        let boot = nonce(0x71);
        assert!(matches!(
            AuthenticatedHttpSession::starting_at(boot, Digest::ZERO, origin(), 0),
            Err(HttpSessionError::CounterSeed)
        ));
        assert!(matches!(
            AuthenticatedHttpSession::starting_at(boot, Digest::ZERO, origin(), u64::MAX),
            Err(HttpSessionError::CounterSeed)
        ));

        let mut session = AuthenticatedHttpSession::starting_at(
            boot,
            Digest::ZERO,
            origin(),
            1_770_000_000_123_456_789,
        )
        .unwrap();
        let first = session
            .begin_request(Operation::ClockHeartbeat, b"clock", SECRET)
            .unwrap();
        assert_eq!(first.counter(), 1_770_000_000_123_456_789);
        assert!(session.abandon_pending());
        let second = session
            .begin_request(Operation::ClockHeartbeat, b"clock", SECRET)
            .unwrap();
        assert_eq!(second.counter(), first.counter() + 1);
    }
}
