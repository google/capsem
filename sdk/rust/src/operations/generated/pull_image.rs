// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PullImageParams {
    pub body: capsem_api::ImagePullRequest,
}

pub async fn pull_image(
    transport: &Transport,
    input: &PullImageParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ImagePullResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/images/pull", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
