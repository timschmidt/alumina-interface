//! Browser `fetch` adapter for the authenticated split-phase protocol session.

use std::fmt;

use alumina_net::{AUTH_DISCOVERY_CONTENT_LENGTH, AuthenticatedMedia, CorsOrigin};
use alumina_protocol::Digest;
use js_sys::{ArrayBuffer, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Headers, Performance, Request, RequestCache, RequestCredentials, RequestInit, RequestMode,
    RequestRedirect, Url, Window, WorkerGlobalScope,
};

use crate::Response;
use crate::capability::{
    CapabilityDownloadError, CapabilityDownloadMachine, CapabilityDownloadPhase,
};
use crate::clock::{BrowserTimeError, ClockProbeError, DeviceClockModel, MonotonicTimeBounds};
use crate::graph::{
    GraphInstallError, GraphInstallMachine, GraphInstallPhase, GraphRunError, GraphRunMachine,
    GraphRunPhase,
};
use crate::health::{RuntimeHealthClientError, RuntimeHealthModel, RuntimeHealthUpdate};
use crate::http::{
    AUTHENTICATION_PATH, AuthenticatedHttpSession, AuthenticatedProtocolRequest,
    AuthenticatedProtocolResponse, AuthenticationChallenge, AuthenticationChallengeError,
    CONTROL_PATH, HttpSessionError, NATIVE_FRAME_MEDIA_TYPE, decode_authentication_challenge,
};
use crate::upload::{CacheUploadError, CacheUploadMachine, CacheUploadPhase, UploadSource};

const JSON_MEDIA_TYPE: &str = "application/json";
const COUNTERS_PER_MILLISECOND: u64 = 1_000_000;

trait BrowserScope {
    fn fetch_request(&self, request: &Request) -> Promise;
    fn calling_origin(&self) -> Result<CorsOrigin, BrowserFetchError>;
    fn performance_clock(&self) -> Option<Performance>;
    fn secure_context(&self) -> bool;
}

impl BrowserScope for Window {
    fn fetch_request(&self, request: &Request) -> Promise {
        self.fetch_with_request(request)
    }

    fn calling_origin(&self) -> Result<CorsOrigin, BrowserFetchError> {
        let origin = self.location().origin().map_err(javascript_error)?;
        CorsOrigin::parse(&origin).map_err(|_| BrowserFetchError::DocumentOrigin)
    }

    fn performance_clock(&self) -> Option<Performance> {
        self.performance()
    }

    fn secure_context(&self) -> bool {
        self.is_secure_context()
    }
}

impl BrowserScope for WorkerGlobalScope {
    fn fetch_request(&self, request: &Request) -> Promise {
        self.fetch_with_request(request)
    }

    fn calling_origin(&self) -> Result<CorsOrigin, BrowserFetchError> {
        CorsOrigin::parse(&self.location().origin()).map_err(|_| BrowserFetchError::DocumentOrigin)
    }

    fn performance_clock(&self) -> Option<Performance> {
        self.performance()
    }

    fn secure_context(&self) -> bool {
        self.is_secure_context()
    }
}

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
    fetch_authentication_challenge_inner(window, origin).await
}

/// Worker-scope variant of [`fetch_authentication_challenge`].
pub async fn fetch_authentication_challenge_in_worker(
    worker: &WorkerGlobalScope,
    origin: &DeviceOrigin,
) -> Result<AuthenticationChallenge, BrowserFetchError> {
    fetch_authentication_challenge_inner(worker, origin).await
}

async fn fetch_authentication_challenge_inner(
    scope: &impl BrowserScope,
    origin: &DeviceOrigin,
) -> Result<AuthenticationChallenge, BrowserFetchError> {
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_cache(RequestCache::NoStore);
    init.set_credentials(RequestCredentials::Omit);
    init.set_mode(RequestMode::Cors);
    init.set_redirect(RequestRedirect::Error);
    annotate_local_address_space(scope, &init)?;
    let request = Request::new_with_str_and_init(&origin.authentication_url(), &init)
        .map_err(javascript_error)?;
    let response = JsFuture::from(scope.fetch_request(&request))
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
    open_authenticated_session_inner(window, device, config_digest).await
}

