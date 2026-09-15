// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListProfileMcpToolsParams {
    pub profile_id: String,
    pub server_id: String,
}

pub async fn list_profile_mcp_tools(
    transport: &Transport,
    input: &ListProfileMcpToolsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::McpToolsListResponse> {
    let path_profile_id = input.profile_id.to_string();
    let path_server_id = input.server_id.to_string();
    let request = Request {
        parameters: &[
            ("profile_id", path_profile_id.as_str()),
            ("server_id", path_server_id.as_str()),
        ],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::GET,
            "/profiles/{profile_id}/mcp/servers/{server_id}/tools/list",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
