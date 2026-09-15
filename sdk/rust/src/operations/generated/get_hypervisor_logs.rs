// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetHypervisorLogsParams {
    pub name: capsem_api::HostLogSource,
    pub grep: Option<String>,
    pub tail: Option<u64>,
    pub max_bytes: Option<u64>,
}

pub async fn get_hypervisor_logs(
    transport: &Transport,
    input: &GetHypervisorLogsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::HostLogsResponse> {
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
    let path_name = crate::operations::enum_value(&input.name)?;
    let request = Request {
        parameters: &[("name", path_name.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/host-logs/{name}", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