/// Opens a fresh HMAC session from a worker, bound to its inherited origin.
pub async fn open_authenticated_session_in_worker(
    worker: &WorkerGlobalScope,
    device: &DeviceOrigin,
    config_digest: Digest,
) -> Result<AuthenticatedHttpSession, BrowserFetchError> {
    open_authenticated_session_inner(worker, device, config_digest).await
}

async fn open_authenticated_session_inner(
    scope: &impl BrowserScope,
    device: &DeviceOrigin,
    config_digest: Digest,
) -> Result<AuthenticatedHttpSession, BrowserFetchError> {
    let calling_origin = scope.calling_origin()?;
    let challenge = fetch_authentication_challenge_inner(scope, device).await?;
    let initial_counter = browser_counter_seed()?;
    AuthenticatedHttpSession::starting_at(
        challenge.nonce(),
        config_digest,
        calling_origin,
        initial_counter,
    )
    .map_err(BrowserFetchError::Session)
}

fn browser_counter_seed() -> Result<u64, BrowserFetchError> {
    let epoch_ms = js_sys::Date::now();
    if !epoch_ms.is_finite() || epoch_ms.is_sign_negative() {
        return Err(BrowserFetchError::CounterSeed);
    }
    let epoch_ms = epoch_ms.floor() as u64;
    let entropy = (js_sys::Math::random() * COUNTERS_PER_MILLISECOND as f64).floor() as u64;
    epoch_ms
        .checked_mul(COUNTERS_PER_MILLISECOND)
        .and_then(|prefix| prefix.checked_add(entropy))
        .filter(|counter| *counter != 0 && *counter != u64::MAX)
        .ok_or(BrowserFetchError::CounterSeed)
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

/// Worker-scope variant of [`fetch_pending_request`].
pub async fn fetch_pending_request_in_worker(
    worker: &WorkerGlobalScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    request: &AuthenticatedProtocolRequest,
    secret: &[u8],
) -> Result<Response, BrowserFetchError> {
    let result = fetch_pending_request_inner(worker, origin, session, request, secret).await;
    if result.is_err() {
        session.abandon_pending();
    }
    result
}

/// Acquires one authenticated causal clock sample over Wi-Fi.
///
/// The first timer read is widened downward before the probe identity is
/// committed. The second is taken only after the signed response is fully read
/// and validated, then widened upward. Request construction, browser dispatch,
/// response buffering, and authentication therefore add uncertainty but can
/// never make the causal interval spuriously narrow.
///
/// # Errors
///
/// Rejects browser timing, exact clock state, HMAC request construction, fetch,
/// response authentication, or heartbeat semantics. Every failed transport
/// spends both the HTTP counter and clock probe identity.
pub async fn drive_clock_probe(
    window: &Window,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    model: &mut DeviceClockModel,
    maximum_timer_error_ns: u64,
    secret: &[u8],
) -> Result<alumina_clock::ClockObservation, BrowserClockError> {
    drive_clock_probe_inner(
        window,
        origin,
        session,
        model,
        maximum_timer_error_ns,
        secret,
    )
    .await
}

/// Worker-scope causal heartbeat acquisition isolated from rendering stalls.
pub async fn drive_clock_probe_in_worker(
    worker: &WorkerGlobalScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    model: &mut DeviceClockModel,
    maximum_timer_error_ns: u64,
    secret: &[u8],
) -> Result<alumina_clock::ClockObservation, BrowserClockError> {
    drive_clock_probe_inner(
        worker,
        origin,
        session,
        model,
        maximum_timer_error_ns,
        secret,
    )
    .await
}

async fn drive_clock_probe_inner(
    scope: &impl BrowserScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    model: &mut DeviceClockModel,
    maximum_timer_error_ns: u64,
    secret: &[u8],
) -> Result<alumina_clock::ClockObservation, BrowserClockError> {
    let performance = scope
        .performance_clock()
        .ok_or(BrowserClockError::PerformanceUnavailable)?;
    let send = MonotonicTimeBounds::from_milliseconds(performance.now(), maximum_timer_error_ns)?;
    let probe = model.begin_probe(send.earliest_ns())?;
    let request = match session.begin_request(probe.operation(), probe.body(), secret) {
        Ok(request) => request,
        Err(error) => {
            model.abandon_pending();
            return Err(BrowserClockError::Session(error));
        }
    };
    let response = match fetch_pending_request_inner(scope, origin, session, &request, secret).await
    {
        Ok(response) => response,
        Err(error) => {
            session.abandon_pending();
            model.abandon_pending();
            return Err(BrowserClockError::Fetch(error));
        }
    };
    let receive =
        match MonotonicTimeBounds::from_milliseconds(performance.now(), maximum_timer_error_ns) {
            Ok(receive) => receive,
            Err(error) => {
                model.abandon_pending();
                return Err(BrowserClockError::Time(error));
            }
        };
    model
        .accept_response(&response, receive.latest_ns())
        .map_err(BrowserClockError::Clock)
}

/// Acquires one authenticated passive runtime-health snapshot.
///
/// The session must be bound to the zero configuration identity declared by
/// [`crate::health::RuntimeHealthRequest`]. Transport and semantic failures
/// retain the model's last valid evidence; an explicit device `Unsupported`
/// response is an accepted update that clears boot-scoped measurements.
pub async fn drive_runtime_health(
    window: &Window,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    model: &mut RuntimeHealthModel,
    secret: &[u8],
) -> Result<RuntimeHealthUpdate, BrowserHealthError> {
    drive_runtime_health_inner(window, origin, session, model, secret).await
}

/// Worker-scope runtime-health acquisition isolated from rendering stalls.
pub async fn drive_runtime_health_in_worker(
    worker: &WorkerGlobalScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    model: &mut RuntimeHealthModel,
    secret: &[u8],
) -> Result<RuntimeHealthUpdate, BrowserHealthError> {
    drive_runtime_health_inner(worker, origin, session, model, secret).await
}

async fn drive_runtime_health_inner(
    scope: &impl BrowserScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    model: &mut RuntimeHealthModel,
    secret: &[u8],
) -> Result<RuntimeHealthUpdate, BrowserHealthError> {
    let operation = model.request();
    if session.config_digest() != operation.config_digest() {
        return Err(BrowserHealthError::ConfigurationIdentity);
    }
    let request = session
        .begin_request(operation.operation(), operation.body(), secret)
        .map_err(BrowserHealthError::Session)?;
    let response = match fetch_pending_request_inner(scope, origin, session, &request, secret).await
    {
        Ok(response) => response,
        Err(error) => {
            session.abandon_pending();
            return Err(BrowserHealthError::Fetch(error));
        }
    };
    model
        .accept_response(&response)
        .map_err(BrowserHealthError::Health)
}

/// Acquires one authenticated, side-effect-free canonical capability range.
///
/// Repeated calls assemble a contiguous complete document. A transport failure
/// abandons only the pending range so the next call emits the identical offset,
/// digest, and byte bound. The session must remain on the zero configuration
/// identity required by the capability service.
pub async fn drive_capability_step(
    window: &Window,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    download: &mut CapabilityDownloadMachine,
    secret: &[u8],
) -> Result<CapabilityDownloadPhase, BrowserCapabilityError> {
    drive_capability_step_inner(window, origin, session, download, secret).await
}

/// Worker-scope variant of [`drive_capability_step`].
pub async fn drive_capability_step_in_worker(
    worker: &WorkerGlobalScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    download: &mut CapabilityDownloadMachine,
    secret: &[u8],
) -> Result<CapabilityDownloadPhase, BrowserCapabilityError> {
    drive_capability_step_inner(worker, origin, session, download, secret).await
}

async fn drive_capability_step_inner(
    scope: &impl BrowserScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    download: &mut CapabilityDownloadMachine,
    secret: &[u8],
) -> Result<CapabilityDownloadPhase, BrowserCapabilityError> {
    if !session.config_digest().is_zero() {
        return Err(BrowserCapabilityError::ConfigurationIdentity);
    }
    let Some(operation) = download.next_request()? else {
        return Ok(CapabilityDownloadPhase::Complete);
    };
    let request = match session.begin_request(operation.operation, &operation.body, secret) {
        Ok(request) => request,
        Err(error) => {
            download.abandon_pending();
            return Err(BrowserCapabilityError::Session(error));
        }
    };
    let response = match fetch_pending_request_inner(scope, origin, session, &request, secret).await
    {
        Ok(response) => response,
        Err(error) => {
            session.abandon_pending();
            download.abandon_pending();
            return Err(BrowserCapabilityError::Fetch(error));
        }
    };
    download.accept_response(&response)?;
    Ok(download.phase())
}

/// Returns the browser-generated calling origin used by the CORS `Origin` header.
pub fn document_origin(window: &Window) -> Result<CorsOrigin, BrowserFetchError> {
    window.calling_origin()
}

/// Returns the worker-generated calling origin inherited by CORS fetches.
pub fn worker_origin(worker: &WorkerGlobalScope) -> Result<CorsOrigin, BrowserFetchError> {
    worker.calling_origin()
}

async fn fetch_pending_request_inner(
    scope: &impl BrowserScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    request: &AuthenticatedProtocolRequest,
    secret: &[u8],
) -> Result<Response, BrowserFetchError> {
    if scope.calling_origin()? != session.origin() {
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
    annotate_local_address_space(scope, &init)?;
    let request =
        Request::new_with_str_and_init(&origin.control_url(), &init).map_err(javascript_error)?;
    let response = JsFuture::from(scope.fetch_request(&request))
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
    drive_cache_upload_step_inner(window, origin, session, upload, source, secret).await
}

/// Worker-scope variant of [`drive_cache_upload_step`].
pub async fn drive_cache_upload_step_in_worker(
    worker: &WorkerGlobalScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    upload: &mut CacheUploadMachine,
    source: &impl UploadSource,
    secret: &[u8],
) -> Result<CacheUploadPhase, BrowserDeliveryError> {
    drive_cache_upload_step_inner(worker, origin, session, upload, source, secret).await
}

async fn drive_cache_upload_step_inner(
    scope: &impl BrowserScope,
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
    let response = match fetch_pending_request_inner(scope, origin, session, &request, secret).await
    {
        Ok(response) => response,
        Err(error) => {
            session.abandon_pending();
            upload.abandon_pending();
            return Err(BrowserDeliveryError::Fetch(error));
        }
    };
    upload.accept_response(&response)?;
    Ok(upload.phase())
}

/// Drives exactly one retry-safe deployed-graph lifecycle operation over Wi-Fi.
///
/// Callers first publish [`crate::graph::GraphPackageUpload`] with
/// [`drive_cache_upload_step`], then repeatedly call this function until the
/// returned phase is [`GraphInstallPhase::Complete`]. Ambiguous I/O preserves
/// the phase and causes the exact install or read-only status poll to be retried.
pub async fn drive_graph_install_step(
    window: &Window,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    install: &mut GraphInstallMachine,
    secret: &[u8],
) -> Result<GraphInstallPhase, BrowserGraphInstallError> {
    drive_graph_install_step_inner(window, origin, session, install, secret).await
}

/// Worker-scope variant of [`drive_graph_install_step`].
pub async fn drive_graph_install_step_in_worker(
    worker: &WorkerGlobalScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    install: &mut GraphInstallMachine,
    secret: &[u8],
) -> Result<GraphInstallPhase, BrowserGraphInstallError> {
    drive_graph_install_step_inner(worker, origin, session, install, secret).await
}

async fn drive_graph_install_step_inner(
    scope: &impl BrowserScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    install: &mut GraphInstallMachine,
    secret: &[u8],
) -> Result<GraphInstallPhase, BrowserGraphInstallError> {
    let Some(operation) = install.next_request()? else {
        return Ok(GraphInstallPhase::Complete);
    };
    let request = match session.begin_request(operation.operation, &operation.body, secret) {
        Ok(request) => request,
        Err(error) => {
            install.abandon_pending();
            return Err(BrowserGraphInstallError::Session(error));
        }
    };
    let response = match fetch_pending_request_inner(scope, origin, session, &request, secret).await
    {
        Ok(response) => response,
        Err(error) => {
            session.abandon_pending();
            install.abandon_pending();
            return Err(BrowserGraphInstallError::Fetch(error));
        }
    };
    install.accept_response(&response)?;
    Ok(install.phase())
}

/// Drives exactly one retry-safe graph start, status poll, or stop over Wi-Fi.
///
/// A returned [`GraphRunPhase::Running`] means both pinned-core actors and the
/// bridge report the exact run. Call [`GraphRunMachine::request_stop`] before
/// calling this again to begin normal stop; execution faults select stop
/// automatically and remain available through [`GraphRunMachine::fault_report`].
pub async fn drive_graph_run_step(
    window: &Window,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    run: &mut GraphRunMachine,
    secret: &[u8],
) -> Result<GraphRunPhase, BrowserGraphRunError> {
    drive_graph_run_step_inner(window, origin, session, run, secret).await
}

/// Worker-scope variant of [`drive_graph_run_step`].
pub async fn drive_graph_run_step_in_worker(
    worker: &WorkerGlobalScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    run: &mut GraphRunMachine,
    secret: &[u8],
) -> Result<GraphRunPhase, BrowserGraphRunError> {
    drive_graph_run_step_inner(worker, origin, session, run, secret).await
}

async fn drive_graph_run_step_inner(
    scope: &impl BrowserScope,
    origin: &DeviceOrigin,
    session: &mut AuthenticatedHttpSession,
    run: &mut GraphRunMachine,
    secret: &[u8],
) -> Result<GraphRunPhase, BrowserGraphRunError> {
    let Some(operation) = run.next_request()? else {
        return Ok(run.phase());
    };
    let request = match session.begin_request(operation.operation, &operation.body, secret) {
        Ok(request) => request,
        Err(error) => {
            run.abandon_pending();
            return Err(BrowserGraphRunError::Session(error));
        }
    };
    let response = match fetch_pending_request_inner(scope, origin, session, &request, secret).await
    {
        Ok(response) => response,
        Err(error) => {
            session.abandon_pending();
            run.abandon_pending();
            return Err(BrowserGraphRunError::Fetch(error));
        }
    };
    run.accept_response(&response)?;
    Ok(run.phase())
}

fn javascript_error(value: JsValue) -> BrowserFetchError {
    BrowserFetchError::Javascript(format!("{value:?}"))
}

fn annotate_local_address_space(
    scope: &impl BrowserScope,
    init: &RequestInit,
) -> Result<(), BrowserFetchError> {
    if !scope.secure_context() {
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
    /// Browser wall-clock facts could not seed a reload-safe request counter.
    CounterSeed,
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
            Self::CounterSeed => formatter.write_str("browser request counter seed is invalid"),
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

/// One authenticated browser heartbeat acquisition failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserClockError {
    /// This browsing context did not expose a monotonic `Performance` clock.
    PerformanceUnavailable,
    /// The configured timer policy or observed timestamp was not conservative.
    Time(BrowserTimeError),
    /// Exact probe construction or heartbeat estimation failed.
    Clock(ClockProbeError),
    /// Native/HMAC request construction failed before fetch.
    Session(HttpSessionError),
    /// Browser fetch or authenticated response validation failed.
    Fetch(BrowserFetchError),
}

impl From<BrowserTimeError> for BrowserClockError {
    fn from(value: BrowserTimeError) -> Self {
        Self::Time(value)
    }
}

impl From<ClockProbeError> for BrowserClockError {
    fn from(value: ClockProbeError) -> Self {
        Self::Clock(value)
    }
}

impl fmt::Display for BrowserClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PerformanceUnavailable => {
                formatter.write_str("browser monotonic clock is unavailable")
            }
            Self::Time(error) => write!(formatter, "browser clock sampling failed: {error}"),
            Self::Clock(error) => write!(formatter, "device clock probe failed: {error}"),
            Self::Session(error) => write!(formatter, "clock request construction failed: {error}"),
            Self::Fetch(error) => write!(formatter, "clock fetch failed: {error}"),
        }
    }
}

