// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateProxyParams {
    pub body: capsem_api::CreateProxyRequest,
}

pub async fn create_proxy(
    transport: &Transport,
    input: &CreateProxyParams,
    options: CallOptions,
) -> crate::Result<capsem_api::CreateProxyResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::POST, "/proxies", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
