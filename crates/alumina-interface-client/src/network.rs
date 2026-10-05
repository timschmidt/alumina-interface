//! Retry-safe authenticated infrastructure-WLAN provisioning state.
//!
//! Credential material is retained only inside a pending join intent. It is
//! never included in a status, scan result, snapshot, or debug rendering.

use core::fmt;

use alumina_net::provisioning::{
    NetworkAuthentication, NetworkFailure, NetworkJoinRequest, NetworkMutationRequest,
    NetworkScanEntry, NetworkScanRequest, NetworkScanResult, NetworkStatus, NetworkStatusFlags,
    NetworkWireError, StationLinkState,
};
use alumina_protocol::{Digest, Operation, StatusCode};

use crate::Response;

/// One canonical network request ready for authenticated transport.
#[derive(Clone, Eq, PartialEq)]
pub struct NetworkOperation {
    operation: Operation,
    body: Vec<u8>,
}

impl NetworkOperation {
    /// Exact native operation.
    pub const fn operation(&self) -> Operation {
        self.operation
    }

    /// Canonical operation body. A join body contains the passphrase.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Provisioning is independent of active machine configuration.
    pub const fn config_digest(&self) -> Digest {
        Digest::ZERO
    }
}

impl fmt::Debug for NetworkOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkOperation")
            .field("operation", &self.operation)
            .field(
                "body",
                &if self.operation == Operation::NetworkJoin {
                    "[credential-bearing body redacted]"
                } else {
                    "[canonical body]"
                },
            )
            .finish()
    }
}

/// Observable lifecycle of the client-side provisioning transaction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NetworkClientPhase {
    /// No operation is selected.
    #[default]
    Idle,
    /// A selected operation is ready to send.
    Ready,
    /// One exact authenticated response is pending.
    AwaitingResponse,
    /// A mutation outcome is ambiguous and must be inspected before retry.
    Reconciling,
}

/// Accepted state transition from one authenticated response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkUpdate {
    /// A credential-free current status was retained.
    Status(NetworkStatus),
    /// A complete bounded scan was retained.
    Scan(NetworkScanResult),
    /// One mutation reached a proven terminal result.
    Mutation {
        /// Exact mutation operation.
        operation: Operation,
        /// Device application status inferred or returned for this mutation.
        result: StatusCode,
        /// Complete credential-free state after the mutation.
        status: NetworkStatus,
        /// Whether a later status poll proved an initially ambiguous result.
        reconciled: bool,
    },
    /// Status proved that the device never observed the retained mutation.
    RetryRequired,
}

#[derive(Clone, Eq, PartialEq)]
enum Intent {
    Status,
    Scan(NetworkScanRequest),
    Join(NetworkJoinRequest),
    Leave(NetworkMutationRequest),
    Recover(NetworkMutationRequest),
}

impl Intent {
    const fn operation(&self) -> Operation {
        match self {
            Self::Status => Operation::NetworkStatus,
            Self::Scan(_) => Operation::NetworkScan,
            Self::Join(_) => Operation::NetworkJoin,
            Self::Leave(_) => Operation::NetworkLeave,
            Self::Recover(_) => Operation::NetworkRecoverAp,
        }
    }

    fn transaction_id(&self) -> Option<u64> {
        match self {
            Self::Status => None,
            Self::Scan(request) => Some(request.transaction_id),
            Self::Join(request) => Some(request.transaction_id),
            Self::Leave(request) | Self::Recover(request) => Some(request.transaction_id),
        }
    }

    fn expected_generation(&self) -> Option<u32> {
        match self {
            Self::Join(request) => Some(request.expected_generation),
            Self::Leave(request) | Self::Recover(request) => Some(request.expected_generation),
            Self::Status | Self::Scan(_) => None,
        }
    }

    const fn mutation(&self) -> bool {
        matches!(self, Self::Join(_) | Self::Leave(_) | Self::Recover(_))
    }

    fn operation_body(&self) -> Result<NetworkOperation, NetworkClientError> {
        let body = match self {
            Self::Status => Vec::new(),
            Self::Scan(request) => request.encode()?.to_vec(),
            Self::Join(request) => request.encode().to_vec(),
            Self::Leave(request) | Self::Recover(request) => request.encode()?.to_vec(),
        };
        Ok(NetworkOperation {
            operation: self.operation(),
            body,
        })
    }
}

