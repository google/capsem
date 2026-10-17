// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ClaimManagedSessionParams {
    pub request_id: String,
    pub body: capsem_api::ManagedSessionClaimRequest,
}

pub async fn claim_managed_session(
    transport: &Transport,
    input: &ClaimManagedSessionParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ManagedSessionClaimResponse> {
    let path_request_id = input.request_id.to_string();
    let request = Request {
        parameters: &[("request_id", path_request_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/vms/managed/{request_id}/claim", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
