// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ValidateEnvironmentTomlParams {
    pub id: String,
    pub revision_id: String,
    pub body: capsem_api::EnvironmentTomlValidationRequest,
}

pub async fn validate_environment_toml(
    transport: &Transport,
    input: &ValidateEnvironmentTomlParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentTomlValidationResponse> {
    let path_id = input.id.to_string();
    let path_revision_id = input.revision_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("revision_id", path_revision_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::POST,
            "/environments/{id}/revisions/{revision_id}/validate",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
