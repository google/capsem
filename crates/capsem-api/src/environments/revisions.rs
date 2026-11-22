use super::*;
use crate::requests::RequestReviewAvailability;

#[derive(Clone, Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct EnvironmentRevisionListQuery {
    /// Continue into older revisions from the cursor on the previous page.
    pub before: Option<i64>,
    /// Maximum returned revisions, from 1 to 64. Defaults to 24.
    pub limit: Option<u16>,
}

impl EnvironmentRevisionListQuery {
    pub fn validated_limit(&self) -> Result<u16, &'static str> {
        let limit = self.limit.unwrap_or(24);
        if !(1..=64).contains(&limit) || self.before.is_some_and(|cursor| cursor <= 0) {
            return Err("invalid revision history query");
        }
        Ok(limit)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionSummary {
    pub environment: EnvironmentRevisionReference,
    pub previous_revision_id: Option<String>,
    pub fork_parent: Option<EnvironmentRevisionReference>,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub reason: String,
    pub request_id: Option<String>,
    pub replayable: bool,
}

impl std::fmt::Debug for EnvironmentRevisionSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentRevisionSummary")
            .field("environment", &self.environment)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionListResponse {
    pub revisions: Vec<EnvironmentRevisionSummary>,
    pub next_cursor: Option<i64>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionDetailResponse {
    pub summary: EnvironmentRevisionSummary,
    pub availability: RequestReviewAvailability,
    /// Exact canonical serialization, present only for complete history.
    pub canonical_toml: Option<String>,
    /// Retained fields of a purged revision, never replayable.
    #[schema(value_type = Option<EnvironmentSpecification<EnvironmentImage, EnvironmentRepository, EnvironmentCatalogPin, EnvironmentMcpDefinition>>)]
    pub partial_specification: Option<Box<EnvironmentSpecification>>,
    pub purged_variables: Vec<String>,
}

impl std::fmt::Debug for EnvironmentRevisionDetailResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentRevisionDetailResponse")
            .field("summary", &self.summary)
            .field("availability", &self.availability)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentTomlValidationRequest {
    pub toml: String,
}

impl std::fmt::Debug for EnvironmentTomlValidationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentTomlValidationRequest")
            .field("bytes", &self.toml.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentTomlValidationResponse {
    pub base: EnvironmentRevisionReference,
    /// Validated selectors for the existing change and review operation.
    #[schema(value_type = EnvironmentSpecification<EnvironmentImageSelection, EnvironmentRepositorySelection, EnvironmentCatalogSelection, EnvironmentMcpCatalogInput>)]
    pub specification: EnvironmentDraft,
}
