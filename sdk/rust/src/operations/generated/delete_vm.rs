// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DeleteVmParams {
    pub id: String,
}

pub async fn delete_vm(
    transport: &Transport,
    input: &DeleteVmParams,
    options: CallOptions,
) -> crate::Result<capsem_api::VmActionResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::DELETE, "/vms/{id}/delete", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
