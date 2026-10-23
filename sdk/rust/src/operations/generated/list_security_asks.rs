// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListSecurityAsksParams {
    pub id: String,
    pub limit: Option<u32>,
}

pub async fn list_security_asks(
    transport: &Transport,
    input: &ListSecurityAsksParams,
    options: CallOptions,
) -> crate::Result<capsem_api::SecurityAskListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/security/asks", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
