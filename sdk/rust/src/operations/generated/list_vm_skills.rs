// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListVmSkillsParams {
    pub id: String,
    pub revision_id: Option<String>,
    pub search: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_vm_skills(
    transport: &Transport,
    input: &ListVmSkillsParams,
    options: CallOptions,
) -> crate::Result<capsem_api::VmSkillListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.revision_id {
        query.push(("revision_id", value.to_string()));
    }
    if let Some(value) = &input.search {
        query.push(("search", value.to_string()));
    }
    if let Some(value) = &input.after {
        query.push(("after", value.to_string()));
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
        .request(reqwest::Method::GET, "/vms/{id}/skills/list", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