impl std::error::Error for BrowserClockError {}

/// One authenticated browser runtime-health acquisition failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserHealthError {
    /// The session would substitute an active configuration into the zero-config request.
    ConfigurationIdentity,
    /// Native/HMAC request construction failed before fetch.
    Session(HttpSessionError),
    /// Browser fetch or authenticated response validation failed.
    Fetch(BrowserFetchError),
    /// Authenticated health bytes or monotonic device evidence were invalid.
    Health(RuntimeHealthClientError),
}

impl fmt::Display for BrowserHealthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigurationIdentity => {
                formatter.write_str("runtime health requires a zero-configuration session")
            }
            Self::Session(error) => {
                write!(formatter, "health request construction failed: {error}")
            }
            Self::Fetch(error) => write!(formatter, "health fetch failed: {error}"),
            Self::Health(error) => write!(formatter, "runtime health rejected: {error}"),
        }
    }
}

impl std::error::Error for BrowserHealthError {}

/// One authenticated browser capability-range acquisition failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserCapabilityError {
    /// The session would substitute an active configuration into this immutable read.
    ConfigurationIdentity,
    /// Native/HMAC request construction failed before fetch.
    Session(HttpSessionError),
    /// Browser fetch or authenticated response validation failed.
    Fetch(BrowserFetchError),
    /// Range, identity, allocation, or complete-document validation failed.
    Capability(CapabilityDownloadError),
}

