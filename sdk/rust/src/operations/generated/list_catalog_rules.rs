// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListCatalogRulesParams {
    pub search: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_catalog_rules(
    transport: &Transport,
    input: &ListCatalogRulesParams,
    options: CallOptions,
) -> crate::Result<capsem_api::RuleCatalogListResponse> {
    let mut query = vec![];
    if let Some(value) = &input.search {
        query.push(("search", value.to_string()));
    }
    if let Some(value) = &input.after {
        query.push(("after", value.to_string()));
    }
    if let Some(value) = &input.limit {
        query.push(("limit", value.to_string()));
    }
    let request = Request {
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport.request(reqwest::Method::GET, "/rules/list", request).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
