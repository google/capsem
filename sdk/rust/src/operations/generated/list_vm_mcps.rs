// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListVmMcpsParams {
    pub id: String,
}

pub async fn list_vm_mcps(
    transport: &Transport,
    input: &ListVmMcpsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::VmMcpServersResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/mcps", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
