// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListRequestsParams {
    pub after: Option<i64>,
    pub limit: Option<u32>,
    pub pending_only: Option<bool>,
}

pub async fn list_requests(
    transport: &Transport,
    input: &ListRequestsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::RequestListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.after {
        query.push(("after", value.to_string()));
    }
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &input.pending_only {
        query.push(("pending_only", value.to_string()));
    }
    let request = Request {
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/requests/list", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
