// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetCatalogSkillParams {
    pub skill_id: String,
    pub version: String,
    pub snapshot: String,
}

pub async fn get_catalog_skill(
    transport: &Transport,
    input: &GetCatalogSkillParams,
    options: CallOptions,
) -> crate::Result<capsem_api::SkillCatalogDetailResponse> {
    let query = [("version", input.version.to_string()), ("snapshot", input.snapshot.to_string())];
    let path_skill_id = input.skill_id.to_string();
    let request = Request {
        parameters: &[("skill_id", path_skill_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/skills/{skill_id}/info", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
