//! Browser `fetch` adapter for the authenticated split-phase protocol session.

use std::fmt;

use alumina_net::{AUTH_DISCOVERY_CONTENT_LENGTH, AuthenticatedMedia, CorsOrigin};
use alumina_protocol::Digest;
use js_sys::{ArrayBuffer, Reflect, Uint8Array};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Headers, Request, RequestCache, RequestCredentials, RequestInit, RequestMode, RequestRedirect,
    Url, Window,
};

use crate::Response;
use crate::http::{
    AUTHENTICATION_PATH, AuthenticatedHttpSession, AuthenticatedProtocolRequest,
    AuthenticatedProtocolResponse, AuthenticationChallenge, AuthenticationChallengeError,
    CONTROL_PATH, HttpSessionError, NATIVE_FRAME_MEDIA_TYPE, decode_authentication_challenge,
};
use crate::upload::{CacheUploadError, CacheUploadMachine, CacheUploadPhase, UploadSource};

const JSON_MEDIA_TYPE: &str = "application/json";

/// Canonical HTTP(S) origin for one directly connected Alumina MCU.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceOrigin {
    origin: String,
}

impl DeviceOrigin {
    /// Validates an absolute HTTP(S) origin without credentials, path, query, or fragment.
    pub fn parse(input: &str) -> Result<Self, BrowserFetchError> {
        let url = Url::new(input).map_err(javascript_error)?;
        if !matches!(url.protocol().as_str(), "http:" | "https:")
            || !url.username().is_empty()
            || !url.password().is_empty()
            || !matches!(url.pathname().as_str(), "" | "/")
            || !url.search().is_empty()
            || !url.hash().is_empty()
        {
            return Err(BrowserFetchError::DeviceOrigin);
        }
        let origin = url.origin();
        if origin == "null" || origin.is_empty() {
            return Err(BrowserFetchError::DeviceOrigin);
        }
        let origin_with_slash = format!("{origin}/");
        if input != origin && input != origin_with_slash {
            return Err(BrowserFetchError::DeviceOrigin);
        }
        Ok(Self { origin })
    }

    /// Canonical scheme/host/port without a trailing path.
    pub fn as_str(&self) -> &str {
        &self.origin
    }

    fn control_url(&self) -> String {
        format!("{}{CONTROL_PATH}", self.origin)
    }

    fn authentication_url(&self) -> String {
        format!("{}{AUTHENTICATION_PATH}", self.origin)
    }
}

/// Fetches and validates the device's public boot-scoped authentication policy.
pub async fn fetch_authentication_challenge(
    window: &Window,
    origin: &DeviceOrigin,
) -> Result<AuthenticationChallenge, BrowserFetchError> {
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_cache(RequestCache::NoStore);
    init.set_credentials(RequestCredentials::Omit);
    init.set_mode(RequestMode::Cors);
    init.set_redirect(RequestRedirect::Error);
    annotate_local_address_space(window, &init)?;
    let request = Request::new_with_str_and_init(&origin.authentication_url(), &init)
        .map_err(javascript_error)?;
    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(javascript_error)?
        .dyn_into::<web_sys::Response>()
        .map_err(javascript_error)?;
    if response.status() != 200 {
        return Err(BrowserFetchError::HttpStatus(response.status()));
    }
    let content_type = response
        .headers()
        .get("Content-Type")
        .map_err(javascript_error)?
        .ok_or(BrowserFetchError::MissingHeader("Content-Type"))?;
    if content_type != JSON_MEDIA_TYPE {
        return Err(BrowserFetchError::Media(content_type));
    }
    let content_length = response
        .headers()
        .get("Content-Length")
        .map_err(javascript_error)?
        .ok_or(BrowserFetchError::MissingHeader("Content-Length"))?;
    if content_length != AUTH_DISCOVERY_CONTENT_LENGTH {
        return Err(BrowserFetchError::ContentLength(content_length));
    }
    let body = JsFuture::from(response.array_buffer().map_err(javascript_error)?)
        .await
        .map_err(javascript_error)?
        .dyn_into::<ArrayBuffer>()
        .map_err(javascript_error)?;
    decode_authentication_challenge(&Uint8Array::new(&body).to_vec())
        .map_err(BrowserFetchError::Challenge)
}

