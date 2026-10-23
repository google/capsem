// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetImageInventoryParams {
    pub name: String,
    pub image: String,
}

pub async fn get_image_inventory(
    transport: &Transport,
    input: &GetImageInventoryParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ImageInventoryResponse> {
    let query = [("image", input.image.to_string())];
    let path_name = input.name.to_string();
    let request = Request {
        parameters: &[("name", path_name.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/images/{name}/inventory", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
