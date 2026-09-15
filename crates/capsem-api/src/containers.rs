//! OCI workloads the service pulls, stages and starts inside a VM.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use utoipa::ToSchema;

/// An OCI image run as a VM's workload.
///
/// `Debug` shows the image and environment names only: arguments and
/// environment values can carry secrets, and registry access always does.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, ToSchema)]
pub struct ContainerSpec {
    /// `docker://IMAGE` or a registry-qualified `registry/repository:tag`.
    pub image: String,
    /// Command replacing the image's default command, as `docker run IMAGE CMD...`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Container environment, applied over the image's.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Access to a private registry, used for this pull only and never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryAccess>,
}

impl fmt::Debug for ContainerSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContainerSpec")
            .field("image", &self.image)
            .field("args", &format_args!("<{} redacted>", self.args.len()))
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("registry", &self.registry)
            .finish()
    }
}

/// Credentials and trust for one registry pull.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, ToSchema)]
pub struct RegistryAccess {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Password or token for `username`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Additional PEM certificate trusted only for this registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<String>,
}

impl fmt::Debug for RegistryAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let present = |value: &Option<String>| if value.is_some() { "<redacted>" } else { "<none>" };
        f.debug_struct("RegistryAccess")
            .field("username", &present(&self.username))
            .field("password", &present(&self.password))
            .field("ca_pem", &present(&self.ca_pem))
            .finish()
    }
}

/// Where a VM's container workload is in its life.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContainerState {
    /// The service is pulling and verifying the image.
    Pulling,
    /// Verified blobs are being written into the VM's workspace.
    Staging,
    /// The guest launcher has been started and the workload is not yet up.
    Starting,
    /// The guest reports the workload running.
    Running,
    /// The workload ended; `exit_code` says how.
    Exited,
    /// Setup or the workload failed; `error` says why.
    Failed,
}

/// A VM's container workload status.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ContainerStatusResponse {
    pub state: ContainerState,
    /// The image reference as requested.
    pub image: String,
    /// Content digest of the verified image, once pulled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Workload exit status reported by the guest, when `state` is `exited`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Why setup or the workload failed, when `state` is `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests;
