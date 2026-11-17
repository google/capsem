//! Environment intent contains editable selectors, never authenticated actors
//! or self-attested host resolution. Service admission supplies that authority.
pub use capsem_config::environment::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSandboxTarget {
    #[schema(format = "uuid")]
    pub session_id: String,
    #[schema(format = "uuid")]
    pub generation: String,
}
impl EnvironmentSandboxTarget {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !crate::identity::canonical_uuid(&self.session_id) || !crate::identity::canonical_uuid(&self.generation) {
            return Err("environment target requires canonical session and generation identities");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionReference {
    #[schema(format = "uuid")]
    pub environment_id: String,
    #[schema(format = "uuid")]
    pub revision_id: String,
}
impl EnvironmentRevisionReference {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !crate::identity::canonical_uuid(&self.environment_id) || !crate::identity::canonical_uuid(&self.revision_id)
        {
            return Err("environment reference requires canonical environment and revision identities");
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCreateRequest {
    /// Client-chosen UUIDv4 for explicit reconciliation, never an authority token.
    #[schema(format = "uuid")]
    pub request_id: String,
    #[schema(value_type = EnvironmentSpecification<EnvironmentImageSelection, EnvironmentRepositorySelection, EnvironmentCatalogSelection>)]
    pub specification: EnvironmentDraft,
    pub reason: String,
    /// Requested target only. The service verifies live authenticated ownership
    /// and derives the requester's base environment rather than trusting a label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requesting_sandbox: Option<EnvironmentSandboxTarget>,
}
impl EnvironmentCreateRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_submission(&self.request_id, &self.reason, self.requesting_sandbox.as_ref())?;
        self.specification.validate()
    }
}
impl std::fmt::Debug for EnvironmentCreateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentCreateRequest").finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentChangeRequest {
    #[schema(format = "uuid")]
    pub request_id: String,
    /// Exact observed base. Changed meaning requires another review.
    #[schema(format = "uuid")]
    pub base_revision_id: String,
    #[schema(value_type = EnvironmentSpecification<EnvironmentImageSelection, EnvironmentRepositorySelection, EnvironmentCatalogSelection>)]
    pub specification: EnvironmentDraft,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requesting_sandbox: Option<EnvironmentSandboxTarget>,
}
impl EnvironmentChangeRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_submission(&self.request_id, &self.reason, self.requesting_sandbox.as_ref())?;
        if !crate::identity::canonical_uuid(&self.base_revision_id) {
            return Err("environment change requires a canonical base revision identity".into());
        }
        self.specification.validate()
    }
}
impl std::fmt::Debug for EnvironmentChangeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentChangeRequest").finish_non_exhaustive()
    }
}
fn validate_submission(id: &str, reason: &str, target: Option<&EnvironmentSandboxTarget>) -> Result<(), String> {
    if !crate::identity::canonical_uuid(id) || id.as_bytes()[14] != b'4' {
        return Err("environment request requires a canonical UUIDv4 request identity".into());
    }
    crate::requests::validate_reason(reason).map_err(str::to_owned)?;
    if let Some(target) = target {
        target.validate().map_err(str::to_owned)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
