// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetEnvironmentReadinessParams {
    pub id: String,
    pub revision_id: String,
}

pub async fn get_environment_readiness(
    transport: &Transport,
    input: &GetEnvironmentReadinessParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentReadinessResponse> {
    let path_id = input.id.to_string();
    let path_revision_id = input.revision_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("revision_id", path_revision_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::GET,
            "/environments/{id}/revisions/{revision_id}/readiness",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
