// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CloseManagedSessionParams {
    pub request_id: String,
    pub body: capsem_api::ManagedSessionCloseRequest,
}

pub async fn close_managed_session(
    transport: &Transport,
    input: &CloseManagedSessionParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ManagedSessionCloseResponse> {
    let path_request_id = input.request_id.to_string();
    let request = Request {
        parameters: &[("request_id", path_request_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/vms/managed/{request_id}/close", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
