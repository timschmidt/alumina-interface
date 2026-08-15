//! Passive observation of the active firmware machine configuration.

use core::fmt;

use alumina_config::{ConfigurationCoordinatorStatus, ConfigurationCoordinatorStatusError};
use alumina_protocol::{Digest, Operation, StatusCode};

use crate::Response;

/// Bodyless, side-effect-free active-configuration query.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConfigurationStatusRequest;

impl ConfigurationStatusRequest {
    /// Exact native operation.
    pub const fn operation(self) -> Operation {
        Operation::ConfigurationGet
    }

    /// Canonical empty request body.
    pub const fn body(self) -> &'static [u8] {
        &[]
    }

    /// A zero digest asks for the current identity without presupposing it.
    pub const fn config_digest(self) -> Digest {
        Digest::ZERO
    }
}

/// Device support state retained for one authenticated boot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConfigurationStatusAvailability {
    /// No authenticated response has been accepted.
    #[default]
    Unobserved,
    /// The selected image does not expose the configuration coordinator.
    Unsupported,
    /// A complete canonical coordinator status is retained.
    Available,
}

/// Accepted result of one passive configuration query.
///
/// The complete status remains in [`ConfigurationStatusModel`], so accepting a
/// response does not copy the comparatively large firmware report into both
/// the model and this transient update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationStatusUpdate {
    /// Firmware explicitly reports no configuration service.
    Unsupported,
    /// A canonical status was decoded and retained.
    Status {
        /// Whether this response differs from retained evidence.
        advanced: bool,
    },
}

/// Session-scoped active-configuration observer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConfigurationStatusModel {
    availability: ConfigurationStatusAvailability,
    latest: Option<ConfigurationCoordinatorStatus>,
}

impl ConfigurationStatusModel {
    /// Starts with no boot-scoped evidence.
    pub const fn new() -> Self {
        Self {
            availability: ConfigurationStatusAvailability::Unobserved,
            latest: None,
        }
    }

    /// Canonical request for the next observation.
    pub const fn request(&self) -> ConfigurationStatusRequest {
        ConfigurationStatusRequest
    }

    /// Current support state.
    pub const fn availability(&self) -> ConfigurationStatusAvailability {
        self.availability
    }

    /// Latest complete canonical status, if available.
    pub const fn latest(&self) -> Option<ConfigurationCoordinatorStatus> {
        self.latest
    }

    /// Accepts one already authenticated and natively correlated response.
    ///
    /// Rejected responses preserve prior valid evidence. An explicit
    /// `Unsupported` response clears boot-scoped status.
    pub fn accept_response(
        &mut self,
        response: &Response,
    ) -> Result<ConfigurationStatusUpdate, ConfigurationStatusClientError> {
        if response.status == StatusCode::Unsupported {
            if !response.body.is_empty() {
                return Err(ConfigurationStatusClientError::ResponseBody);
            }
            self.availability = ConfigurationStatusAvailability::Unsupported;
            self.latest = None;
            return Ok(ConfigurationStatusUpdate::Unsupported);
        }
        if response.status != StatusCode::Ok {
            if !response.body.is_empty() {
                return Err(ConfigurationStatusClientError::ResponseBody);
            }
            return Err(ConfigurationStatusClientError::DeviceStatus(
                response.status,
            ));
        }
        let status = ConfigurationCoordinatorStatus::decode(&response.body)
            .map_err(ConfigurationStatusClientError::Wire)?;
        let advanced = self.latest != Some(status)
            || self.availability != ConfigurationStatusAvailability::Available;
        self.availability = ConfigurationStatusAvailability::Available;
        self.latest = Some(status);
        Ok(ConfigurationStatusUpdate::Status { advanced })
    }

    /// Erases all evidence after a validated boot change.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Rejection of one authenticated configuration-status response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationStatusClientError {
    /// An error status incorrectly carried an operation body.
    ResponseBody,
    /// Firmware returned a typed non-success status.
    DeviceStatus(StatusCode),
    /// The fixed configuration status was malformed or noncanonical.
    Wire(ConfigurationCoordinatorStatusError),
}

impl fmt::Display for ConfigurationStatusClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResponseBody => formatter.write_str("configuration status body is not canonical"),
            Self::DeviceStatus(status) => {
                write!(formatter, "configuration status failed with {status:?}")
            }
            Self::Wire(error) => write!(formatter, "configuration status rejected: {error:?}"),
        }
    }
}

impl core::error::Error for ConfigurationStatusClientError {}

#[cfg(test)]
mod tests {
    use alumina_config::{
        ConfigurationCoordinatorFault, ConfigurationCoordinatorFlags,
        ConfigurationCoordinatorPhase, RealtimeConfigurationReport,
    };

    use super::*;

    #[test]
    fn accepts_empty_canonical_status_and_explicit_unsupported() {
        let status = ConfigurationCoordinatorStatus {
            phase: ConfigurationCoordinatorPhase::Empty,
            flags: ConfigurationCoordinatorFlags(0),
            fault: ConfigurationCoordinatorFault::None,
            operation_transaction_id: 0,
            operation_digest: Digest::ZERO,
            operation_bytes: 0,
            validated_bytes: 0,
            storage_chunks_read: 0,
            active_transaction_id: 0,
            active_digest: Digest::ZERO,
            active_bytes: 0,
            summary: None,
            realtime: RealtimeConfigurationReport::empty(),
        };
        let mut model = ConfigurationStatusModel::new();
        let response = Response {
            status: StatusCode::Ok,
            body: status.encode().unwrap().to_vec(),
        };
        assert_eq!(
            model.accept_response(&response),
            Ok(ConfigurationStatusUpdate::Status { advanced: true })
        );
        assert_eq!(model.latest(), Some(status));
        assert_eq!(
            model.accept_response(&response),
            Ok(ConfigurationStatusUpdate::Status { advanced: false })
        );
        assert_eq!(
            model.accept_response(&Response {
                status: StatusCode::Unsupported,
                body: Vec::new(),
            }),
            Ok(ConfigurationStatusUpdate::Unsupported)
        );
        assert_eq!(model.latest(), None);
    }

    #[test]
    fn malformed_or_failed_response_preserves_evidence() {
        let mut model = ConfigurationStatusModel::new();
        assert_eq!(
            model.accept_response(&Response {
                status: StatusCode::Ok,
                body: vec![0; 264],
            }),
            Err(ConfigurationStatusClientError::Wire(
                ConfigurationCoordinatorStatusError::Magic
            ))
        );
        assert_eq!(
            model.accept_response(&Response {
                status: StatusCode::Busy,
                body: Vec::new(),
            }),
            Err(ConfigurationStatusClientError::DeviceStatus(
                StatusCode::Busy
            ))
        );
        assert_eq!(
            model.availability(),
            ConfigurationStatusAvailability::Unobserved
        );
    }
}
