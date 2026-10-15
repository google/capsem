// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CancelConnectionAuthorizationParams {
    pub transaction_id: String,
}

pub async fn cancel_connection_authorization(
    transport: &Transport,
    input: &CancelConnectionAuthorizationParams,
    options: CallOptions,
) -> crate::Result<capsem_api::AuthorizationTransaction> {
    let path_transaction_id = input.transaction_id.to_string();
    let request = Request {
        parameters: &[("transaction_id", path_transaction_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::DELETE,
            "/connections/authorizations/{transaction_id}",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
