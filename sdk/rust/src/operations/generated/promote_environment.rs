// Generated from sdk/specification/openapi.json. Do not edit.
use std::time::Duration;

use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PromoteEnvironmentParams {
    pub id: String,
    pub body: capsem_api::EnvironmentPromotionRequest,
}

pub async fn promote_environment(
    transport: &Transport,
    input: &PromoteEnvironmentParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentMutationResponse> {
    let options = CallOptions {
        timeout: Some(
            options
                .timeout
                .unwrap_or_else(|| transport.timeout().max(Duration::from_millis(1020000))),
        ),
    };
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/environments/{id}/promote", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
