// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetVmTimelineParams {
    pub id: String,
    pub trace_id: Option<String>,
    pub since: Option<String>,
    pub limit: Option<u64>,
    pub layers: Option<Vec<capsem_api::TimelineLayer>>,
}

pub async fn get_vm_timeline(
    transport: &Transport,
    input: &GetVmTimelineParams,
    options: CallOptions,
) -> crate::Result<capsem_api::TimelineResponse> {
    let mut query = vec![];
    if let Some(value) = &input.trace_id {
        query.push(("trace_id", value.to_string()));
    }
    if let Some(value) = &input.since {
        query.push(("since", value.to_string()));
    }
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &input.layers {
        query.push(("layers", crate::operations::enum_values(value)?));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/timeline", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
