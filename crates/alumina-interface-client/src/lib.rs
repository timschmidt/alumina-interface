//! Headless protocol framing shared by native tests, the browser, and simulators.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use core::fmt;

use alumina_protocol::{
    DeviceCycle, Digest, FrameHeader, HeaderError, MessageDirection, MessageError, MessageHeader,
    Operation, StatusCode,
};

pub mod clock;
pub mod diagnostics;
pub mod graph;
pub mod health;
pub mod http;
pub mod schedule;
pub mod upload;
#[cfg(target_arch = "wasm32")]
pub mod wasm;
pub mod worker;

/// Transport-independent maximum accepted by this first headless client.
pub const MAXIMUM_PAYLOAD_BYTES: u32 = 64 * 1024;

/// A request/response byte transport, implemented later by native Wi-Fi and browser fetch/socket IO.
pub trait Transport {
    /// Transport-specific failure.
    type Error;

    /// Exchange exactly one framed request for exactly one framed response.
    fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>, Self::Error>;
}

/// One decoded and correlated protocol response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    /// Typed protocol status.
    pub status: StatusCode,
    /// Operation-specific canonical response bytes.
    pub body: Vec<u8>,
}

/// Headless native Alumina V1 client over an injected byte transport.
#[derive(Debug)]
pub struct ProtocolClient<T> {
    transport: T,
    next_sequence: u32,
    next_correlation: u32,
    config_digest: Digest,
}

impl<T: Transport> ProtocolClient<T> {
    /// Bind a transport and the active configuration identity.
    pub const fn new(transport: T, config_digest: Digest) -> Self {
        Self {
            transport,
            next_sequence: 0,
            next_correlation: 1,
            config_digest,
        }
    }

    /// Borrow the underlying transport for deterministic inspection.
    pub const fn transport(&self) -> &T {
        &self.transport
    }

    /// Mutably borrow the transport for deterministic simulator publication.
    pub const fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Send one typed operation with an opaque canonical body.
    pub fn request(
        &mut self,
        operation: Operation,
        body: &[u8],
    ) -> Result<Response, ClientError<T::Error>> {
        let correlation = self.next_correlation;
        let sequence = self.next_sequence;
        let request = encode_request(operation, body, sequence, correlation, self.config_digest)
            .map_err(ClientError::Request)?;

        let response = self
            .transport
            .exchange(&request)
            .map_err(ClientError::Transport)?;
        let decoded = decode_response(
            &response,
            operation,
            sequence,
            correlation,
            self.config_digest,
        )
        .map_err(ClientError::Response)?;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.next_correlation = self.next_correlation.wrapping_add(1).max(1);
        Ok(decoded)
    }
}

fn encode_request(
    operation: Operation,
    body: &[u8],
    sequence: u32,
    correlation: u32,
    config_digest: Digest,
) -> Result<Vec<u8>, RequestEncodeError> {
    let body_len = u32::try_from(body.len()).map_err(|_| RequestEncodeError::RequestTooLarge)?;
    let payload_len = u32::try_from(MessageHeader::WIRE_LEN)
        .expect("message header length fits u32")
        .checked_add(body_len)
        .filter(|length| *length <= MAXIMUM_PAYLOAD_BYTES)
        .ok_or(RequestEncodeError::RequestTooLarge)?;
    let message = MessageHeader::request(operation, correlation, body_len);
    message
        .validate(operation.frame_kind(), payload_len)
        .map_err(RequestEncodeError::Message)?;
    let frame = FrameHeader::new(
        operation.frame_kind(),
        payload_len,
        sequence,
        DeviceCycle(0),
        config_digest,
    );
    let mut request = Vec::with_capacity(FrameHeader::WIRE_LEN + payload_len as usize);
    request.extend_from_slice(&frame.encode());
    request.extend_from_slice(&message.encode());
    request.extend_from_slice(body);
    Ok(request)
}

