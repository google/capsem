// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateSessionCredentialGrantParams {
    pub id: String,
    pub body: capsem_api::CreateSessionGrantRequest,
}

pub async fn create_session_credential_grant(
    transport: &Transport,
    input: &CreateSessionCredentialGrantParams,
    options: CallOptions,
) -> crate::Result<capsem_api::SessionGrantSummary> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/vms/{id}/credential-grants", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
