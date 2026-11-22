//! Environment intent contains editable selectors, never authenticated actors
//! or self-attested host resolution. Service admission supplies that authority.
pub use capsem_config::environment::*;
mod catalogs;
pub use catalogs::*;
mod plugins;
pub use plugins::*;
mod revisions;
pub use revisions::*;
mod rules;
pub use rules::*;
mod skills;
use serde::{Deserialize, Serialize};
pub use skills::*;
use utoipa::{IntoParams, ToSchema};

/// Reconciliation result for a submitted mutation, including an accepted
/// decision whose effect remains pending or failed. Request IDs are stable;
/// clients must not infer a completed revision from HTTP success alone.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentMutationResponse {
    #[schema(format = "uuid")]
    pub request_id: String,
    #[schema(format = "uuid")]
    pub environment_id: String,
    pub decision: crate::requests::RequestState,
    pub application: crate::requests::RequestApplicationState,
    pub result: Option<EnvironmentRevisionReference>,
    pub application_error: Option<String>,
    /// Exact observation available to the scoped requester for cancellation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_revision: Option<String>,
}
impl std::fmt::Debug for EnvironmentMutationResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentMutationResponse")
            .field("request_id", &self.request_id)
            .field("environment_id", &self.environment_id)
            .field("decision", &self.decision)
            .field("application", &self.application)
            .finish_non_exhaustive()
    }
}