fn decode_response(
    encoded: &[u8],
    operation: Operation,
    sequence: u32,
    correlation: u32,
    config_digest: Digest,
) -> Result<Response, ResponseDecodeError> {
    let minimum = FrameHeader::WIRE_LEN + MessageHeader::WIRE_LEN;
    if encoded.len() < minimum {
        return Err(ResponseDecodeError::TruncatedResponse);
    }
    let frame = FrameHeader::decode(&encoded[..FrameHeader::WIRE_LEN], MAXIMUM_PAYLOAD_BYTES)
        .map_err(ResponseDecodeError::Frame)?;
    let expected_len = FrameHeader::WIRE_LEN
        .checked_add(frame.payload_len as usize)
        .ok_or(ResponseDecodeError::TruncatedResponse)?;
    if encoded.len() != expected_len {
        return Err(ResponseDecodeError::ResponseLength);
    }
    if frame.sequence != sequence {
        return Err(ResponseDecodeError::Sequence);
    }
    if frame.config_digest != config_digest {
        return Err(ResponseDecodeError::ConfigurationIdentity);
    }
    let message_start = FrameHeader::WIRE_LEN;
    let message_end = message_start + MessageHeader::WIRE_LEN;
    let message = MessageHeader::decode_and_validate(
        &encoded[message_start..message_end],
        frame.kind,
        frame.payload_len,
    )
    .map_err(ResponseDecodeError::Message)?;
    if message.direction != MessageDirection::Response
        || message.operation != operation
        || message.correlation_id != correlation
    {
        return Err(ResponseDecodeError::Correlation);
    }
    Ok(Response {
        status: message.status,
        body: encoded[message_end..].to_vec(),
    })
}

/// Failure while constructing a canonical native request frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestEncodeError {
    /// Request body exceeded the client payload budget.
    RequestTooLarge,
    /// The typed message header rejected the requested operation/body pairing.
    Message(MessageError),
}

impl fmt::Display for RequestEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestTooLarge => formatter.write_str("request exceeds the payload budget"),
            Self::Message(error) => write!(formatter, "invalid request message: {error:?}"),
        }
    }
}

impl std::error::Error for RequestEncodeError {}

/// Failure while validating one exact correlated native response frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponseDecodeError {
    /// Response did not contain both fixed headers.
    TruncatedResponse,
    /// Outer response length did not match the exact received bytes.
    ResponseLength,
    /// Universal frame validation failed.
    Frame(HeaderError),
    /// Typed message validation failed.
    Message(MessageError),
    /// Response sequence did not match the exact request.
    Sequence,
    /// Operation or correlation identity did not match the request.
    Correlation,
    /// Response was compiled or sampled against another configuration.
    ConfigurationIdentity,
}

impl fmt::Display for ResponseDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedResponse => formatter.write_str("response is truncated"),
            Self::ResponseLength => formatter.write_str("response length is not canonical"),
            Self::Frame(error) => write!(formatter, "invalid frame header: {error:?}"),
            Self::Message(error) => write!(formatter, "invalid message header: {error:?}"),
            Self::Sequence => formatter.write_str("response sequence does not match request"),
            Self::Correlation => formatter.write_str("response correlation does not match request"),
            Self::ConfigurationIdentity => {
                formatter.write_str("response configuration identity does not match request")
            }
        }
    }
}

impl std::error::Error for ResponseDecodeError {}

/// Failure before a response may influence client state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientError<E> {
    /// Underlying transport failed.
    Transport(E),
    /// Canonical request encoding failed before transport.
    Request(RequestEncodeError),
    /// Correlated canonical response validation failed after transport.
    Response(ResponseDecodeError),
}

impl<E: fmt::Display> fmt::Display for ClientError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "transport failed: {error}"),
            Self::Request(error) => write!(formatter, "request encoding failed: {error}"),
            Self::Response(error) => write!(formatter, "response validation failed: {error}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for ClientError<E> {}

/// Deterministic operation handler used by the simulator transport.
pub trait SimulatorHandler {
    /// Produce a typed response for one validated request.
    fn respond(&mut self, operation: Operation, body: &[u8]) -> SimulatedResponse;
}

impl<F> SimulatorHandler for F
where
    F: FnMut(Operation, &[u8]) -> SimulatedResponse,
{
    fn respond(&mut self, operation: Operation, body: &[u8]) -> SimulatedResponse {
        self(operation, body)
    }
}

/// Deterministic simulated response selected by a headless fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulatedResponse {
    /// Typed status returned by the simulator.
    pub status: StatusCode,
    /// Operation-specific canonical body.
    pub body: Vec<u8>,
}

/// In-memory simulator transport that uses the same bytes as a native client.
#[derive(Debug)]
pub struct SimulatorTransport<H> {
    handler: H,
    request_count: u32,
}

