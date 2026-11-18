//! Lifecycle wait selections are matching data, never caller or launch authority.
use crate::environments::EnvironmentRevisionReference;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Names which retained launch selection supplies a wait budget. This is
/// matching context only; the eventual mutation still requires admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VmLifecycleIntent {
    /// Start or resume the VM; an active owner's current boot remains selected.
    Start,
    /// Fork the source into a new VM using its desired environment revision.
    Fork,
    /// Create from a persistent source using its saved environment revision.
    Clone,
}

/// The exact host selection used to plan a lifecycle wait. The service must
/// compare every field with its actual selection before admitting a mutation.
/// A stopped legacy VM may have no prior spawn generation; an active owner may not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VmLifecycleSelection {
    pub id: String,
    /// Actual owner generation, or the retained last spawn when stopped.
    /// Absence is a legacy matching fact, never a wildcard for any generation.
    #[schema(format = "uuid")]
    pub generation: Option<String>,
    /// Whether an actual native owner was registered, including preparation.
    pub running: bool,
    /// Effective intended launch revision, not an applied-policy receipt.
    pub environment: Option<EnvironmentRevisionReference>,
}
impl VmLifecycleSelection {
    pub fn validate(&self) -> Result<(), &'static str> {
        crate::validate_name("VM", &self.id).map_err(|_| "lifecycle selection requires a valid VM identifier")?;
        if self.running && self.generation.is_none() {
            return Err("active lifecycle selection requires its original generation");
        }
        if self
            .generation
            .as_ref()
            .is_some_and(|value| !crate::identity::canonical_uuid(value))
        {
            return Err("lifecycle selection requires a canonical nonnil generation");
        }
        if let Some(environment) = &self.environment {
            environment.validate()?;
        }
        Ok(())
    }
    pub fn validate_target(&self, id: &str) -> Result<(), &'static str> {
        self.validate()?;
        if self.id != id {
            return Err("lifecycle selection targets a different VM");
        }
        Ok(())
    }
}

/// Timing for the original host selection. It grants no permission and makes
/// no claim that the workload has finished preparing or that policy is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VmReadinessResponse {
    pub selection: VmLifecycleSelection,
    pub startup_timeout_seconds: u32,
}

/// Omit the expectation for compatibility with earlier empty start/resume calls.
/// New clients carry a verified selection; it is checked at admission, not trusted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VmStartRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<VmLifecycleSelection>,
}
impl VmStartRequest {
    pub fn validate(&self, id: &str) -> Result<(), &'static str> {
        if let Some(expected) = &self.expected {
            expected.validate_target(id)?;
        }
        Ok(())
    }
}
