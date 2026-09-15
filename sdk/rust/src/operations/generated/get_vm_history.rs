// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetVmHistoryParams {
    pub id: String,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub search: Option<String>,
    pub layer: Option<capsem_api::HistoryLayerFilter>,
}

pub async fn get_vm_history(
    transport: &Transport,
    input: &GetVmHistoryParams,
    options: CallOptions,
) -> crate::Result<capsem_api::HistoryResponse> {
    let mut query = vec![];
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &input.offset {
        query.push(("offset", value.to_string()));
    }
    if let Some(value) = &input.search {
        query.push(("search", value.to_string()));
    }
    if let Some(value) = &input.layer {
        query.push(("layer", crate::operations::enum_value(value)?));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/history", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
