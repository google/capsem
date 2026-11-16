// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateAgentAccessParams {
    pub body: capsem_api::CreateAgentAccessRequest,
}

pub async fn create_agent_access(
    transport: &Transport,
    input: &CreateAgentAccessParams,
    options: CallOptions,
) -> crate::Result<capsem_api::CreateAgentAccessResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/agent-access", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
