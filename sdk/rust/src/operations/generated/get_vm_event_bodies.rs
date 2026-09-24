// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetVmEventBodiesParams {
    pub id: String,
    pub event_id: String,
    pub max_bytes: Option<u64>,
}

pub async fn get_vm_event_bodies(
    transport: &Transport,
    input: &GetVmEventBodiesParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EventBodiesResponse> {
    let mut query = vec![];
    if let Some(value) = &input.max_bytes {
        query.push(("max_bytes", value.to_string()));
    }
    let path_id = input.id.to_string();
    let path_event_id = input.event_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("event_id", path_event_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/bodies/{event_id}", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
