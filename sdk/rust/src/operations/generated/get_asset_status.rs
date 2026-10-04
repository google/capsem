// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

pub async fn get_asset_status(transport: &Transport, options: CallOptions) -> crate::Result<capsem_api::AssetStatus> {
    let request = Request {
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/assets/status", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
