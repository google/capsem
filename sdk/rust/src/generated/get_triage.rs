// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetTriageParams {
    pub since: Option<String>,
    pub limit: Option<u64>,
    pub id: Option<String>,
}

pub async fn get_triage(
    transport: &Transport,
    input: &GetTriageParams,
    options: CallOptions,
) -> crate::Result<capsem_api::TriageResponse> {
    let mut query = vec![];
    if let Some(value) = &input.since {
        query.push(("since", value.to_string()));
    }
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &input.id {
        query.push(("id", value.to_string()));
    }
    let request = Request {
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::GET, "/triage", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