impl fmt::Debug for Intent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status => formatter.write_str("Status"),
            Self::Scan(request) => formatter.debug_tuple("Scan").field(request).finish(),
            Self::Join(request) => formatter.debug_tuple("Join").field(request).finish(),
            Self::Leave(request) => formatter.debug_tuple("Leave").field(request).finish(),
            Self::Recover(request) => formatter.debug_tuple("Recover").field(request).finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum State {
    Ready(Intent),
    Awaiting {
        intent: Intent,
        reconciliation: bool,
    },
    Reconciling(Intent),
}

/// Boot-scoped retry-safe network provisioning owner.
#[derive(Clone, Eq, PartialEq)]
pub struct NetworkProvisioningMachine {
    next_transaction_id: u64,
    state: Option<State>,
    latest_status: Option<NetworkStatus>,
    latest_scan: Option<NetworkScanResult>,
}

impl NetworkProvisioningMachine {
    /// Start with a caller-selected random nonzero transaction namespace.
    ///
    /// The seed must leave room for at least one subsequent transaction. A
    /// browser worker should use a reload-resistant random/epoch seed and own
    /// all network mutations for that authenticated boot.
    pub const fn starting_at(transaction_seed: u64) -> Result<Self, NetworkClientError> {
        if transaction_seed == 0 || transaction_seed == u64::MAX {
            return Err(NetworkClientError::TransactionSeed);
        }
        Ok(Self {
            next_transaction_id: transaction_seed,
            state: None,
            latest_status: None,
            latest_scan: None,
        })
    }

    /// Current split-phase transaction state.
    pub const fn phase(&self) -> NetworkClientPhase {
        match self.state {
            None => NetworkClientPhase::Idle,
            Some(State::Ready(_)) => NetworkClientPhase::Ready,
            Some(State::Awaiting { .. }) => NetworkClientPhase::AwaitingResponse,
            Some(State::Reconciling(_)) => NetworkClientPhase::Reconciling,
        }
    }

    /// Latest independently decoded credential-free device state.
    pub const fn latest_status(&self) -> Option<NetworkStatus> {
        self.latest_status
    }

    /// Latest independently decoded bounded scan result.
    pub const fn latest_scan(&self) -> Option<NetworkScanResult> {
        self.latest_scan
    }

    /// Select a side-effect-free current-status poll.
    pub fn request_status(&mut self) -> Result<(), NetworkClientError> {
        self.require_idle()?;
        self.state = Some(State::Ready(Intent::Status));
        Ok(())
    }

    /// Select a fresh bounded scan.
    pub fn request_scan(&mut self) -> Result<u64, NetworkClientError> {
        self.require_idle()?;
        let transaction_id = self.allocate_transaction()?;
        self.state = Some(State::Ready(Intent::Scan(NetworkScanRequest {
            transaction_id,
        })));
        Ok(transaction_id)
    }

    /// Select an exact scanned BSSID for association.
    pub fn request_join(
        &mut self,
        entry: NetworkScanEntry,
        passphrase: &str,
    ) -> Result<u64, NetworkClientError> {
        self.require_idle()?;
        let status = self
            .latest_status
            .ok_or(NetworkClientError::StatusRequired)?;
        let transaction_id = self.allocate_transaction()?;
        let request = NetworkJoinRequest::try_new(
            transaction_id,
            status.generation,
            entry.ssid(),
            entry.authentication,
            passphrase,
            Some(entry.bssid),
            Some(entry.channel),
        )?;
        self.state = Some(State::Ready(Intent::Join(request)));
        Ok(transaction_id)
    }

