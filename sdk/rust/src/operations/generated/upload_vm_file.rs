// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UploadVmFileParams {
    pub id: String,
    pub path: String,
    pub exact: Option<bool>,
    pub body: Vec<u8>,
}

pub async fn upload_vm_file(
    transport: &Transport,
    input: &UploadVmFileParams,
    options: CallOptions,
) -> crate::Result<capsem_api::UploadResponse> {
    let mut query = vec![("path", input.path.to_string())];
    if let Some(value) = &input.exact {
        query.push(("exact", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        body: Some(input.body.clone()),
        content_type: crate::transport::MediaType::Binary,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/vms/{id}/files/content", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
