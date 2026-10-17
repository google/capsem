// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetConnectionParams {
    pub connection_id: String,
}

pub async fn get_connection(
    transport: &Transport,
    input: &GetConnectionParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ConnectionSummary> {
    let path_connection_id = input.connection_id.to_string();
    let request = Request {
        parameters: &[("connection_id", path_connection_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/connections/{connection_id}", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
