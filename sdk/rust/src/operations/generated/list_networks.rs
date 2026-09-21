// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

pub async fn list_networks(
    transport: &Transport,
    options: CallOptions,
) -> crate::Result<capsem_api::NetworkListResponse> {
    let request = Request {
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::GET, "/networks", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
