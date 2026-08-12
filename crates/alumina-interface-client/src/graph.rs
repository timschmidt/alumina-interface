//! Typed graph-package upload and authenticated install lifecycle.

use core::fmt;

use alumina_graph_ir::{
    GRAPH_IR_PACKAGE_BYTES, GraphDeploymentWireError, GraphIrPackage, GraphPublication,
    GraphRunRequest, GraphSelection,
};
use alumina_protocol::{DeviceCycle, Operation, StatusCode};
use alumina_runtime::graph::{
    GraphActorPhase, GraphBridgePhase, GraphCoordinatorFault, GraphCoordinatorPhase,
    GraphCoordinatorReport, GraphCoordinatorReportError, RealtimeGraphState,
};
use alumina_storage::{
    CacheLimits, ChunkUploadHeader, ManifestHasher, ObjectKind, StoredObject, UploadId, UploadPlan,
    sha256,
};

use crate::upload::UploadSource;
use crate::{ClientError, ProtocolClient, Response, Transport};

/// One immutable fixed graph package plus its resumable storage declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphPackageUpload {
    package: GraphIrPackage,
    plan: UploadPlan,
}

impl GraphPackageUpload {
    /// Bind a package to one nonzero resumable upload and storage chunk size.
    pub fn new(
        package: GraphIrPackage,
        upload_id: UploadId,
        chunk_bytes: u32,
        limits: CacheLimits,
    ) -> Result<Self, GraphPackageUploadError> {
        let chunk_bytes_usize =
            usize::try_from(chunk_bytes).map_err(|_| GraphPackageUploadError::Arithmetic)?;
        if chunk_bytes_usize == 0 {
            return Err(GraphPackageUploadError::ChunkSize);
        }
        let chunk_count_usize = GRAPH_IR_PACKAGE_BYTES.div_ceil(chunk_bytes_usize);
        let chunk_count =
            u32::try_from(chunk_count_usize).map_err(|_| GraphPackageUploadError::Arithmetic)?;
        let object = StoredObject {
            kind: ObjectKind::DeployedGraph,
            content: sha256(package.bytes()),
            byte_len: GRAPH_IR_PACKAGE_BYTES as u64,
        };
        let mut manifest = ManifestHasher::new(object, chunk_bytes, chunk_count, limits)
            .map_err(GraphPackageUploadError::Storage)?;
        for (index, chunk) in package.bytes().chunks(chunk_bytes_usize).enumerate() {
            manifest
                .push(
                    u32::try_from(index).map_err(|_| GraphPackageUploadError::Arithmetic)?,
                    sha256(chunk),
                    u32::try_from(chunk.len()).map_err(|_| GraphPackageUploadError::Arithmetic)?,
                )
                .map_err(GraphPackageUploadError::Storage)?;
        }
        let plan = UploadPlan {
            upload_id,
            object,
            manifest: manifest
                .finalize()
                .map_err(GraphPackageUploadError::Storage)?,
            chunk_bytes,
            chunk_count,
        };
        plan.validate(limits)
            .map_err(GraphPackageUploadError::Storage)?;
        Ok(Self { package, plan })
    }

    /// Borrow the independently decoded fixed package.
    pub const fn package(&self) -> &GraphIrPackage {
        &self.package
    }

    /// Construct the exact authenticated GraphInstall body authority.
    pub fn publication(
        &self,
        transaction_id: u64,
    ) -> Result<GraphPublication, GraphDeploymentWireError> {
        let publication = GraphPublication {
            transaction_id,
            publication: alumina_storage::PublishedObject {
                object: self.plan.object,
                manifest: self.plan.manifest,
            },
            package_digest: self.package.digest(),
            implementation_digest: self.package.header().implementation_digest,
        };
        publication.validate()?;
        Ok(publication)
    }
}

impl UploadSource for GraphPackageUpload {
    fn upload_plan(&self) -> UploadPlan {
        self.plan
    }

    fn chunk_header(&self, index: u32) -> Option<ChunkUploadHeader> {
        let bytes = self.chunk_bytes(index)?;
        Some(ChunkUploadHeader {
            upload_id: self.plan.upload_id,
            index,
            byte_len: u32::try_from(bytes.len()).ok()?,
            content: sha256(bytes),
        })
    }

    fn chunk_bytes(&self, index: u32) -> Option<&[u8]> {
        if index >= self.plan.chunk_count {
            return None;
        }
        let start = usize::try_from(index)
            .ok()?
            .checked_mul(usize::try_from(self.plan.chunk_bytes).ok()?)?;
        let end = start
            .checked_add(usize::try_from(self.plan.chunk_bytes).ok()?)?
            .min(GRAPH_IR_PACKAGE_BYTES);
        self.package.bytes().get(start..end)
    }
}

/// Construction failure before any package byte may be uploaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphPackageUploadError {
    /// Storage chunk size was zero.
    ChunkSize,
    /// A host/package count could not be represented exactly.
    Arithmetic,
    /// Shared storage policy rejected the object or manifest.
    Storage(alumina_storage::Error),
}

impl fmt::Display for GraphPackageUploadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for GraphPackageUploadError {}

/// One graph-family operation body ready for native authenticated framing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphOperation {
    /// Exact graph lifecycle operation.
    pub operation: Operation,
    /// Canonical operation-specific body.
    pub body: Vec<u8>,
}

/// Typed application status paired with the canonical combined lifecycle report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphResponse {
    /// Firmware application status.
    pub status: StatusCode,
    /// Service/core-1 lifecycle observation.
    pub report: GraphCoordinatorReport,
}

impl<T: Transport> ProtocolClient<T> {
    /// Fetch current candidate and active graph state.
    pub fn graph_status(&mut self) -> Result<GraphResponse, GraphClientError<T::Error>> {
        let response = self
            .request(Operation::GraphGet, &[])
            .map_err(GraphClientError::Client)?;
        decode_graph_response(response).map_err(GraphClientError::Report)
    }

