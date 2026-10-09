// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct StopProxyParams {
    pub id: String,
    pub body: capsem_api::ProxyLeaseRequest,
}

pub async fn stop_proxy(
    transport: &Transport,
    input: &StopProxyParams,
    options: CallOptions,
) -> crate::Result<capsem_api::StopProxyResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/proxies/{id}/stop", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
