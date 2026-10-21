// Generated from sdk/specification/openapi.json. Do not edit.
use std::time::Duration;

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
    let options = CallOptions {
        timeout: Some(
            options
                .timeout
                .unwrap_or_else(|| transport.timeout().max(Duration::from_millis(1020000))),
        ),
    };
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