    /// Begin or reconcile independent installation of one published package.
    pub fn graph_install(
        &mut self,
        publication: GraphPublication,
    ) -> Result<GraphResponse, GraphClientError<T::Error>> {
        let body = publication.encode().map_err(GraphClientError::Wire)?;
        let response = self
            .request(Operation::GraphInstall, &body)
            .map_err(GraphClientError::Client)?;
        let response = decode_graph_response(response).map_err(GraphClientError::Report)?;
        validate_success_identity(response, selection(publication))?;
        Ok(response)
    }

    /// Select one candidate and request dual-core authorization.
    pub fn graph_activate(
        &mut self,
        selection: GraphSelection,
    ) -> Result<GraphResponse, GraphClientError<T::Error>> {
        let body = selection.encode().map_err(GraphClientError::Wire)?;
        let response = self
            .request(Operation::GraphActivate, &body)
            .map_err(GraphClientError::Client)?;
        let response = decode_graph_response(response).map_err(GraphClientError::Report)?;
        validate_success_identity(response, selection)?;
        Ok(response)
    }

    /// Discard a candidate or clear the exact active graph package.
    pub fn graph_clear(
        &mut self,
        selection: GraphSelection,
    ) -> Result<GraphResponse, GraphClientError<T::Error>> {
        let body = selection.encode().map_err(GraphClientError::Wire)?;
        let response = self
            .request(Operation::GraphClear, &body)
            .map_err(GraphClientError::Client)?;
        let response = decode_graph_response(response).map_err(GraphClientError::Report)?;
        validate_success_identity(response, selection)?;
        Ok(response)
    }

    /// Admit one exact future graph epoch; acceptance is distinct from both cores running.
    pub fn graph_start(
        &mut self,
        request: GraphRunRequest,
    ) -> Result<GraphResponse, GraphClientError<T::Error>> {
        let body = request.encode().map_err(GraphClientError::Wire)?;
        let response = self
            .request(Operation::GraphStart, &body)
            .map_err(GraphClientError::Client)?;
        let response = decode_graph_response(response).map_err(GraphClientError::Report)?;
        validate_success_run(response, request)?;
        Ok(response)
    }

    /// Stop or reconcile the exact previously admitted graph epoch.
    pub fn graph_stop(
        &mut self,
        request: GraphRunRequest,
    ) -> Result<GraphResponse, GraphClientError<T::Error>> {
        let body = request.encode().map_err(GraphClientError::Wire)?;
        let response = self
            .request(Operation::GraphStop, &body)
            .map_err(GraphClientError::Client)?;
        let response = decode_graph_response(response).map_err(GraphClientError::Report)?;
        validate_success_run(response, request)?;
        Ok(response)
    }
}

/// UI-facing phase for retry-safe installation after storage publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphInstallPhase {
    /// GraphInstall has not yet been acknowledged.
    Installing,
    /// Polling while both cores replay and admit bytes.
    AwaitingCandidate,
    /// GraphActivate is the unique next request.
    Activating,
    /// Polling until core-1 selection and service authorization agree.
    AwaitingActive,
    /// The exact selected package is active and authorized.
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GraphInstallAction {
    Install,
    PollCandidate,
    Activate,
    PollActive,
}

/// Transport-independent install/poll/activate coordinator.
#[derive(Debug)]
pub struct GraphInstallMachine {
    publication: GraphPublication,
    selection: GraphSelection,
    phase: GraphInstallPhase,
    pending: Option<GraphInstallAction>,
    latest: Option<GraphCoordinatorReport>,
}

impl GraphInstallMachine {
    /// Bind one exact already-published package.
    pub fn new(publication: GraphPublication) -> Result<Self, GraphDeploymentWireError> {
        publication.validate()?;
        Ok(Self {
            publication,
            selection: selection(publication),
            phase: GraphInstallPhase::Installing,
            pending: None,
            latest: None,
        })
    }

    /// Current lifecycle phase.
    pub const fn phase(&self) -> GraphInstallPhase {
        self.phase
    }

    /// Latest accepted canonical status.
    pub const fn latest_report(&self) -> Option<GraphCoordinatorReport> {
        self.latest
    }

    /// Emit the unique next install, poll, or activation operation.
    pub fn next_request(&mut self) -> Result<Option<GraphOperation>, GraphInstallError> {
        if self.pending.is_some() {
            return Err(GraphInstallError::RequestPending);
        }
        let (action, operation, body) = match self.phase {
            GraphInstallPhase::Installing => (
                GraphInstallAction::Install,
                Operation::GraphInstall,
                self.publication
                    .encode()
                    .map_err(GraphInstallError::Wire)?
                    .to_vec(),
            ),
            GraphInstallPhase::AwaitingCandidate => (
                GraphInstallAction::PollCandidate,
                Operation::GraphGet,
                Vec::new(),
            ),
            GraphInstallPhase::Activating => (
                GraphInstallAction::Activate,
                Operation::GraphActivate,
                self.selection
                    .encode()
                    .map_err(GraphInstallError::Wire)?
                    .to_vec(),
            ),
            GraphInstallPhase::AwaitingActive => (
                GraphInstallAction::PollActive,
                Operation::GraphGet,
                Vec::new(),
            ),
            GraphInstallPhase::Complete => return Ok(None),
        };
        self.pending = Some(action);
        Ok(Some(GraphOperation { operation, body }))
    }

