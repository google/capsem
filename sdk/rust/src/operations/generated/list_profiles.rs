// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

pub async fn list_profiles(
    transport: &Transport,
    options: CallOptions,
) -> crate::Result<capsem_api::ProfilesListResponse> {
    let request = Request {
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/profiles/list", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
