// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ResolveSecurityAskParams {
    pub id: String,
    pub ask_id: String,
    pub body: capsem_api::SecurityAskResolveRequest,
}

pub async fn resolve_security_ask(
    transport: &Transport,
    input: &ResolveSecurityAskParams,
    options: CallOptions,
) -> crate::Result<capsem_api::SecurityAskRequest> {
    let path_id = input.id.to_string();
    let path_ask_id = input.ask_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("ask_id", path_ask_id.as_str())],
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::POST,
            "/vms/{id}/security/asks/{ask_id}/resolve",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
