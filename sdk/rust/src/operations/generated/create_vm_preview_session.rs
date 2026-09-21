// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateVmPreviewSessionParams {
    pub id: String,
    pub exposure_id: String,
}

pub async fn create_vm_preview_session(
    transport: &Transport,
    input: &CreateVmPreviewSessionParams,
    options: CallOptions,
) -> crate::Result<capsem_api::PreviewSessionResponse> {
    let path_id = input.id.to_string();
    let path_exposure_id = input.exposure_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("exposure_id", path_exposure_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::POST,
            "/vms/{id}/exposures/{exposure_id}/preview-session",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
