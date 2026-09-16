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

/// How host clients may reach an exposure.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExposureAccess {
    /// An ordinary host-loopback TCP listener.
    #[default]
    LoopbackTcp,
    /// Browser HTTP carried only after a scoped preview-session admission.
    HttpPreview,
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
    #[serde(default)]
    pub access: ExposureAccess,
}

/// One live exposure as the VM owner declares it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ExposureInfo {
    /// Stable identity used to revoke it: the host port, as a string.
    pub id: String,
    /// Loopback host port that forwards to the guest.
    /// Present only for an ordinary loopback TCP listener. Browser previews
    /// use the gateway's dedicated preview origin and expose no bypass port.
    pub host_port: Option<u16>,
    pub guest_port: u16,
    pub target: ExposureTarget,
    pub access: ExposureAccess,
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
    pub fn loopback(host_port: u16, guest_port: u16, target: ExposureTarget) -> Self {
        Self {
            id: host_port.to_string(),
            host_port: Some(host_port),
            guest_port,
            target,
            access: ExposureAccess::LoopbackTcp,
        }
    }

    pub fn preview(id: String, guest_port: u16, target: ExposureTarget) -> Self {
        Self {
            id,
            host_port: None,
            guest_port,
            target,
            access: ExposureAccess::HttpPreview,
        }
    }
}

/// A single-use browser bootstrap. The token is submitted in a POST body to
/// `url`; it must never be appended to that URL.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct PreviewSessionResponse {
    pub url: String,
    pub bootstrap_token: String,
    pub expires_in_seconds: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PreviewSessionMaterial {
    pub exposure: ExposureInfo,
    pub owner_generation: String,
    pub bootstrap_token: String,
    pub expires_in_seconds: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreviewBootstrapExchangeRequest {
    pub bootstrap_token: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PreviewBootstrapExchangeResponse {
    pub session_token: String,
    pub expires_in_seconds: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewAdmissionKind {
    Request,
    WebsocketUpgrade,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreviewConnectionAdmissionRequest {
    pub session_token: String,
    pub kind: PreviewAdmissionKind,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PreviewConnectionAdmissionResponse {
    pub handoff_socket: String,
    pub handoff_token: u64,
    pub owner_generation: String,
}

#[cfg(test)]
mod tests;
