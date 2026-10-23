// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AcknowledgeBrowserRequestParams {
    pub id: String,
    pub body: capsem_api::BrowserAcknowledgeRequest,
}

pub async fn acknowledge_browser_request(
    transport: &Transport,
    input: &AcknowledgeBrowserRequestParams,
    options: CallOptions,
) -> crate::Result<capsem_api::BrowserAcknowledgeResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/vms/{id}/browser/acknowledge", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
