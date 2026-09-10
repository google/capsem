// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListVmSnapshotsParams {
    pub id: String,
}

pub async fn list_vm_snapshots(
    transport: &Transport,
    input: &ListVmSnapshotsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::SnapshotsList> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/snapshots/list", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
