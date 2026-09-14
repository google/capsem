// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateNetworkParams {
    pub body: capsem_api::CreateNetworkRequest,
}

pub async fn create_network(
    transport: &Transport,
    input: &CreateNetworkParams,
    options: CallOptions,
) -> crate::Result<capsem_api::NetworkInfo> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::POST, "/networks", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