    /// Accept one already authenticated/correlated native response.
    pub fn accept_response(&mut self, response: &Response) -> Result<(), GraphInstallError> {
        let action = self
            .pending
            .take()
            .ok_or(GraphInstallError::NoPendingRequest)?;
        if response.status != StatusCode::Ok {
            return Err(GraphInstallError::DeviceStatus(response.status));
        }
        let graph = decode_graph_response(response.clone()).map_err(GraphInstallError::Report)?;
        validate_report_identity(graph.report, self.selection)?;
        if graph.report.phase == GraphCoordinatorPhase::Rejected
            || graph.report.fault != GraphCoordinatorFault::None
            || graph.report.realtime.state == RealtimeGraphState::Rejected
                && realtime_matches(graph.report, self.selection)
        {
            return Err(GraphInstallError::Rejected);
        }
        self.latest = Some(graph.report);
        self.phase = match action {
            GraphInstallAction::Install | GraphInstallAction::PollCandidate => {
                match graph.report.phase {
                    GraphCoordinatorPhase::CandidateValid => GraphInstallPhase::Activating,
                    GraphCoordinatorPhase::Active
                        if active_matches(graph.report, self.selection) =>
                    {
                        GraphInstallPhase::Complete
                    }
                    GraphCoordinatorPhase::Validating => GraphInstallPhase::AwaitingCandidate,
                    GraphCoordinatorPhase::Recovering
                    | GraphCoordinatorPhase::Preparing
                    | GraphCoordinatorPhase::Activating
                    | GraphCoordinatorPhase::Committing
                    | GraphCoordinatorPhase::Authorizing => GraphInstallPhase::AwaitingActive,
                    _ => return Err(GraphInstallError::UnexpectedPhase),
                }
            }
            GraphInstallAction::Activate | GraphInstallAction::PollActive => {
                match graph.report.phase {
                    GraphCoordinatorPhase::Active
                        if active_matches(graph.report, self.selection) =>
                    {
                        GraphInstallPhase::Complete
                    }
                    GraphCoordinatorPhase::Preparing
                    | GraphCoordinatorPhase::Activating
                    | GraphCoordinatorPhase::Committing
                    | GraphCoordinatorPhase::Authorizing => GraphInstallPhase::AwaitingActive,
                    _ => return Err(GraphInstallError::UnexpectedPhase),
                }
            }
        };
        Ok(())
    }

    /// Abandon an ambiguous I/O result without advancing the lifecycle.
    pub fn abandon_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }
}

/// UI-facing phase for one retry-safe, boot-local graph execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphRunPhase {
    /// The exact `GraphStart` request has not yet been acknowledged.
    Starting,
    /// Start was accepted; the client is polling for both pinned-core actors.
    AwaitingRunning,
    /// Both actors and the shared bridge report this exact run as running.
    Running,
    /// `GraphStop` is the unique next request, including after a retained fault.
    Stopping,
    /// Stop was accepted; the client is polling for both acknowledgements.
    AwaitingStopped,
    /// Both actors are installed and the bridge is empty for this completed run.
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GraphRunAction {
    Start,
    PollRunning,
    Stop,
    PollStopped,
}

/// Transport-independent exact start/poll/stop coordinator.
///
/// A canonical execution fault is retained and automatically changes the next
/// operation to `GraphStop`; callers can inspect [`Self::fault_report`] after
/// the stop handshake clears the firmware's shared latch.
#[derive(Debug)]
pub struct GraphRunMachine {
    request: GraphRunRequest,
    phase: GraphRunPhase,
    pending: Option<GraphRunAction>,
    latest: Option<GraphCoordinatorReport>,
    fault: Option<GraphCoordinatorReport>,
}

impl GraphRunMachine {
    /// Bind one exact active package, nonzero boot-local run, and future device epoch.
    pub fn new(request: GraphRunRequest) -> Result<Self, GraphDeploymentWireError> {
        request.validate()?;
        Ok(Self {
            request,
            phase: GraphRunPhase::Starting,
            pending: None,
            latest: None,
            fault: None,
        })
    }

    /// Build an exact run request from the package authority used for installation.
    pub fn from_publication(
        publication: GraphPublication,
        run_id: u64,
        start_cycle: DeviceCycle,
    ) -> Result<Self, GraphDeploymentWireError> {
        publication.validate()?;
        Self::new(GraphRunRequest {
            transaction_id: publication.transaction_id,
            run_id,
            start_cycle,
            content_digest: publication.content_digest(),
            package_digest: publication.package_digest,
            implementation_digest: publication.implementation_digest,
        })
    }

    /// Complete exact request retained across transport retries.
    pub const fn request(&self) -> GraphRunRequest {
        self.request
    }

    /// Current execution lifecycle phase.
    pub const fn phase(&self) -> GraphRunPhase {
        self.phase
    }

    /// Latest accepted canonical status.
    pub const fn latest_report(&self) -> Option<GraphCoordinatorReport> {
        self.latest
    }

    /// First accepted execution-fault report, retained after stop reconciliation.
    pub const fn fault_report(&self) -> Option<GraphCoordinatorReport> {
        self.fault
    }

    /// Request an exact stop after `GraphStart` has been acknowledged.
    pub fn request_stop(&mut self) -> Result<(), GraphRunError> {
        if self.pending.is_some() {
            return Err(GraphRunError::RequestPending);
        }
        match self.phase {
            GraphRunPhase::Starting => Err(GraphRunError::StartNotAcknowledged),
            GraphRunPhase::AwaitingRunning | GraphRunPhase::Running => {
                self.phase = GraphRunPhase::Stopping;
                Ok(())
            }
            GraphRunPhase::Stopping | GraphRunPhase::AwaitingStopped | GraphRunPhase::Complete => {
                Ok(())
            }
        }
    }

    /// Emit the unique next start, status poll, or stop operation.
    pub fn next_request(&mut self) -> Result<Option<GraphOperation>, GraphRunError> {
        if self.pending.is_some() {
            return Err(GraphRunError::RequestPending);
        }
        let (action, operation, body) = match self.phase {
            GraphRunPhase::Starting => (
                GraphRunAction::Start,
                Operation::GraphStart,
                self.request.encode().map_err(GraphRunError::Wire)?.to_vec(),
            ),
            GraphRunPhase::AwaitingRunning => {
                (GraphRunAction::PollRunning, Operation::GraphGet, Vec::new())
            }
            GraphRunPhase::Running | GraphRunPhase::Complete => return Ok(None),
            GraphRunPhase::Stopping => (
                GraphRunAction::Stop,
                Operation::GraphStop,
                self.request.encode().map_err(GraphRunError::Wire)?.to_vec(),
            ),
            GraphRunPhase::AwaitingStopped => {
                (GraphRunAction::PollStopped, Operation::GraphGet, Vec::new())
            }
        };
        self.pending = Some(action);
        Ok(Some(GraphOperation { operation, body }))
    }

