// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetVmStatsSummaryParams {
    pub id: String,
}

pub async fn get_vm_stats_summary(
    transport: &Transport,
    input: &GetVmStatsSummaryParams,
    options: CallOptions,
) -> crate::Result<capsem_api::VmStatsSummaryResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/stats/summary", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
