// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

pub async fn get_mcp_default(
    transport: &Transport,
    options: CallOptions,
) -> crate::Result<capsem_api::McpDefaultPermissionResponse> {
    let request = Request {
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/mcp/default/info", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
