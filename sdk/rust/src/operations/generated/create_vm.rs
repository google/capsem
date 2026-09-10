// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateVmParams {
    pub body: capsem_api::ProvisionRequest,
}

pub async fn create_vm(
    transport: &Transport,
    input: &CreateVmParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ProvisionResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::POST, "/vms/create", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
