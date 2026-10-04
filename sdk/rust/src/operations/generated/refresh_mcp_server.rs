// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RefreshMcpServerParams {
    pub server_id: String,
}

pub async fn refresh_mcp_server(
    transport: &Transport,
    input: &RefreshMcpServerParams,
    options: CallOptions,
) -> crate::Result<capsem_api::McpRefreshResponse> {
    let path_server_id = input.server_id.to_string();
    let request = Request {
        parameters: &[("server_id", path_server_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/mcp/servers/{server_id}/refresh", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