    /// Select an explicit infrastructure WLAN without a preceding scan.
    #[allow(clippy::too_many_arguments)]
    pub fn request_join_explicit(
        &mut self,
        ssid: &str,
        authentication: NetworkAuthentication,
        passphrase: &str,
        bssid: Option<[u8; 6]>,
        channel: Option<u8>,
    ) -> Result<u64, NetworkClientError> {
        self.require_idle()?;
        let status = self
            .latest_status
            .ok_or(NetworkClientError::StatusRequired)?;
        let transaction_id = self.allocate_transaction()?;
        let request = NetworkJoinRequest::try_new(
            transaction_id,
            status.generation,
            ssid,
            authentication,
            passphrase,
            bssid,
            channel,
        )?;
        self.state = Some(State::Ready(Intent::Join(request)));
        Ok(transaction_id)
    }

    /// Select a station leave while retaining the protected device AP.
    pub fn request_leave(&mut self) -> Result<u64, NetworkClientError> {
        self.select_mutation(false)
    }

    /// Select explicit recreation of the protected device AP.
    pub fn request_recover_ap(&mut self) -> Result<u64, NetworkClientError> {
        self.select_mutation(true)
    }

    /// Produce the next canonical request and mark it in flight.
    pub fn next_request(&mut self) -> Result<Option<NetworkOperation>, NetworkClientError> {
        let Some(state) = self.state.take() else {
            return Ok(None);
        };
        match state {
            State::Ready(intent) => {
                let request = intent.operation_body()?;
                self.state = Some(State::Awaiting {
                    intent,
                    reconciliation: false,
                });
                Ok(Some(request))
            }
            State::Reconciling(intent) => {
                self.state = Some(State::Awaiting {
                    intent,
                    reconciliation: true,
                });
                Ok(Some(NetworkOperation {
                    operation: Operation::NetworkStatus,
                    body: Vec::new(),
                }))
            }
            awaiting @ State::Awaiting { .. } => {
                self.state = Some(awaiting);
                Err(NetworkClientError::RequestPending)
            }
        }
    }

    /// Accept one already authenticated and natively correlated response.
    pub fn accept_response(
        &mut self,
        response: &Response,
    ) -> Result<NetworkUpdate, NetworkClientError> {
        let Some(State::Awaiting {
            intent,
            reconciliation,
        }) = self.state.take()
        else {
            return Err(NetworkClientError::NoPendingRequest);
        };
        if reconciliation {
            self.accept_reconciliation(intent, response)
        } else {
            self.accept_direct(intent, response)
        }
    }

    /// Resolve an ambiguous transport result without blindly repeating a mutation.
    pub fn abandon_pending(&mut self) -> bool {
        let Some(State::Awaiting { intent, .. }) = self.state.take() else {
            return false;
        };
        self.state = Some(if intent.mutation() {
            State::Reconciling(intent)
        } else {
            State::Ready(intent)
        });
        true
    }

    /// Erase boot-scoped observations and credential-bearing pending state.
    pub fn reset(&mut self, transaction_seed: u64) -> Result<(), NetworkClientError> {
        *self = Self::starting_at(transaction_seed)?;
        Ok(())
    }

    fn select_mutation(&mut self, recover: bool) -> Result<u64, NetworkClientError> {
        self.require_idle()?;
        let status = self
            .latest_status
            .ok_or(NetworkClientError::StatusRequired)?;
        let transaction_id = self.allocate_transaction()?;
        let request = NetworkMutationRequest {
            transaction_id,
            expected_generation: status.generation,
        };
        self.state = Some(State::Ready(if recover {
            Intent::Recover(request)
        } else {
            Intent::Leave(request)
        }));
        Ok(transaction_id)
    }

    fn require_idle(&self) -> Result<(), NetworkClientError> {
        if self.state.is_none() {
            Ok(())
        } else {
            Err(NetworkClientError::RequestPending)
        }
    }

    fn allocate_transaction(&mut self) -> Result<u64, NetworkClientError> {
        let transaction = self.next_transaction_id;
        self.next_transaction_id = transaction
            .checked_add(1)
            .filter(|value| *value != 0)
            .ok_or(NetworkClientError::TransactionExhausted)?;
        Ok(transaction)
    }

    fn accept_direct(
        &mut self,
        intent: Intent,
        response: &Response,
    ) -> Result<NetworkUpdate, NetworkClientError> {
        match intent {
            Intent::Status => self.accept_status_response(response),
            Intent::Scan(request) => self.accept_scan_response(request, response),
            mutation => self.accept_mutation_response(mutation, response, false),
        }
    }

