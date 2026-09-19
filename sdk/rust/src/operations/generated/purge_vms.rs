// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PurgeVmsParams {
    pub body: capsem_api::PurgeRequest,
}

pub async fn purge_vms(
    transport: &Transport,
    input: &PurgeVmsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::PurgeResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::POST, "/purge", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
