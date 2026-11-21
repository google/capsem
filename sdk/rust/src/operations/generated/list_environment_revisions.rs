// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListEnvironmentRevisionsParams {
    pub id: String,
    pub before: Option<i64>,
    pub limit: Option<u32>,
}

pub async fn list_environment_revisions(
    transport: &Transport,
    input: &ListEnvironmentRevisionsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::EnvironmentRevisionListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.before {
        query.push(("before", value.to_string()));
    }
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    let path_id = input.id.to_string();
    let request = Request {
        parameters: &[("id", path_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/environments/{id}/revisions/list", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
