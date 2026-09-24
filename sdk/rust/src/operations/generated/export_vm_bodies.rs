// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExportVmBodiesParams {
    pub id: String,
}

pub async fn export_vm_bodies(
    transport: &Transport,
    input: &ExportVmBodiesParams,
    options: CallOptions,
) -> crate::Result<Vec<u8>> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        accept: crate::transport::MediaType::Gzip,
        options,
        ..Default::default()
    };
    transport
        .request(reqwest::Method::GET, "/vms/{id}/bodies/export.warc.gz", request)
        .await
}