    /// Accept one already authenticated and correlated native response.
    pub fn accept_response(&mut self, response: &Response) -> Result<(), GraphRunError> {
        let action = self.pending.take().ok_or(GraphRunError::NoPendingRequest)?;
        if response.status != StatusCode::Ok {
            return Err(GraphRunError::DeviceStatus(response.status));
        }
        let graph = decode_graph_response(response.clone()).map_err(GraphRunError::Report)?;
        validate_run_report(graph.report, self.request)?;
        if graph.report.phase == GraphCoordinatorPhase::Rejected
            || graph.report.realtime.state == RealtimeGraphState::Rejected
                && realtime_matches(graph.report, self.request.selection())
        {
            return Err(GraphRunError::Rejected);
        }
        self.latest = Some(graph.report);
        if graph.report.phase == GraphCoordinatorPhase::ExecutionFaulted {
            if self.fault.is_none() {
                self.fault = Some(graph.report);
            }
            self.phase = GraphRunPhase::Stopping;
            return Ok(());
        }
        if graph.report.fault != GraphCoordinatorFault::None
            || graph.report.execution.fault.is_some()
        {
            return Err(GraphRunError::Rejected);
        }
        self.phase = match action {
            GraphRunAction::Start | GraphRunAction::PollRunning => match graph.report.phase {
                GraphCoordinatorPhase::Starting => GraphRunPhase::AwaitingRunning,
                GraphCoordinatorPhase::Running if running_matches(graph.report, self.request) => {
                    GraphRunPhase::Running
                }
                GraphCoordinatorPhase::Stopping => GraphRunPhase::AwaitingStopped,
                GraphCoordinatorPhase::Active if stopped_matches(graph.report, self.request) => {
                    GraphRunPhase::Complete
                }
                _ => return Err(GraphRunError::UnexpectedPhase),
            },
            GraphRunAction::Stop | GraphRunAction::PollStopped => match graph.report.phase {
                GraphCoordinatorPhase::Stopping => GraphRunPhase::AwaitingStopped,
                GraphCoordinatorPhase::Active if stopped_matches(graph.report, self.request) => {
                    GraphRunPhase::Complete
                }
                _ => return Err(GraphRunError::UnexpectedPhase),
            },
        };
        Ok(())
    }

    /// Abandon an ambiguous I/O result without changing the retry phase.
    pub fn abandon_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }
}

fn decode_graph_response(response: Response) -> Result<GraphResponse, GraphCoordinatorReportError> {
    Ok(GraphResponse {
        status: response.status,
        report: GraphCoordinatorReport::decode(&response.body)?,
    })
}

fn selection(publication: GraphPublication) -> GraphSelection {
    GraphSelection {
        transaction_id: publication.transaction_id,
        content_digest: publication.content_digest(),
        package_digest: publication.package_digest,
    }
}

fn validate_success_identity<E>(
    response: GraphResponse,
    expected: GraphSelection,
) -> Result<(), GraphClientError<E>> {
    if response.status == StatusCode::Ok {
        validate_report_identity(response.report, expected).map_err(|_| GraphClientError::Identity)
    } else {
        Ok(())
    }
}

fn validate_success_run<E>(
    response: GraphResponse,
    expected: GraphRunRequest,
) -> Result<(), GraphClientError<E>> {
    if response.status == StatusCode::Ok {
        validate_run_report(response.report, expected).map_err(|_| GraphClientError::Identity)
    } else {
        Ok(())
    }
}

fn validate_report_identity(
    report: GraphCoordinatorReport,
    expected: GraphSelection,
) -> Result<(), ()> {
    if report.transaction_id != expected.transaction_id
        || report.content_digest != expected.content_digest
        || report.package_digest != expected.package_digest
    {
        Err(())
    } else {
        Ok(())
    }
}

fn active_matches(report: GraphCoordinatorReport, expected: GraphSelection) -> bool {
    report.active_content_digest == expected.content_digest
        && report.realtime.state == RealtimeGraphState::Active
        && report.realtime.content_digest == expected.content_digest
        && report.realtime.package_digest == expected.package_digest
        && report.realtime.active_content_digest == expected.content_digest
        && report.realtime.active_authorized
}

fn realtime_matches(report: GraphCoordinatorReport, expected: GraphSelection) -> bool {
    report.realtime.transaction_id == expected.transaction_id
        && report.realtime.content_digest == expected.content_digest
        && report.realtime.package_digest == expected.package_digest
}

fn validate_run_report(
    report: GraphCoordinatorReport,
    expected: GraphRunRequest,
) -> Result<(), ()> {
    validate_report_identity(report, expected.selection())?;
    if report.execution.run_id != expected.run_id
        || report.execution.start_cycle != expected.start_cycle
    {
        Err(())
    } else {
        Ok(())
    }
}

fn running_matches(report: GraphCoordinatorReport, expected: GraphRunRequest) -> bool {
    validate_run_report(report, expected).is_ok()
        && report.execution.service_phase == GraphActorPhase::Running
        && report.execution.realtime_phase == GraphActorPhase::Running
        && report.execution.bridge_phase == GraphBridgePhase::Running
        && report.execution.fault.is_none()
}

fn stopped_matches(report: GraphCoordinatorReport, expected: GraphRunRequest) -> bool {
    validate_run_report(report, expected).is_ok()
        && report.execution.service_phase == GraphActorPhase::Installed
        && report.execution.realtime_phase == GraphActorPhase::Installed
        && report.execution.bridge_phase == GraphBridgePhase::Empty
        && report.execution.service_next_cycle.is_none()
        && report.execution.realtime_next_cycle.is_none()
        && report.execution.fault.is_none()
}

