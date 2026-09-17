// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

pub async fn restart_hypervisor(
    transport: &Transport,
    options: CallOptions,
) -> crate::Result<capsem_api::RestartResponse> {
    let request = Request {
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::POST, "/restart", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
