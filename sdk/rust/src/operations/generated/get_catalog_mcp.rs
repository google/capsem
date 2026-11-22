// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetCatalogMcpParams {
    pub mcp_id: String,
    pub version: String,
    pub snapshot: String,
}

pub async fn get_catalog_mcp(
    transport: &Transport,
    input: &GetCatalogMcpParams,
    options: CallOptions,
) -> crate::Result<capsem_api::McpCatalogDetailResponse> {
    let query = [
        ("version", input.version.to_string()),
        ("snapshot", input.snapshot.to_string()),
    ];
    let path_mcp_id = input.mcp_id.to_string();
    let request = Request {
        parameters: &[("mcp_id", path_mcp_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/mcps/{mcp_id}/info", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