/// Opens a fresh HMAC session bound to the actual browser document origin.
pub async fn open_authenticated_session(
    window: &Window,
    device: &DeviceOrigin,
    config_digest: Digest,
) -> Result<AuthenticatedHttpSession, BrowserFetchError> {
    let calling_origin = document_origin(window)?;
    let challenge = fetch_authentication_challenge(window, device).await?;
    Ok(AuthenticatedHttpSession::new(
        challenge.nonce(),
        config_digest,
        calling_origin,
    ))
}

/// Performs one browser fetch and accepts only the exact signed response.
///
/// Redirects and browser credentials are disabled. Any transport/metadata/
/// authentication failure abandons the pending session counter; the caller must
/// independently return its operation state machine to reconciliation.
pub async fn fetch_pending_request(
    window: &Window,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    request: &AuthenticatedProtocolRequest,
    secret: &[u8],
) -> Result<Response, BrowserFetchError> {
    let result = fetch_pending_request_inner(window, origin, session, request, secret).await;
    if result.is_err() {
        session.abandon_pending();
    }
    result
}

/// Returns the browser-generated calling origin used by the CORS `Origin` header.
pub fn document_origin(window: &Window) -> Result<CorsOrigin, BrowserFetchError> {
    let origin = window.location().origin().map_err(javascript_error)?;
    CorsOrigin::parse(&origin).map_err(|_| BrowserFetchError::DocumentOrigin)
}

async fn fetch_pending_request_inner(
    window: &Window,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    request: &AuthenticatedProtocolRequest,
    secret: &[u8],
) -> Result<Response, BrowserFetchError> {
    if document_origin(window)? != session.origin() {
        return Err(BrowserFetchError::DocumentOrigin);
    }
    let headers = Headers::new().map_err(javascript_error)?;
    headers
        .set("Content-Type", request.content_type())
        .map_err(javascript_error)?;
    headers
        .set(
            request.counter_header_name(),
            &request.counter_header_value(),
        )
        .map_err(javascript_error)?;
    headers
        .set(
            request.authorization_header_name(),
            request.authorization_header_value(),
        )
        .map_err(javascript_error)?;

    let bytes = Uint8Array::from(request.body());
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_headers_headers(&headers);
    init.set_body_opt_u8_array(Some(&bytes));
    init.set_cache(RequestCache::NoStore);
    init.set_credentials(RequestCredentials::Omit);
    init.set_mode(RequestMode::Cors);
    init.set_redirect(RequestRedirect::Error);
    annotate_local_address_space(window, &init)?;
    let request =
        Request::new_with_str_and_init(&origin.control_url(), &init).map_err(javascript_error)?;
    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(javascript_error)?
        .dyn_into::<web_sys::Response>()
        .map_err(javascript_error)?;

    let status = response.status();
    let response_headers = response.headers();
    let content_type = response_headers
        .get("Content-Type")
        .map_err(javascript_error)?
        .ok_or(BrowserFetchError::MissingHeader("Content-Type"))?;
    let media = match content_type.as_str() {
        NATIVE_FRAME_MEDIA_TYPE => AuthenticatedMedia::NativeFrame,
        JSON_MEDIA_TYPE => AuthenticatedMedia::Json,
        _ => return Err(BrowserFetchError::Media(content_type)),
    };
    let counter = response_headers
        .get(AuthenticatedProtocolResponse::counter_header_name())
        .map_err(javascript_error)?
        .ok_or(BrowserFetchError::MissingHeader(
            AuthenticatedProtocolResponse::counter_header_name(),
        ))?;
    let authorization = response_headers
        .get(AuthenticatedProtocolResponse::authorization_header_name())
        .map_err(javascript_error)?
        .ok_or(BrowserFetchError::MissingHeader(
            AuthenticatedProtocolResponse::authorization_header_name(),
        ))?;
    let body = JsFuture::from(response.array_buffer().map_err(javascript_error)?)
        .await
        .map_err(javascript_error)?
        .dyn_into::<ArrayBuffer>()
        .map_err(javascript_error)?;
    let body = Uint8Array::new(&body).to_vec();
    session
        .accept_response(
            AuthenticatedProtocolResponse {
                http_status: status,
                media,
                counter_header: &counter,
                authorization_header: &authorization,
                body: &body,
            },
            secret,
        )
        .map_err(BrowserFetchError::Session)
}

