// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetEnvironmentRevisionParams {
    pub id: String,
    pub revision_id: String,
}

pub async fn get_environment_revision(
    transport: &Transport,
    input: &GetEnvironmentRevisionParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentRevisionDetailResponse> {
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
            "/environments/{id}/revisions/{revision_id}/info",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
