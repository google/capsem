// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListSessionCredentialGrantsParams {
    pub id: String,
}

pub async fn list_session_credential_grants(
    transport: &Transport,
    input: &ListSessionCredentialGrantsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::SessionGrantListResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/vms/{id}/credential-grants", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
