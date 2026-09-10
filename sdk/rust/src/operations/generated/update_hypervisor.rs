// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateHypervisorParams {
    pub body: capsem_api::UpdateApplyRequest,
}

pub async fn update_hypervisor(
    transport: &Transport,
    input: &UpdateHypervisorParams,
    options: CallOptions,
) -> crate::Result<capsem_api::UpdateActionResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/update/apply", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