impl<H> SimulatorTransport<H> {
    /// Construct a deterministic simulator transport.
    pub const fn new(handler: H) -> Self {
        Self {
            handler,
            request_count: 0,
        }
    }

    /// Number of completely validated requests observed.
    pub const fn request_count(&self) -> u32 {
        self.request_count
    }
}

impl<H: SimulatorHandler> Transport for SimulatorTransport<H> {
    type Error = SimulatorError;

    fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>, Self::Error> {
        let minimum = FrameHeader::WIRE_LEN + MessageHeader::WIRE_LEN;
        if request.len() < minimum {
            return Err(SimulatorError::TruncatedRequest);
        }
        let frame = FrameHeader::decode(&request[..FrameHeader::WIRE_LEN], MAXIMUM_PAYLOAD_BYTES)
            .map_err(SimulatorError::Frame)?;
        if request.len() != FrameHeader::WIRE_LEN + frame.payload_len as usize {
            return Err(SimulatorError::RequestLength);
        }
        let message_start = FrameHeader::WIRE_LEN;
        let message_end = message_start + MessageHeader::WIRE_LEN;
        let message = MessageHeader::decode_and_validate(
            &request[message_start..message_end],
            frame.kind,
            frame.payload_len,
        )
        .map_err(SimulatorError::Message)?;
        if message.direction != MessageDirection::Request {
            return Err(SimulatorError::NotARequest);
        }
        let response = self
            .handler
            .respond(message.operation, &request[message_end..]);
        let body_len =
            u32::try_from(response.body.len()).map_err(|_| SimulatorError::ResponseTooLarge)?;
        let payload_len = u32::try_from(MessageHeader::WIRE_LEN)
            .expect("message header length fits u32")
            .checked_add(body_len)
            .filter(|length| *length <= MAXIMUM_PAYLOAD_BYTES)
            .ok_or(SimulatorError::ResponseTooLarge)?;
        let response_message = MessageHeader::response(
            message.operation,
            message.correlation_id,
            response.status,
            body_len,
        );
        response_message
            .validate(frame.kind, payload_len)
            .map_err(SimulatorError::Message)?;
        let response_frame = FrameHeader::new(
            frame.kind,
            payload_len,
            frame.sequence,
            frame.cycle,
            frame.config_digest,
        );
        let mut encoded = Vec::with_capacity(FrameHeader::WIRE_LEN + payload_len as usize);
        encoded.extend_from_slice(&response_frame.encode());
        encoded.extend_from_slice(&response_message.encode());
        encoded.extend_from_slice(&response.body);
        self.request_count = self.request_count.saturating_add(1);
        Ok(encoded)
    }
}

/// Rejection by the deterministic simulator transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimulatorError {
    /// Request did not contain both fixed headers.
    TruncatedRequest,
    /// Outer request length did not match the exact bytes.
    RequestLength,
    /// Universal frame validation failed.
    Frame(HeaderError),
    /// Typed message validation failed.
    Message(MessageError),
    /// Simulator transports accept requests, not responses or events.
    NotARequest,
    /// Handler produced a response beyond the bounded client budget.
    ResponseTooLarge,
}

impl fmt::Display for SimulatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for SimulatorError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_client_and_simulator_share_canonical_bytes() {
        let digest = Digest([0x5a; 32]);
        let simulator = SimulatorTransport::new(|operation, body: &[u8]| {
            assert_eq!(operation, Operation::IdentityGet);
            assert_eq!(body, b"who");
            SimulatedResponse {
                status: StatusCode::Ok,
                body: b"simulator-device".to_vec(),
            }
        });
        let mut client = ProtocolClient::new(simulator, digest);

        let response = client.request(Operation::IdentityGet, b"who").unwrap();

        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.body, b"simulator-device");
        assert_eq!(client.transport().request_count(), 1);
    }

    #[test]
    fn simulator_preserves_typed_failure_status() {
        let simulator = SimulatorTransport::new(|_, _: &[u8]| SimulatedResponse {
            status: StatusCode::ForbiddenState,
            body: Vec::new(),
        });
        let mut client = ProtocolClient::new(simulator, Digest([1; 32]));

        let response = client.request(Operation::JobCommit, &[]).unwrap();

        assert_eq!(response.status, StatusCode::ForbiddenState);
        assert!(response.body.is_empty());
    }
}
