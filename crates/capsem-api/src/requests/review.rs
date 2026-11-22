//! Public request review views contain immutable inputs and read-only provenance.
use super::*;
use crate::environments::{
    EnvironmentCatalogPin, EnvironmentImage, EnvironmentMcpDefinition, EnvironmentRepository,
    EnvironmentRevisionReference, EnvironmentSandboxTarget, EnvironmentSpecification,
};

#[derive(Debug, Default, Clone, Serialize, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RequestListQuery {
    /// Continue after the cursor returned by the previous page.
    pub after: Option<i64>,
    pub limit: Option<u16>,
    pub pending_only: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    EnvironmentChange,
    EnvironmentArchive,
    EnvironmentImageUpdate,
    EnvironmentPromotion,
    EnvironmentApply,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestSummary {
    #[schema(format = "uuid")]
    pub request_id: String,
    pub kind: RequestKind,
    #[schema(format = "uuid")]
    pub environment_id: String,
    pub base: Option<EnvironmentRevisionReference>,
    pub source: Option<EnvironmentRevisionReference>,
    pub requester: Option<EnvironmentSandboxTarget>,
    pub actor: String,
    pub reason: String,
    pub timestamp_unix_ms: i64,
    pub expires_at_unix_ms: i64,
    pub decision: RequestState,
    pub application: RequestApplicationState,
    pub result: Option<EnvironmentRevisionReference>,
}

impl std::fmt::Debug for RequestSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestSummary")
            .field("request_id", &self.request_id)
            .field("kind", &self.kind)
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestListResponse {
    pub requests: Vec<RequestSummary>,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestReviewAvailability {
    Complete,
    Purged,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentReviewContent {
    pub availability: RequestReviewAvailability,
    #[schema(value_type = Option<EnvironmentSpecification<EnvironmentImage, EnvironmentRepository, EnvironmentCatalogPin, EnvironmentMcpDefinition>>)]
    pub specification: Option<Box<EnvironmentSpecification>>,
    pub purged_variables: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestDecisionDetails {
    pub actor: String,
    pub reason: String,
    pub timestamp_unix_ms: i64,
}

impl std::fmt::Debug for RequestDecisionDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestDecisionDetails")
            .field("timestamp_unix_ms", &self.timestamp_unix_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestDetailResponse {
    pub summary: RequestSummary,
    /// Immutable observation required when resolving the reviewed request.
    pub observed_revision: String,
    pub disposition: Option<RequestDecisionDetails>,
    pub base: Option<EnvironmentReviewContent>,
    pub proposed: EnvironmentReviewContent,
    pub targets: Vec<EnvironmentSandboxTarget>,
}