/// Typed client framing, request encoding, report decoding, or identity failure.
#[derive(Debug)]
pub enum GraphClientError<E> {
    /// Native request/transport/response framing failed.
    Client(ClientError<E>),
    /// Canonical graph request body failed to encode.
    Wire(GraphDeploymentWireError),
    /// Canonical combined graph report failed to decode.
    Report(GraphCoordinatorReportError),
    /// Successful response named another transaction or package.
    Identity,
}

impl<E: fmt::Display> fmt::Display for GraphClientError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(error) => write!(formatter, "graph request failed: {error}"),
            Self::Wire(error) => write!(formatter, "graph request body failed: {error:?}"),
            Self::Report(error) => write!(formatter, "graph status failed: {error:?}"),
            Self::Identity => formatter.write_str("graph response identity differs"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for GraphClientError<E> {}

/// Install state-machine rejection before lifecycle state may advance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphInstallError {
    /// One request is already awaiting a response.
    RequestPending,
    /// No request is awaiting the supplied response.
    NoPendingRequest,
    /// Canonical operation body failed to encode.
    Wire(GraphDeploymentWireError),
    /// Device returned a typed non-success status.
    DeviceStatus(StatusCode),
    /// Combined service/core-1 report failed canonical decoding.
    Report(GraphCoordinatorReportError),
    /// Report named a different package or transaction.
    Identity,
    /// Either core reported a fail-closed rejection.
    Rejected,
    /// Successful report phase was impossible for the pending action.
    UnexpectedPhase,
}

impl From<()> for GraphInstallError {
    fn from((): ()) -> Self {
        Self::Identity
    }
}

impl fmt::Display for GraphInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for GraphInstallError {}

/// Exact run state-machine rejection before execution authority may advance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphRunError {
    /// One request is already awaiting a response.
    RequestPending,
    /// No request is awaiting the supplied response.
    NoPendingRequest,
    /// A stop cannot replace a start whose acceptance is still ambiguous.
    StartNotAcknowledged,
    /// Canonical start/stop body failed to encode.
    Wire(GraphDeploymentWireError),
    /// Device returned a typed non-success status.
    DeviceStatus(StatusCode),
    /// Combined service/core-1 report failed canonical decoding.
    Report(GraphCoordinatorReportError),
    /// Report named a different package, transaction, run, or epoch.
    Identity,
    /// Package lifecycle or either core rejected the run.
    Rejected,
    /// Successful report phase or actor agreement contradicted the pending action.
    UnexpectedPhase,
}

impl From<()> for GraphRunError {
    fn from((): ()) -> Self {
        Self::Identity
    }
}

impl fmt::Display for GraphRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for GraphRunError {}

#[cfg(test)]
mod tests {
    use alumina_graph_ir::{
        BOOLEAN_LATEST_STATE_BYTES, BOOLEAN_STREAM_ITEM_BYTES, GraphIrChannel, GraphIrChannelOwner,
        GraphIrDomain, GraphIrFullPolicy, GraphIrHeader, GraphIrNode, GraphIrOpcode,
        GraphIrSchedule, GraphIrSummary,
    };
    use alumina_protocol::{DeviceId, Digest};
    use alumina_runtime::graph::{
        GraphDeploymentFault, GraphExecutionFault, GraphExecutionReport, GraphFaultObservation,
        RealtimeGraphReport,
    };

    use super::*;

    const LIMITS: CacheLimits = CacheLimits {
        maximum_object_bytes: GRAPH_IR_PACKAGE_BYTES as u64,
        maximum_chunk_bytes: 512,
        maximum_chunks: 64,
    };
    const CHUNK_BYTES: u32 = 173;

    fn digest(byte: u8) -> Digest {
        Digest([byte; 32])
    }

    fn package() -> GraphIrPackage {
        GraphIrPackage::encode(
            GraphIrHeader {
                device_id: DeviceId([1; 16]),
                graph_digest: digest(2),
                implementation_digest: digest(3),
                capability_digest: digest(4),
                config_digest: digest(5),
                service_schedule: GraphIrSchedule {
                    clock_id: 10,
                    period_cycles: 1_000,
                    total_wcet_cycles: 20,
                    executor_reserve_cycles: 100,
                    node_count: 1,
                },
                realtime_schedule: GraphIrSchedule {
                    clock_id: 20,
                    period_cycles: 2_000,
                    total_wcet_cycles: 40,
                    executor_reserve_cycles: 100,
                    node_count: 2,
                },
                total_state_bytes: BOOLEAN_LATEST_STATE_BYTES,
                service_state_bytes: 0,
                realtime_state_bytes: BOOLEAN_LATEST_STATE_BYTES,
                channel_storage_bytes: 3 * BOOLEAN_STREAM_ITEM_BYTES,
                bridge_storage_bytes: 2 * BOOLEAN_STREAM_ITEM_BYTES,
            },
            &[
                GraphIrNode {
                    graph_node_id: 1,
                    domain: GraphIrDomain::Service,
                    opcode: GraphIrOpcode::BooleanStreamConstant,
                    schedule_clock_id: 10,
                    period_cycles: 1_000,
                    wcet_cycles: 20,
                    state_offset: 0,
                    state_bytes: 0,
                    parameter: 1,
                },
                GraphIrNode {
                    graph_node_id: 2,
                    domain: GraphIrDomain::Realtime,
                    opcode: GraphIrOpcode::BooleanLatest,
                    schedule_clock_id: 20,
                    period_cycles: 2_000,
                    wcet_cycles: 20,
                    state_offset: 0,
                    state_bytes: BOOLEAN_LATEST_STATE_BYTES,
                    parameter: 0,
                },
                GraphIrNode {
                    graph_node_id: 3,
                    domain: GraphIrDomain::Realtime,
                    opcode: GraphIrOpcode::BooleanStreamSink,
                    schedule_clock_id: 20,
                    period_cycles: 2_000,
                    wcet_cycles: 20,
                    state_offset: BOOLEAN_LATEST_STATE_BYTES,
                    state_bytes: 0,
                    parameter: 0,
                },
            ],
            &[
                GraphIrChannel {
                    graph_wire_id: 1,
                    source_node: 0,
                    target_node: 1,
                    owner: GraphIrChannelOwner::ServiceToRealtime,
                    full_policy: GraphIrFullPolicy::Fault,
                    capacity: 2,
                    item_bytes: BOOLEAN_STREAM_ITEM_BYTES,
                    storage_offset: 0,
                    storage_bytes: 2 * BOOLEAN_STREAM_ITEM_BYTES,
                },
                GraphIrChannel {
                    graph_wire_id: 2,
                    source_node: 1,
                    target_node: 2,
                    owner: GraphIrChannelOwner::Realtime,
                    full_policy: GraphIrFullPolicy::Fault,
                    capacity: 1,
                    item_bytes: BOOLEAN_STREAM_ITEM_BYTES,
                    storage_offset: 0,
                    storage_bytes: BOOLEAN_STREAM_ITEM_BYTES,
                },
            ],
        )
        .unwrap()
    }