/// Drives exactly one retry-safe cache operation over authenticated Wi-Fi.
///
/// Browser event loops call this repeatedly until [`CacheUploadPhase::Complete`].
/// A failed fetch returns both protocol and upload machines to a state from
/// which the next call starts with exact `StorageInspect` reconciliation.
pub async fn drive_cache_upload_step(
    window: &Window,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    upload: &mut CacheUploadMachine,
    source: &impl UploadSource,
    secret: &[u8],
) -> Result<CacheUploadPhase, BrowserDeliveryError> {
    let Some(operation) = upload.next_request(source)? else {
        return Ok(CacheUploadPhase::Complete);
    };
    let request = match session.begin_request(operation.operation, &operation.body, secret) {
        Ok(request) => request,
        Err(error) => {
            upload.abandon_pending();
            return Err(BrowserDeliveryError::Session(error));
        }
    };
    let response = match fetch_pending_request(window, origin, session, &request, secret).await {
        Ok(response) => response,
        Err(error) => {
            upload.abandon_pending();
            return Err(BrowserDeliveryError::Fetch(error));
        }
    };
    upload.accept_response(&response)?;
    Ok(upload.phase())
}

fn javascript_error(value: JsValue) -> BrowserFetchError {
    BrowserFetchError::Javascript(format!("{value:?}"))
}

fn annotate_local_address_space(
    window: &Window,
    init: &RequestInit,
) -> Result<(), BrowserFetchError> {
    if !window.is_secure_context() {
        return Ok(());
    }
    Reflect::set(
        init.as_ref(),
        &JsValue::from_str("targetAddressSpace"),
        &JsValue::from_str("local"),
    )
    .map(|_| ())
    .map_err(javascript_error)
}

/// Browser transport, response-metadata, or authenticated-session failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserFetchError {
    /// Device URL was not a path-free absolute HTTP(S) origin.
    DeviceOrigin,
    /// Calling document has an opaque/non-HTTP origin or differs from the HMAC session.
    DocumentOrigin,
    /// Browser API rejected construction, fetch, headers, or response bytes.
    Javascript(String),
    /// Device returned an HTTP failure before a signed control response existed.
    HttpStatus(u16),
    /// Required signed-response metadata was absent.
    MissingHeader(&'static str),
    /// Response content type was not one of the two authenticated representations.
    Media(String),
    /// Public authentication discovery was malformed or incompatible.
    Challenge(AuthenticationChallengeError),
    /// Public discovery body declared a length outside the exact fixed schema.
    ContentLength(String),
    /// Response failed exact HMAC/native-protocol validation.
    Session(HttpSessionError),
}

impl fmt::Display for BrowserFetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceOrigin => formatter.write_str("device URL is not a canonical HTTP origin"),
            Self::DocumentOrigin => {
                formatter.write_str("browser document origin does not match the HMAC session")
            }
            Self::Javascript(error) => write!(formatter, "browser fetch failed: {error}"),
            Self::HttpStatus(status) => write!(formatter, "device returned HTTP status {status}"),
            Self::MissingHeader(header) => {
                write!(formatter, "authenticated response omitted {header}")
            }
            Self::Media(media) => write!(formatter, "unsupported response media {media}"),
            Self::Challenge(error) => write!(formatter, "device challenge rejected: {error}"),
            Self::ContentLength(length) => {
                write!(
                    formatter,
                    "device challenge declared content length {length}"
                )
            }
            Self::Session(error) => write!(formatter, "authenticated session failed: {error}"),
        }
    }
}

impl std::error::Error for BrowserFetchError {}

/// One-step browser delivery failure with transport versus upload semantics preserved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserDeliveryError {
    /// Upload state rejected its local source or prior device progress.
    Upload(CacheUploadError),
    /// Native/HMAC request construction failed before fetch.
    Session(HttpSessionError),
    /// Browser fetch or authenticated response validation failed.
    Fetch(BrowserFetchError),
}

impl From<CacheUploadError> for BrowserDeliveryError {
    fn from(value: CacheUploadError) -> Self {
        Self::Upload(value)
    }
}

impl fmt::Display for BrowserDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Upload(error) => write!(formatter, "cache upload failed: {error}"),
            Self::Session(error) => write!(formatter, "request construction failed: {error}"),
            Self::Fetch(error) => write!(formatter, "device fetch failed: {error}"),
        }
    }
}

impl std::error::Error for BrowserDeliveryError {}
