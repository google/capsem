// Generated from sdk/specification/openapi.json. Do not edit.
use std::time::Duration;

use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CheckEnvironmentImageUpdateParams {
    pub id: String,
    pub revision_id: String,
}

pub async fn check_environment_image_update(
    transport: &Transport,
    input: &CheckEnvironmentImageUpdateParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentImageUpdateResponse> {
    let options = CallOptions {
        timeout: Some(
            options
                .timeout
                .unwrap_or_else(|| transport.timeout().max(Duration::from_millis(1020000))),
        ),
    };
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
            "/environments/{id}/revisions/{revision_id}/image-update",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
