// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DownloadVmFileParams {
    pub id: String,
    pub path: String,
}

pub async fn download_vm_file(
    transport: &Transport,
    input: &DownloadVmFileParams,
    options: CallOptions,
) -> crate::Result<Vec<u8>> {
    let query = [("path", input.path.to_string())];
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        accept: crate::transport::MediaType::Binary,
        options,
        ..Default::default()
    };
    transport
        .request(reqwest::Method::GET, "/vms/{id}/files/content", request)
        .await
}
