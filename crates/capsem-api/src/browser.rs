//! Metadata-only acknowledgement of a successfully opened host browser request.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAcknowledgeRequest {
    pub owner_generation: String,
    pub request_id: u64,
}

impl BrowserAcknowledgeRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !crate::identity::canonical_uuid(&self.owner_generation) || self.request_id == 0 {
            return Err("browser acknowledgement requires a canonical owner generation and positive request ID");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BrowserAcknowledgeResponse {
    pub vm_id: String,
    pub owner_generation: String,
    pub request_id: u64,
}

#[cfg(test)]
mod tests;
