// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RevokeSessionCredentialGrantParams {
    pub id: String,
    pub grant_id: String,
}

pub async fn revoke_session_credential_grant(
    transport: &Transport,
    input: &RevokeSessionCredentialGrantParams,
    options: CallOptions,
) -> crate::Result<capsem_api::RevokeSessionGrantResponse> {
    let path_id = input.id.to_string();
    let path_grant_id = input.grant_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("grant_id", path_grant_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::DELETE,
            "/vms/{id}/credential-grants/{grant_id}",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
