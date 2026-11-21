// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListEnvironmentRulesParams {
    pub id: String,
    pub revision_id: Option<String>,
}

pub async fn list_environment_rules(
    transport: &Transport,
    input: &ListEnvironmentRulesParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentRuleListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.revision_id {
        query.push(("revision_id", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/environments/{id}/rules/list", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
