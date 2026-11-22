// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListVmMcpToolsParams {
    pub id: String,
    pub server_id: String,
}

pub async fn list_vm_mcp_tools(
    transport: &Transport,
    input: &ListVmMcpToolsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::VmMcpToolsResponse> {
    let path_id = input.id.to_string();
    let path_server_id = input.server_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("server_id", path_server_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/mcps/{server_id}/tools", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