/// Canonical current content verified against the committed host-ledger revision.
/// Effective corporate policy is a separate view of this same revision.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCurrentResponse {
    pub document: EnvironmentDocument,
    pub archived: bool,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub reason: String,
    pub request_id: Option<String>,
}
impl std::fmt::Debug for EnvironmentCurrentResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentCurrentResponse")
            .field("environment_id", &self.document.identity.id)
            .field("revision_id", &self.document.identity.revision_id)
            .finish_non_exhaustive()
    }
}

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

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPolicyListQuery {
    /// Require the current environment to match this saved revision.
    #[param(format = "uuid")]
    pub revision_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct EnvironmentListQuery {
    /// Case-insensitive substring of the environment name, description, image, or ID.
    pub search: Option<String>,
    /// Continue from the response cursor, keeping the same search and caller scope.
    pub after: Option<String>,
    /// Maximum number of returned items, from 1 to 64. Defaults to 24.
    pub limit: Option<u16>,
}

impl EnvironmentListQuery {
    pub fn validate(&self) -> Result<(), &'static str> {
        let search = self.search.as_deref().unwrap_or_default();
        if self.limit.is_some_and(|limit| !(1..=64).contains(&limit))
            || search.len() > capsem_proto::ENVIRONMENT_MAX_NAME_BYTES
            || search.chars().any(char::is_control)
            || self
                .after
                .as_deref()
                .is_some_and(|cursor| !crate::identity::canonical_uuid(cursor))
        {
            return Err("invalid environment list query");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentListResponse {
    pub environments: Vec<EnvironmentListItem>,
    /// A bounded search can return an empty page with a continuation cursor.
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentListItem {
    pub environment: EnvironmentRevisionReference,
    pub archived: bool,
    /// Missing when the current file cannot be verified against committed history.
    pub summary: Option<EnvironmentListSummary>,
    /// Number of VMs currently bound to this environment, visible to the operator.
    pub instance_count: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentListSummary {
    pub name: String,
    pub description: String,
    pub image: Option<EnvironmentImage>,
    pub timestamp_unix_ms: i64,
}

impl std::fmt::Debug for EnvironmentListSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentListSummary")
            .field("timestamp_unix_ms", &self.timestamp_unix_ms)
            .finish_non_exhaustive()
    }
}

/// Immutable timing metadata for a selected revision. This grants no launch
/// authority; current admission still applies when the workload is dispatched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentReadinessResponse {
    pub environment: EnvironmentRevisionReference,
    /// Approved startup deadline, or zero when the revision has no image startup.
    pub startup_timeout_seconds: u32,
}

/// A verified image candidate for an immutable saved revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentImageUpdateResponse {
    pub environment: EnvironmentRevisionReference,
    pub current: Option<EnvironmentImage>,
    /// Empty for an environment with no image, a pinned image, or the latest compatible digest.
    pub available: Option<EnvironmentImage>,
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
    #[schema(value_type = EnvironmentSpecification<EnvironmentImageSelection, EnvironmentRepositorySelection, EnvironmentCatalogSelection, EnvironmentMcpCatalogInput>)]
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
    #[schema(value_type = EnvironmentSpecification<EnvironmentImageSelection, EnvironmentRepositorySelection, EnvironmentCatalogSelection, EnvironmentMcpCatalogInput>)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPromotionRequest {
    #[schema(format = "uuid")]
    pub request_id: String,
    /// The parent revision observed during review.
    #[schema(format = "uuid")]
    pub base_revision_id: String,
    /// The exact reviewed, committed fork revision.
    pub source: EnvironmentRevisionReference,
    pub reason: String,
}

impl EnvironmentPromotionRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_submission(&self.request_id, &self.reason, None)?;
        if !crate::identity::canonical_uuid(&self.base_revision_id) {
            return Err("environment promotion requires a canonical base revision identity".into());
        }
        self.source.validate().map_err(str::to_owned)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentApplyTarget {
    #[schema(format = "uuid")]
    pub session_id: String,
    #[schema(format = "uuid")]
    pub generation: String,
    /// The target's desired revision observed during review.
    pub base: EnvironmentRevisionReference,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentApplyRequest {
    #[schema(format = "uuid")]
    pub request_id: String,
    /// The destination environment's current revision when reviewed.
    #[schema(format = "uuid")]
    pub base_revision_id: String,
    /// The exact saved revision to apply to the selected generations.
    #[schema(format = "uuid")]
    pub revision_id: String,
    pub targets: Vec<EnvironmentApplyTarget>,
    pub reason: String,
}

impl EnvironmentApplyRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_submission(&self.request_id, &self.reason, None)?;
        if !crate::identity::canonical_uuid(&self.base_revision_id)
            || !crate::identity::canonical_uuid(&self.revision_id)
            || self.targets.is_empty()
            || self.targets.len() > capsem_proto::APPROVAL_PENDING_TOTAL
        {
            return Err("environment application requires exact revisions and a bounded target selection".into());
        }
        let mut sessions = std::collections::BTreeSet::new();
        for target in &self.targets {
            if !crate::identity::canonical_uuid(&target.session_id)
                || !crate::identity::canonical_uuid(&target.generation)
                || !sessions.insert(&target.session_id)
            {
                return Err("environment application requires distinct targets and exact generations".into());
            }
            target.base.validate().map_err(str::to_owned)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for EnvironmentChangeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentChangeRequest").finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentArchiveAction {
    Archive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentArchiveRequest {
    pub action: EnvironmentArchiveAction,
    #[schema(format = "uuid")]
    pub request_id: String,
    #[schema(format = "uuid")]
    pub base_revision_id: String,
    pub reason: String,
}

impl EnvironmentArchiveRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_submission(&self.request_id, &self.reason, None)?;
        if !crate::identity::canonical_uuid(&self.base_revision_id) {
            return Err("environment archive requires a canonical base revision identity".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentImageUpdateAction {
    UpdateImage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentImageUpdateRequest {
    pub action: EnvironmentImageUpdateAction,
    #[schema(format = "uuid")]
    pub request_id: String,
    #[schema(format = "uuid")]
    pub base_revision_id: String,
    /// Immutable SHA-256 candidate the operator reviewed, including the `sha256:` prefix.
    pub expected_digest: String,
    pub reason: String,
}

impl EnvironmentImageUpdateRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_submission(&self.request_id, &self.reason, None)?;
        if !crate::identity::canonical_uuid(&self.base_revision_id)
            || !capsem_config::is_sha256_digest_shape(&self.expected_digest)
        {
            return Err("environment image update requires a canonical base revision and SHA-256 digest".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum EnvironmentChangeIntent {
    Edit(Box<EnvironmentChangeRequest>),
    Archive(EnvironmentArchiveRequest),
    ImageUpdate(EnvironmentImageUpdateRequest),
}

impl EnvironmentChangeIntent {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Edit(request) => &request.request_id,
            Self::Archive(request) => &request.request_id,
            Self::ImageUpdate(request) => &request.request_id,
        }
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
