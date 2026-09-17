// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AttachNetworkMemberParams {
    pub id: String,
    pub vm_id: String,
}

pub async fn attach_network_member(
    transport: &Transport,
    input: &AttachNetworkMemberParams,
    options: CallOptions,
) -> crate::Result<capsem_api::NetworkInfo> {
    let path_id = input.id.to_string();
    let path_vm_id = input.vm_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("vm_id", path_vm_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::PUT, "/networks/{id}/members/{vm_id}", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
