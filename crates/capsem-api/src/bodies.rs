//! Archived body retrieval contracts.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum BodyEncoding {
    Utf8,
    Base64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EventBodiesResponse {
    pub event_id: String,
    pub bodies: Vec<ArchivedEventBody>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ArchivedEventBody {
    pub event_id: String,
    pub source_table: String,
    pub direction: String,
    pub content_type: Option<String>,
    pub original_bytes: u64,
    pub stored_bytes: u64,
    pub truncated: bool,
    pub truncated_for_transport: bool,
    pub body_hash: String,
    pub encoding: BodyEncoding,
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
pub struct EventBodiesQuery {
    /// Maximum decoded bytes returned per body. The service clamps this to its
    /// protocol limit.
    pub max_bytes: Option<usize>,
}
