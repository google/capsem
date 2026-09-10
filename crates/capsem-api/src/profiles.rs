use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_svg: Option<String>,
    pub availability: ProfileAvailabilitySummary,
    pub source: String,
    pub rule_count: usize,
    pub default_rule_count: usize,
    pub plugin_count: usize,
    pub mcp_server_count: usize,
    pub update_semantics: ProfileUpdateSemantics,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ProfileUpdateSemantics {
    pub new_sessions: ProfileNewSessionUpdateSemantics,
    pub existing_vms: ProfileExistingVmUpdateSemantics,
    pub upgrade_action: ProfileUpgradeAction,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileNewSessionUpdateSemantics {
    UseCurrentProfileCatalog,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileExistingVmUpdateSemantics {
    PinnedUntilRecreate,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileUpgradeAction {
    RecreateVm,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ProfileAvailabilitySummary {
    pub web: bool,
    pub shell: bool,
    pub mobile: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ProfilesListResponse {
    pub profiles: Vec<ProfileSummary>,
}
