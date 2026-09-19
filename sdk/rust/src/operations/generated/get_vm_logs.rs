// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetVmLogsParams {
    pub id: String,
    pub grep: Option<String>,
    pub tail: Option<u64>,
    pub max_bytes: Option<u64>,
}

pub async fn get_vm_logs(
    transport: &Transport,
    input: &GetVmLogsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::LogsResponse> {
    let mut query = vec![];
    if let Some(value) = &input.grep {
        query.push(("grep", value.to_string()));
    }
    if let Some(value) = &input.tail {
        query.push(("tail", value.to_string()));
    }
    if let Some(value) = &input.max_bytes {
        query.push(("max_bytes", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/logs", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
