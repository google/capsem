// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListImagesParams {
    pub refresh: Option<bool>,
}

pub async fn list_images(
    transport: &Transport,
    input: &ListImagesParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ImageListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.refresh {
        query.push(("refresh", value.to_string()));
    }
    let request = Request {
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::GET, "/images", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