    fn package_upload() -> GraphPackageUpload {
        GraphPackageUpload::new(package(), UploadId(7), CHUNK_BYTES, LIMITS).unwrap()
    }

    fn selected(publication: GraphPublication) -> GraphSelection {
        GraphSelection {
            transaction_id: publication.transaction_id,
            content_digest: publication.content_digest(),
            package_digest: publication.package_digest,
        }
    }

    fn wire_summary() -> GraphIrSummary {
        GraphIrSummary {
            node_count: 3,
            channel_count: 2,
            bridge_count: 1,
            service_state_bytes: 0,
            realtime_state_bytes: 0,
            channel_storage_bytes: 0,
            bridge_storage_bytes: 0,
        }
    }

    fn receiving_report(selection: GraphSelection) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::Validating,
            fault: GraphCoordinatorFault::None,
            transaction_id: selection.transaction_id,
            content_digest: selection.content_digest,
            package_digest: selection.package_digest,
            validated_bytes: GRAPH_IR_PACKAGE_BYTES as u32,
            storage_chunks_read: 24,
            active_content_digest: Digest::ZERO,
            realtime: RealtimeGraphReport {
                state: RealtimeGraphState::Receiving,
                transaction_id: selection.transaction_id,
                content_digest: selection.content_digest,
                package_digest: selection.package_digest,
                consumed_bytes: 208,
                summary: None,
                fault: GraphDeploymentFault::None,
                active_content_digest: Digest::ZERO,
                active_authorized: false,
            },
            execution: GraphExecutionReport::empty(),
        }
    }

    fn candidate_report(selection: GraphSelection) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::CandidateValid,
            realtime: RealtimeGraphReport {
                state: RealtimeGraphState::CandidateValid,
                consumed_bytes: GRAPH_IR_PACKAGE_BYTES as u32,
                summary: Some(wire_summary()),
                ..receiving_report(selection).realtime
            },
            ..receiving_report(selection)
        }
    }

    fn activating_report(selection: GraphSelection) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::Activating,
            realtime: RealtimeGraphReport {
                state: RealtimeGraphState::Active,
                consumed_bytes: GRAPH_IR_PACKAGE_BYTES as u32,
                summary: Some(wire_summary()),
                active_content_digest: selection.content_digest,
                ..receiving_report(selection).realtime
            },
            ..receiving_report(selection)
        }
    }

    fn preparing_report(selection: GraphSelection) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::Preparing,
            ..candidate_report(selection)
        }
    }

    fn committing_report(selection: GraphSelection) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::Committing,
            ..activating_report(selection)
        }
    }

    fn active_report(selection: GraphSelection) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::Active,
            active_content_digest: selection.content_digest,
            realtime: RealtimeGraphReport {
                active_authorized: true,
                ..activating_report(selection).realtime
            },
            ..receiving_report(selection)
        }
    }

    fn response(report: GraphCoordinatorReport) -> Response {
        Response {
            status: StatusCode::Ok,
            body: report.encode().unwrap().to_vec(),
        }
    }

    fn run_request(publication: GraphPublication) -> GraphRunRequest {
        GraphRunRequest {
            transaction_id: publication.transaction_id,
            run_id: 9,
            start_cycle: DeviceCycle(100_000),
            content_digest: publication.content_digest(),
            package_digest: publication.package_digest,
            implementation_digest: publication.implementation_digest,
        }
    }

    fn starting_report(request: GraphRunRequest) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::Starting,
            execution: GraphExecutionReport {
                service_phase: GraphActorPhase::Prepared,
                realtime_phase: GraphActorPhase::Installed,
                bridge_phase: GraphBridgePhase::Primed,
                run_id: request.run_id,
                start_cycle: request.start_cycle,
                service_next_cycle: Some(DeviceCycle(request.start_cycle.0 + 1_000)),
                realtime_next_cycle: None,
                service_last_tick: Some(0),
                realtime_last_tick: None,
                fault: None,
            },
            ..active_report(request.selection())
        }
    }

    fn running_report(request: GraphRunRequest) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::Running,
            execution: GraphExecutionReport {
                service_phase: GraphActorPhase::Running,
                realtime_phase: GraphActorPhase::Running,
                bridge_phase: GraphBridgePhase::Running,
                run_id: request.run_id,
                start_cycle: request.start_cycle,
                service_next_cycle: Some(DeviceCycle(request.start_cycle.0 + 1_000)),
                realtime_next_cycle: Some(DeviceCycle(request.start_cycle.0 + 2_000)),
                service_last_tick: Some(0),
                realtime_last_tick: Some(0),
                fault: None,
            },
            ..active_report(request.selection())
        }
    }

    fn stopping_report(request: GraphRunRequest) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::Stopping,
            execution: GraphExecutionReport {
                service_phase: GraphActorPhase::Installed,
                realtime_phase: GraphActorPhase::Running,
                bridge_phase: GraphBridgePhase::Stopping,
                run_id: request.run_id,
                start_cycle: request.start_cycle,
                service_next_cycle: None,
                realtime_next_cycle: Some(DeviceCycle(request.start_cycle.0 + 2_000)),
                service_last_tick: Some(0),
                realtime_last_tick: Some(0),
                fault: None,
            },
            ..active_report(request.selection())
        }
    }

    fn stopped_report(request: GraphRunRequest) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            execution: GraphExecutionReport {
                service_phase: GraphActorPhase::Installed,
                realtime_phase: GraphActorPhase::Installed,
                bridge_phase: GraphBridgePhase::Empty,
                run_id: request.run_id,
                start_cycle: request.start_cycle,
                service_next_cycle: None,
                realtime_next_cycle: None,
                service_last_tick: Some(0),
                realtime_last_tick: Some(0),
                fault: None,
            },
            ..active_report(request.selection())
        }
    }

    fn faulted_report(request: GraphRunRequest) -> GraphCoordinatorReport {
        GraphCoordinatorReport {
            phase: GraphCoordinatorPhase::ExecutionFaulted,
            fault: GraphCoordinatorFault::Execution,
            execution: GraphExecutionReport {
                service_phase: GraphActorPhase::Faulted,
                realtime_phase: GraphActorPhase::Running,
                bridge_phase: GraphBridgePhase::Running,
                run_id: request.run_id,
                start_cycle: request.start_cycle,
                service_next_cycle: None,
                realtime_next_cycle: Some(DeviceCycle(request.start_cycle.0 + 2_000)),
                service_last_tick: Some(0),
                realtime_last_tick: Some(0),
                fault: Some(GraphFaultObservation {
                    generation: 1,
                    fault: GraphExecutionFault::SafetyNotAuthorized,
                    detail: 0,
                }),
            },
            ..active_report(request.selection())
        }
    }

    #[test]
    fn graph_package_upload_covers_exact_bytes_and_binds_both_digests() {
        let upload = package_upload();
        let plan = upload.upload_plan();
        let expected_chunks =
            u32::try_from(GRAPH_IR_PACKAGE_BYTES.div_ceil(CHUNK_BYTES as usize)).unwrap();
        assert_eq!(plan.object.kind, ObjectKind::DeployedGraph);
        assert_eq!(plan.object.byte_len, GRAPH_IR_PACKAGE_BYTES as u64);
        assert_eq!(plan.object.content, sha256(upload.package().bytes()));
        assert_eq!(plan.chunk_bytes, CHUNK_BYTES);
        assert_eq!(plan.chunk_count, expected_chunks);

        let mut replay = Vec::new();
        for index in 0..plan.chunk_count {
            let bytes = upload.chunk_bytes(index).unwrap();
            let header = upload.chunk_header(index).unwrap();
            assert_eq!(header.upload_id, plan.upload_id);
            assert_eq!(header.index, index);
            assert_eq!(header.byte_len, u32::try_from(bytes.len()).unwrap());
            assert_eq!(header.content, sha256(bytes));
            replay.extend_from_slice(bytes);
        }
        assert_eq!(replay, upload.package().bytes());
        assert!(upload.chunk_bytes(plan.chunk_count).is_none());

        let publication = upload.publication(41).unwrap();
        assert_eq!(publication.publication.object, plan.object);
        assert_eq!(publication.publication.manifest, plan.manifest);
        assert_eq!(publication.content_digest(), plan.object.content.digest);
        assert_eq!(publication.package_digest, upload.package().digest());
        assert_eq!(
            publication.implementation_digest,
            upload.package().header().implementation_digest
        );
        assert_ne!(publication.content_digest(), publication.package_digest);
    }

    #[test]
    fn install_machine_reconciles_candidate_then_authorized_active_state() {
        let publication = package_upload().publication(41).unwrap();
        let selection = selected(publication);
        let mut machine = GraphInstallMachine::new(publication).unwrap();

        let install = machine.next_request().unwrap().unwrap();
        assert_eq!(install.operation, Operation::GraphInstall);
        assert_eq!(GraphPublication::decode(&install.body), Ok(publication));
        assert_eq!(
            machine.next_request(),
            Err(GraphInstallError::RequestPending)
        );
        machine
            .accept_response(&response(receiving_report(selection)))
            .unwrap();
        assert_eq!(machine.phase(), GraphInstallPhase::AwaitingCandidate);

        let poll = machine.next_request().unwrap().unwrap();
        assert_eq!(poll.operation, Operation::GraphGet);
        assert!(poll.body.is_empty());
        machine
            .accept_response(&response(candidate_report(selection)))
            .unwrap();
        assert_eq!(machine.phase(), GraphInstallPhase::Activating);

        let activate = machine.next_request().unwrap().unwrap();
        assert_eq!(activate.operation, Operation::GraphActivate);
        assert_eq!(GraphSelection::decode(&activate.body), Ok(selection));
        machine
            .accept_response(&response(preparing_report(selection)))
            .unwrap();
        assert_eq!(machine.phase(), GraphInstallPhase::AwaitingActive);

        let poll = machine.next_request().unwrap().unwrap();
        assert_eq!(poll.operation, Operation::GraphGet);
        machine
            .accept_response(&response(committing_report(selection)))
            .unwrap();
        assert_eq!(machine.phase(), GraphInstallPhase::AwaitingActive);

        let poll = machine.next_request().unwrap().unwrap();
        assert_eq!(poll.operation, Operation::GraphGet);
        machine
            .accept_response(&response(active_report(selection)))
            .unwrap();
        assert_eq!(machine.phase(), GraphInstallPhase::Complete);
        assert_eq!(machine.latest_report(), Some(active_report(selection)));
        assert!(machine.next_request().unwrap().is_none());
    }

    #[test]
    fn ambiguous_or_foreign_responses_never_advance_install_authority() {
        let publication = package_upload().publication(41).unwrap();
        let selection = selected(publication);
        let mut machine = GraphInstallMachine::new(publication).unwrap();
        let _lost_request = machine.next_request().unwrap().unwrap();
        assert!(machine.abandon_pending());
        assert_eq!(machine.phase(), GraphInstallPhase::Installing);

        let retry = machine.next_request().unwrap().unwrap();
        assert_eq!(GraphPublication::decode(&retry.body), Ok(publication));
        let mut foreign = selection;
        foreign.transaction_id += 1;
        assert_eq!(
            machine.accept_response(&response(receiving_report(foreign))),
            Err(GraphInstallError::Identity)
        );
        assert_eq!(machine.phase(), GraphInstallPhase::Installing);
        assert!(machine.latest_report().is_none());
    }

    #[test]
    fn prior_realtime_rejection_cannot_reject_a_new_install_transaction() {
        let publication = package_upload().publication(41).unwrap();
        let selection = selected(publication);
        let mut machine = GraphInstallMachine::new(publication).unwrap();
        let _install = machine.next_request().unwrap().unwrap();
        let mut prior = selection;
        prior.transaction_id -= 1;
        let report = GraphCoordinatorReport {
            realtime: RealtimeGraphReport {
                state: RealtimeGraphState::Rejected,
                transaction_id: prior.transaction_id,
                content_digest: prior.content_digest,
                package_digest: prior.package_digest,
                consumed_bytes: 0,
                summary: None,
                fault: GraphDeploymentFault::Sequence,
                active_content_digest: Digest::ZERO,
                active_authorized: false,
            },
            ..receiving_report(selection)
        };

        machine.accept_response(&response(report)).unwrap();
        assert_eq!(machine.phase(), GraphInstallPhase::AwaitingCandidate);
        assert_eq!(machine.latest_report(), Some(report));
    }

    #[test]
    fn typed_client_rejects_a_successful_foreign_graph_identity() {
        let publication = package_upload().publication(41).unwrap();
        let mut foreign = selected(publication);
        foreign.transaction_id += 1;
        let simulator = crate::SimulatorTransport::new(move |operation, body: &[u8]| {
            assert_eq!(operation, Operation::GraphInstall);
            assert_eq!(GraphPublication::decode(body), Ok(publication));
            crate::SimulatedResponse {
                status: StatusCode::Ok,
                body: receiving_report(foreign).encode().unwrap().to_vec(),
            }
        });
        let mut client = ProtocolClient::new(simulator, digest(5));

        assert!(matches!(
            client.graph_install(publication),
            Err(GraphClientError::Identity)
        ));
    }

    #[test]
    fn run_machine_distinguishes_start_acceptance_from_dual_core_execution() {
        let publication = package_upload().publication(41).unwrap();
        let request = run_request(publication);
        let mut machine =
            GraphRunMachine::from_publication(publication, request.run_id, request.start_cycle)
                .unwrap();

        assert_eq!(
            machine.request_stop(),
            Err(GraphRunError::StartNotAcknowledged)
        );
        let start = machine.next_request().unwrap().unwrap();
        assert_eq!(start.operation, Operation::GraphStart);
        assert_eq!(GraphRunRequest::decode(&start.body), Ok(request));
        machine
            .accept_response(&response(starting_report(request)))
            .unwrap();
        assert_eq!(machine.phase(), GraphRunPhase::AwaitingRunning);

        let poll = machine.next_request().unwrap().unwrap();
        assert_eq!(poll.operation, Operation::GraphGet);
        assert!(poll.body.is_empty());
        machine
            .accept_response(&response(running_report(request)))
            .unwrap();
        assert_eq!(machine.phase(), GraphRunPhase::Running);
        assert!(machine.next_request().unwrap().is_none());

        machine.request_stop().unwrap();
        let stop = machine.next_request().unwrap().unwrap();
        assert_eq!(stop.operation, Operation::GraphStop);
        assert_eq!(GraphRunRequest::decode(&stop.body), Ok(request));
        machine
            .accept_response(&response(stopping_report(request)))
            .unwrap();
        assert_eq!(machine.phase(), GraphRunPhase::AwaitingStopped);

        let poll = machine.next_request().unwrap().unwrap();
        assert_eq!(poll.operation, Operation::GraphGet);
        machine
            .accept_response(&response(stopped_report(request)))
            .unwrap();
        assert_eq!(machine.phase(), GraphRunPhase::Complete);
        assert_eq!(machine.latest_report(), Some(stopped_report(request)));
        assert_eq!(machine.fault_report(), None);
        assert!(machine.next_request().unwrap().is_none());
    }

    #[test]
    fn run_fault_is_retained_and_makes_exact_stop_the_next_operation() {
        let publication = package_upload().publication(41).unwrap();
        let request = run_request(publication);
        let mut machine = GraphRunMachine::new(request).unwrap();

        let _start = machine.next_request().unwrap().unwrap();
        machine
            .accept_response(&response(starting_report(request)))
            .unwrap();
        let _poll = machine.next_request().unwrap().unwrap();
        let fault = faulted_report(request);
        machine.accept_response(&response(fault)).unwrap();

        assert_eq!(machine.phase(), GraphRunPhase::Stopping);
        assert_eq!(machine.fault_report(), Some(fault));
        let stop = machine.next_request().unwrap().unwrap();
        assert_eq!(stop.operation, Operation::GraphStop);
        assert_eq!(GraphRunRequest::decode(&stop.body), Ok(request));
    }

    #[test]
    fn typed_run_client_rejects_a_successful_foreign_epoch() {
        let publication = package_upload().publication(41).unwrap();
        let request = run_request(publication);
        let mut foreign = request;
        foreign.run_id += 1;
        let simulator = crate::SimulatorTransport::new(move |operation, body: &[u8]| {
            assert_eq!(operation, Operation::GraphStart);
            assert_eq!(GraphRunRequest::decode(body), Ok(request));
            crate::SimulatedResponse {
                status: StatusCode::Ok,
                body: starting_report(foreign).encode().unwrap().to_vec(),
            }
        });
        let mut client = ProtocolClient::new(simulator, digest(5));

        assert!(matches!(
            client.graph_start(request),
            Err(GraphClientError::Identity)
        ));
    }
}