impl From<CapabilityDownloadError> for BrowserCapabilityError {
    fn from(value: CapabilityDownloadError) -> Self {
        Self::Capability(value)
    }
}

impl fmt::Display for BrowserCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigurationIdentity => {
                formatter.write_str("capability reads require a zero-configuration session")
            }
            Self::Session(error) => {
                write!(formatter, "capability request construction failed: {error}")
            }
            Self::Fetch(error) => write!(formatter, "capability fetch failed: {error}"),
            Self::Capability(error) => write!(formatter, "capability response rejected: {error}"),
        }
    }
}

impl std::error::Error for BrowserCapabilityError {}

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

/// One-step browser graph-install failure with lifecycle authority preserved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserGraphInstallError {
    /// Graph install state rejected the local request or device report.
    Install(GraphInstallError),
    /// Native/HMAC request construction failed before fetch.
    Session(HttpSessionError),
    /// Browser fetch or authenticated response validation failed.
    Fetch(BrowserFetchError),
}

impl From<GraphInstallError> for BrowserGraphInstallError {
    fn from(value: GraphInstallError) -> Self {
        Self::Install(value)
    }
}

impl fmt::Display for BrowserGraphInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Install(error) => write!(formatter, "graph install failed: {error}"),
            Self::Session(error) => write!(formatter, "request construction failed: {error}"),
            Self::Fetch(error) => write!(formatter, "device fetch failed: {error}"),
        }
    }
}

impl std::error::Error for BrowserGraphInstallError {}

/// One-step browser graph-run failure with exact epoch authority preserved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserGraphRunError {
    /// Graph execution state rejected the local request or device report.
    Run(GraphRunError),
    /// Native/HMAC request construction failed before fetch.
    Session(HttpSessionError),
    /// Browser fetch or authenticated response validation failed.
    Fetch(BrowserFetchError),
}

impl From<GraphRunError> for BrowserGraphRunError {
    fn from(value: GraphRunError) -> Self {
        Self::Run(value)
    }
}

impl fmt::Display for BrowserGraphRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(error) => write!(formatter, "graph run failed: {error}"),
            Self::Session(error) => write!(formatter, "request construction failed: {error}"),
            Self::Fetch(error) => write!(formatter, "device fetch failed: {error}"),
        }
    }
}

impl std::error::Error for BrowserGraphRunError {}