    fn accept_status_response(
        &mut self,
        response: &Response,
    ) -> Result<NetworkUpdate, NetworkClientError> {
        if response.status != StatusCode::Ok {
            if !response.body.is_empty() {
                self.state = Some(State::Ready(Intent::Status));
                return Err(NetworkClientError::ResponseBody);
            }
            return Err(NetworkClientError::DeviceStatus(response.status));
        }
        let status = NetworkStatus::decode(&response.body)?;
        self.latest_status = Some(status);
        Ok(NetworkUpdate::Status(status))
    }

    fn accept_scan_response(
        &mut self,
        request: NetworkScanRequest,
        response: &Response,
    ) -> Result<NetworkUpdate, NetworkClientError> {
        if response.status != StatusCode::Ok {
            if !response.body.is_empty() {
                self.state = Some(State::Ready(Intent::Scan(request)));
                return Err(NetworkClientError::ResponseBody);
            }
            return Err(NetworkClientError::DeviceStatus(response.status));
        }
        let result = match NetworkScanResult::decode(&response.body) {
            Ok(result) if result.transaction_id == request.transaction_id => result,
            Ok(_) => {
                self.state = Some(State::Ready(Intent::Scan(request)));
                return Err(NetworkClientError::TransactionMismatch);
            }
            Err(error) => {
                self.state = Some(State::Ready(Intent::Scan(request)));
                return Err(NetworkClientError::Wire(error));
            }
        };
        self.latest_scan = Some(result);
        Ok(NetworkUpdate::Scan(result))
    }

    fn accept_mutation_response(
        &mut self,
        intent: Intent,
        response: &Response,
        reconciled: bool,
    ) -> Result<NetworkUpdate, NetworkClientError> {
        let status = match NetworkStatus::decode(&response.body) {
            Ok(status) => status,
            Err(error) => {
                self.state = Some(State::Reconciling(intent));
                return Err(NetworkClientError::Wire(error));
            }
        };
        let transaction = intent
            .transaction_id()
            .expect("mutation intents always carry a transaction");
        if status.last_transaction_id != transaction {
            self.state = Some(State::Reconciling(intent));
            return Err(NetworkClientError::TransactionMismatch);
        }
        let inferred = network_failure_status(status.last_failure);
        if response.status != inferred
            || validate_terminal_outcome(&intent, status, inferred).is_err()
        {
            self.state = Some(State::Reconciling(intent));
            return Err(NetworkClientError::Outcome);
        }
        self.latest_status = Some(status);
        Ok(NetworkUpdate::Mutation {
            operation: intent.operation(),
            result: inferred,
            status,
            reconciled,
        })
    }

    fn accept_reconciliation(
        &mut self,
        intent: Intent,
        response: &Response,
    ) -> Result<NetworkUpdate, NetworkClientError> {
        if response.status != StatusCode::Ok {
            self.state = Some(State::Reconciling(intent));
            return Err(NetworkClientError::DeviceStatus(response.status));
        }
        let status = match NetworkStatus::decode(&response.body) {
            Ok(status) => status,
            Err(error) => {
                self.state = Some(State::Reconciling(intent));
                return Err(NetworkClientError::Wire(error));
            }
        };
        self.latest_status = Some(status);
        let transaction = intent
            .transaction_id()
            .expect("reconciled intents are mutations");
        if status.last_transaction_id == transaction {
            let result = network_failure_status(status.last_failure);
            if validate_terminal_outcome(&intent, status, result).is_err() {
                self.state = Some(State::Reconciling(intent));
                return Err(NetworkClientError::Outcome);
            }
            return Ok(NetworkUpdate::Mutation {
                operation: intent.operation(),
                result,
                status,
                reconciled: true,
            });
        }
        if status.last_transaction_id < transaction
            && intent.expected_generation() == Some(status.generation)
        {
            self.state = Some(State::Ready(intent));
            return Ok(NetworkUpdate::RetryRequired);
        }
        Err(NetworkClientError::Superseded)
    }
}

