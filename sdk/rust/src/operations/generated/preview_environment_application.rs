// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PreviewEnvironmentApplicationParams {
    pub id: String,
    pub body: capsem_api::EnvironmentApplyPreviewRequest,
}

pub async fn preview_environment_application(
    transport: &Transport,
    input: &PreviewEnvironmentApplicationParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentApplyPreviewResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/environments/{id}/apply/preview", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
