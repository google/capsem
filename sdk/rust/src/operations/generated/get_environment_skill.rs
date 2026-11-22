// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetEnvironmentSkillParams {
    pub id: String,
    pub skill_id: String,
    pub revision_id: Option<String>,
}

pub async fn get_environment_skill(
    transport: &Transport,
    input: &GetEnvironmentSkillParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentSkillDetailResponse> {
    let mut query = vec![];
    if let Some(value) = &input.revision_id {
        query.push(("revision_id", value.to_string()));
    }
    let path_id = input.id.to_string();
    let path_skill_id = input.skill_id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str()), ("skill_id", path_skill_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(
            reqwest::Method::GET,
            "/environments/{id}/skills/{skill_id}/info",
            request,
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
