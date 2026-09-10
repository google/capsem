//! Combined gateway overview; SDK consumers call this Hypervisor.info.

use crate::{ProfileCatalogStatus, UpdateStatusResponse, VmAction, VmLifecycleState};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAvailability {
    Running,
    Unavailable,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct HypervisorInfo {
    pub service: ServiceAvailability,
    pub gateway_version: String,
    pub vm_count: usize,
    pub vms: Vec<VmSummary>,
    pub resource_summary: Option<ResourceSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profiles: Option<ProfileCatalogStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdateStatusResponse>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct VmSummary {
    pub id: String,
    pub name: Option<String>,
    pub status: VmLifecycleState,
    pub persistent: bool,
    pub profile_id: String,
    // Telemetry (present for running VMs, absent for stopped)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_estimated_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tool_calls: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_file_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_call_count: Option<u64>,
    #[serde(default)]
    pub can_resume: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_blocked_reason: Option<String>,
    /// Why a crashed VM died: the `process.log` tail the service captured.
    /// The service splits the two on purpose -- a defunct session carries its
    /// reason here and leaves `resume_blocked_reason` empty -- so a consumer
    /// that reads only the latter has nothing to show for the one state where
    /// the user most needs a reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub available_actions: Vec<VmAction>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ResourceSummary {
    pub total_ram_mb: u64,
    pub total_cpus: u32,
    pub running_count: usize,
    pub stopped_count: usize,
    pub suspended_count: usize,
}
