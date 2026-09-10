//! Managed service restart acknowledgement and reconnection contract.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceManager {
    Launchd,
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestartStatus {
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestartAuthentication {
    NewTokenRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RestartResponse {
    pub status: RestartStatus,
    pub manager: ServiceManager,
    /// The restarted gateway rotates its token. Obtain fresh credentials and
    /// construct a new SDK client; never replay the restart mutation.
    pub authentication: RestartAuthentication,
}
