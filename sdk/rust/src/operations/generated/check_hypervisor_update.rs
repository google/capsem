// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CheckHypervisorUpdateParams {
    pub body: capsem_api::UpdateCheckRequest,
}

pub async fn check_hypervisor_update(
    transport: &Transport,
    input: &CheckHypervisorUpdateParams,
    options: CallOptions,
) -> crate::Result<capsem_api::UpdateActionResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/update/check", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
