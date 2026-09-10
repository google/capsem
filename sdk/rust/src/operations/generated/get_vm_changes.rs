// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetVmChangesParams {
    pub id: String,
    pub checkpoint: String,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

pub async fn get_vm_changes(
    transport: &Transport,
    input: &GetVmChangesParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ChangesResponse> {
    let mut query = vec![("checkpoint", input.checkpoint.to_string())];
    if let Some(value) = &input.offset {
        query.push(("offset", value.to_string()));
    }
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/changes", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
