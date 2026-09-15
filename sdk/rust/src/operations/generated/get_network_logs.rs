// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetNetworkLogsParams {
    pub id: String,
    pub cursor: Option<String>,
    pub limit: Option<u64>,
    pub vm: Option<String>,
    pub connection: Option<String>,
    pub r#type: Option<String>,
    pub decision: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

pub async fn get_network_logs(
    transport: &Transport,
    input: &GetNetworkLogsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::NetworkLogsResponse> {
    let mut query = vec![];
    if let Some(value) = &input.cursor {
        query.push(("cursor", value.to_string()));
    }
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &input.vm {
        query.push(("vm", value.to_string()));
    }
    if let Some(value) = &input.connection {
        query.push(("connection", value.to_string()));
    }
    if let Some(value) = &input.r#type {
        query.push(("type", value.to_string()));
    }
    if let Some(value) = &input.decision {
        query.push(("decision", value.to_string()));
    }
    if let Some(value) = &input.since {
        query.push(("since", value.to_string()));
    }
    if let Some(value) = &input.until {
        query.push(("until", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/networks/{id}/logs", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
