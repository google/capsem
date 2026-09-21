// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CallProfileMcpToolParams {
    pub profile_id: String,
    pub server_id: String,
    pub tool_id: String,
    pub body: capsem_api::Value,
}

pub async fn call_profile_mcp_tool(
    transport: &Transport,
    input: &CallProfileMcpToolParams,
    options: CallOptions,
) -> crate::Result<capsem_api::Value> {
    let path_profile_id = input.profile_id.to_string();
    let path_server_id = input.server_id.to_string();
    let path_tool_id = input.tool_id.to_string();
    let request = Request {
        parameters: &[
            ("profile_id", path_profile_id.as_str()),
            ("server_id", path_server_id.as_str()),
            ("tool_id", path_tool_id.as_str()),
        ],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::POST,
            "/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
