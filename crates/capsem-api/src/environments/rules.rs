use super::EnvironmentRevisionReference;
use capsem_config::{DetectionLevel, SecurityRuleAction};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Compiled rules for a saved blueprint under current corporate policy.
/// A running VM can still be bound to an older revision or policy snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRuleListResponse {
    pub revision: EnvironmentRevisionReference,
    /// Enforcement order, with lower priorities evaluated first.
    pub access: Vec<EnvironmentEffectiveAccessRule>,
    /// Observations have severity and do not make access decisions.
    pub detection: Vec<EnvironmentEffectiveDetectionRule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentRuleSource {
    Builtin,
    Corporate,
    Environment,
    Catalog,
    EnvironmentMcp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentEffectiveAccessRule {
    pub id: String,
    pub name: String,
    pub condition: String,
    pub action: SecurityRuleAction,
    pub priority: i32,
    pub enabled: bool,
    pub source: EnvironmentRuleSource,
    pub locked: bool,
    #[schema(required = true)]
    pub detection_level: Option<DetectionLevel>,
    /// A direct custom rule's key in the saved specification, when editable.
    #[schema(required = true)]
    pub editable_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentEffectiveDetectionRule {
    pub id: String,
    pub name: String,
    pub condition: String,
    pub level: DetectionLevel,
    pub enabled: bool,
    pub source: EnvironmentRuleSource,
    #[schema(required = true)]
    pub editable_key: Option<String>,
}
