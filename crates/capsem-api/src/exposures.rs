//! Loopback host ports that reach a VM's container or its own loopback.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The guest network namespace an exposure reaches. There is no fallback
/// between them.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExposureTarget {
    /// Loopback inside the VM's running container workload.
    #[default]
    Container,
    /// The VM's own loopback. Capsem's guest service ports are refused.
    Vm,
}

/// Expose a guest port on a host loopback port.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ExposureRequest {
    pub guest_port: u16,
    /// Loopback host port to listen on; 0 or absent picks a free one.
    #[serde(default)]
    pub host_port: u16,
    #[serde(default)]
    pub target: ExposureTarget,
}

/// One live exposure as the VM owner declares it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ExposureInfo {
    /// Stable identity used to revoke it: the host port, as a string.
    pub id: String,
    /// Loopback host port that forwards to the guest.
    pub host_port: u16,
    pub guest_port: u16,
    pub target: ExposureTarget,
}

/// A VM's live exposures.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ExposureListResponse {
    /// Identity of the VM owner process that holds the listeners. It changes
    /// when the owner restarts and restores them; a string so JavaScript
    /// clients keep every bit.
    pub owner_generation: String,
    pub exposures: Vec<ExposureInfo>,
}

impl ExposureInfo {
    pub fn new(host_port: u16, guest_port: u16, target: ExposureTarget) -> Self {
        Self {
            id: host_port.to_string(),
            host_port,
            guest_port,
            target,
        }
    }
}

#[cfg(test)]
mod tests;
