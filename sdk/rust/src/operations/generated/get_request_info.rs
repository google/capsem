// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetRequestInfoParams {
    pub id: String,
}

pub async fn get_request_info(
    transport: &Transport,
    input: &GetRequestInfoParams,
    options: CallOptions,
) -> crate::Result<capsem_api::RequestDetailResponse> {
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/requests/{id}/info", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
