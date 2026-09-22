// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListProfileMcpServersParams {
    pub profile_id: String,
}

pub async fn list_profile_mcp_servers(
    transport: &Transport,
    input: &ListProfileMcpServersParams,
    options: CallOptions,
) -> crate::Result<capsem_api::McpServersListResponse> {
    let path_profile_id = input.profile_id.to_string();
    let request = Request {
        parameters: &[("profile_id", path_profile_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/profiles/{profile_id}/mcp/servers/list", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
