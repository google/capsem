// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DisconnectConnectionParams {
    pub connection_id: String,
}

pub async fn disconnect_connection(
    transport: &Transport,
    input: &DisconnectConnectionParams,
    options: CallOptions,
) -> crate::Result<capsem_api::DisconnectResponse> {
    let path_connection_id = input.connection_id.to_string();
    let request = Request {
        parameters: &[("connection_id", path_connection_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::DELETE, "/connections/{connection_id}", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
