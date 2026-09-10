// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ForkVmParams {
    pub id: String,
    pub body: capsem_api::ForkRequest,
}

pub async fn fork_vm(
    transport: &Transport,
    input: &ForkVmParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ForkResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/vms/{id}/fork", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