impl fmt::Debug for NetworkProvisioningMachine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkProvisioningMachine")
            .field("next_transaction_id", &self.next_transaction_id)
            .field("state", &self.state)
            .field("latest_status", &self.latest_status)
            .field("latest_scan", &self.latest_scan)
            .finish()
    }
}

fn network_failure_status(failure: NetworkFailure) -> StatusCode {
    match failure {
        NetworkFailure::None => StatusCode::Ok,
        NetworkFailure::Association => StatusCode::Unauthorized,
        NetworkFailure::Timeout | NetworkFailure::Dhcp => StatusCode::Deadline,
        NetworkFailure::Interrupted => StatusCode::Conflict,
        NetworkFailure::Scan | NetworkFailure::Driver | NetworkFailure::Storage => {
            StatusCode::Internal
        }
    }
}

fn validate_terminal_outcome(
    intent: &Intent,
    status: NetworkStatus,
    result: StatusCode,
) -> Result<(), ()> {
    if result != StatusCode::Ok {
        return status
            .flags
            .contains(NetworkStatusFlags::LAST_OPERATION_FAILED)
            .then_some(())
            .ok_or(());
    }
    if status
        .flags
        .contains(NetworkStatusFlags::LAST_OPERATION_FAILED)
        || !status.flags.contains(NetworkStatusFlags::AP_EXPECTED)
    {
        return Err(());
    }
    match intent {
        Intent::Join(_) => (status.station_link == StationLinkState::Addressed
            && status
                .flags
                .contains(NetworkStatusFlags::STATION_CONFIGURED))
        .then_some(())
        .ok_or(()),
        Intent::Leave(_) | Intent::Recover(_) => (status.station_link
            == StationLinkState::Disconnected
            && !status
                .flags
                .contains(NetworkStatusFlags::STATION_CONFIGURED))
        .then_some(())
        .ok_or(()),
        Intent::Status | Intent::Scan(_) => Err(()),
    }
}

/// Rejection that cannot advance retained network authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkClientError {
    /// Initial transaction namespace was zero or exhausted on first use.
    TransactionSeed,
    /// No further nonzero monotonically increasing transaction is available.
    TransactionExhausted,
    /// An operation is already selected or awaiting reconciliation.
    RequestPending,
    /// No request exists for the supplied response.
    NoPendingRequest,
    /// A mutation requires a current generation from authenticated status.
    StatusRequired,
    /// Firmware returned a non-success application status.
    DeviceStatus(StatusCode),
    /// Failure status carried bytes whose meaning would be ambiguous.
    ResponseBody,
    /// A fixed provisioning body was malformed or noncanonical.
    Wire(NetworkWireError),
    /// Response transaction identity differed from the exact retained request.
    TransactionMismatch,
    /// Credential-free status contradicted the operation result.
    Outcome,
    /// A newer transaction or generation replaced an ambiguous mutation.
    Superseded,
}

impl From<NetworkWireError> for NetworkClientError {
    fn from(error: NetworkWireError) -> Self {
        Self::Wire(error)
    }
}

impl fmt::Display for NetworkClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "network provisioning rejected state: {self:?}")
    }
}

impl core::error::Error for NetworkClientError {}

#[cfg(test)]
mod tests {
    use alumina_net::{NetworkPhase, NetworkSupervisor};

    use super::*;

    fn provisioning_status(last_transaction_id: u64) -> NetworkStatus {
        let mut supervisor = NetworkSupervisor::new();
        supervisor.begin_access_point().unwrap();
        supervisor.access_point_ready().unwrap();
        let mut status = NetworkStatus::provisioning(supervisor);
        status.last_transaction_id = last_transaction_id;
        status
    }

    fn joined_status(generation: u32, transaction_id: u64) -> NetworkStatus {
        NetworkStatus::try_new(
            NetworkPhase::AccessPointAndStation,
            generation,
            1,
            transaction_id,
            StationLinkState::Addressed,
            NetworkAuthentication::Wpa2Personal,
            NetworkStatusFlags(
                NetworkStatusFlags::AP_EXPECTED
                    | NetworkStatusFlags::STATION_CONFIGURED
                    | NetworkStatusFlags::STATION_ASSOCIATED
                    | NetworkStatusFlags::STATION_IPV4_READY,
            ),
            6,
            -38,
            24,
            [192, 168, 1, 77],
            [192, 168, 1, 1],
            [2, 0xa1, 0x51, 0, 0, 1],
            NetworkFailure::None,
            Some("Alumina Lab"),
        )
        .unwrap()
    }

