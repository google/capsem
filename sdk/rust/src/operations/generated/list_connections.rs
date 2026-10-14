// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListConnectionsParams {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

pub async fn list_connections(
    transport: &Transport,
    input: &ListConnectionsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ConnectionListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &input.cursor {
        query.push(("cursor", value.to_string()));
    }
    let request = Request {
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::GET, "/connections", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
