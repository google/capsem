// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetProfileMcpInfoParams {
    pub profile_id: String,
}

pub async fn get_profile_mcp_info(
    transport: &Transport,
    input: &GetProfileMcpInfoParams,
    options: CallOptions,
) -> crate::Result<capsem_api::ProfileMcpInfoResponse> {
    let path_profile_id = input.profile_id.to_string();
    let request = Request {
        parameters: &[("profile_id", path_profile_id.as_str())],
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/profiles/{profile_id}/mcp/info", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
