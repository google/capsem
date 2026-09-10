// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetVmSnapshotsStatusParams {
    pub id: String,
}

pub async fn get_vm_snapshots_status(
    transport: &Transport,
    input: &GetVmSnapshotsStatusParams,
    options: CallOptions,
) -> crate::Result<capsem_api::SnapshotsStatus> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/snapshots/status", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
