// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListVmFilesParams {
    pub id: String,
    pub path: Option<String>,
    pub depth: Option<i64>,
}

pub async fn list_vm_files(
    transport: &Transport,
    input: &ListVmFilesParams,
    options: CallOptions,
) -> crate::Result<capsem_api::FileListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.path {
        query.push(("path", value.to_string()));
    }
    if let Some(value) = &input.depth {
        query.push(("depth", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/files/list", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
