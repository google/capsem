// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ResumeVmParams {
    pub id: String,
    pub body: Option<capsem_api::VmStartRequest>,
}

pub async fn resume_vm(
    transport: &Transport,
    input: &ResumeVmParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ProvisionResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        body: input.body.as_ref().map(serde_json::to_vec).transpose()?,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::POST, "/vms/{id}/resume", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
