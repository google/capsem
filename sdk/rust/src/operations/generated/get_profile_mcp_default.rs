// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetProfileMcpDefaultParams {
    pub profile_id: String,
}

pub async fn get_profile_mcp_default(
    transport: &Transport,
    input: &GetProfileMcpDefaultParams,
    options: CallOptions,
) -> crate::Result<capsem_api::McpDefaultPermissionResponse> {
    let path_profile_id = input.profile_id.to_string();
    let request = Request {
        parameters: &[("profile_id", path_profile_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/profiles/{profile_id}/mcp/default/info", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
