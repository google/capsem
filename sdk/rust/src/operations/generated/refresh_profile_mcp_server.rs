// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RefreshProfileMcpServerParams {
    pub profile_id: String,
    pub server_id: String,
}

pub async fn refresh_profile_mcp_server(
    transport: &Transport,
    input: &RefreshProfileMcpServerParams,
    options: CallOptions,
) -> crate::Result<capsem_api::McpRefreshResponse> {
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
            reqwest::Method::POST,
            "/profiles/{profile_id}/mcp/servers/{server_id}/refresh",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