    fn response(status: StatusCode, body: Vec<u8>) -> Response {
        Response { status, body }
    }

    #[test]
    fn lost_join_response_is_proven_by_status_without_resending_credentials() {
        let mut machine = NetworkProvisioningMachine::starting_at(40).unwrap();
        machine.request_status().unwrap();
        assert_eq!(
            machine.next_request().unwrap().unwrap().operation(),
            Operation::NetworkStatus
        );
        let initial = provisioning_status(0);
        machine
            .accept_response(&response(StatusCode::Ok, initial.encode().to_vec()))
            .unwrap();
        let transaction = machine
            .request_join_explicit(
                "Alumina Lab",
                NetworkAuthentication::Wpa2Personal,
                "alumina-lab-secret",
                Some([2, 0xa1, 0x51, 0, 0, 1]),
                Some(6),
            )
            .unwrap();
        let join = machine.next_request().unwrap().unwrap();
        assert_eq!(join.operation(), Operation::NetworkJoin);
        assert!(!format!("{join:?}").contains("alumina-lab-secret"));
        assert!(machine.abandon_pending());
        assert_eq!(machine.phase(), NetworkClientPhase::Reconciling);
        let inspect = machine.next_request().unwrap().unwrap();
        assert_eq!(inspect.operation(), Operation::NetworkStatus);
        assert!(inspect.body().is_empty());
        let status = joined_status(initial.generation.wrapping_add(2), transaction);
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, status.encode().to_vec()))
                .unwrap(),
            NetworkUpdate::Mutation {
                operation: Operation::NetworkJoin,
                result: StatusCode::Ok,
                status,
                reconciled: true,
            }
        );
        assert_eq!(machine.phase(), NetworkClientPhase::Idle);
    }

    #[test]
    fn status_proves_unobserved_mutation_before_exact_retry() {
        let mut machine = NetworkProvisioningMachine::starting_at(70).unwrap();
        machine.request_status().unwrap();
        machine.next_request().unwrap();
        let initial = provisioning_status(0);
        machine
            .accept_response(&response(StatusCode::Ok, initial.encode().to_vec()))
            .unwrap();
        machine
            .request_join_explicit(
                "Alumina Lab",
                NetworkAuthentication::Wpa2Personal,
                "alumina-lab-secret",
                None,
                None,
            )
            .unwrap();
        let first = machine.next_request().unwrap().unwrap();
        machine.abandon_pending();
        machine.next_request().unwrap();
        assert_eq!(
            machine
                .accept_response(&response(StatusCode::Ok, initial.encode().to_vec()))
                .unwrap(),
            NetworkUpdate::RetryRequired
        );
        let retry = machine.next_request().unwrap().unwrap();
        assert_eq!(retry, first);
    }

    #[test]
    fn newer_generation_refuses_ambiguous_credential_replay() {
        let mut machine = NetworkProvisioningMachine::starting_at(90).unwrap();
        machine.request_status().unwrap();
        machine.next_request().unwrap();
        let initial = provisioning_status(0);
        machine
            .accept_response(&response(StatusCode::Ok, initial.encode().to_vec()))
            .unwrap();
        machine
            .request_join_explicit(
                "Alumina Lab",
                NetworkAuthentication::Wpa2Personal,
                "alumina-lab-secret",
                None,
                None,
            )
            .unwrap();
        machine.next_request().unwrap();
        machine.abandon_pending();
        machine.next_request().unwrap();
        let mut changed = initial;
        changed.generation = changed.generation.wrapping_add(1);
        assert_eq!(
            machine.accept_response(&response(StatusCode::Ok, changed.encode().to_vec())),
            Err(NetworkClientError::Superseded)
        );
        assert_eq!(machine.phase(), NetworkClientPhase::Idle);
    }
}
