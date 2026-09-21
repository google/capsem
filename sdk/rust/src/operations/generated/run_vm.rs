// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RunVmParams {
    pub body: capsem_api::RunRequest,
}

pub async fn run_vm(
    transport: &Transport,
    input: &RunVmParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ExecResponse> {
    let request = Request {
        body: Some(serde_json::to_vec(&input.body)?),
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::POST, "/run", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
