// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct InjectCredentialParams {
    pub body: capsem_api::CredentialInjectRequest,
}

pub async fn inject_credential(
    transport: &Transport,
    input: &InjectCredentialParams,
    options: CallOptions,
) -> crate::Result<capsem_api::CredentialInjectResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/credentials/inject", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
