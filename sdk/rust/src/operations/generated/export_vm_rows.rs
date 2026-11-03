// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExportVmRowsParams {
    pub id: String,
    pub tables: Option<String>,
}

pub async fn export_vm_rows(
    transport: &Transport,
    input: &ExportVmRowsParams,
    options: CallOptions,
) -> crate::Result<Vec<u8>> {
    let mut query = vec![];
    if let Some(value) = &input.tables {
        query.push(("tables", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        accept: crate::transport::MediaType::Gzip,
        options,
        ..Default::default()
    };
    transport
        .request(reqwest::Method::GET, "/vms/{id}/export/rows.ndjson.gz", request)
        .await
}
