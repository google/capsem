// Generated from sdk/specification/openapi.json. Do not edit.
use crate::transport::{CallOptions, Request, Transport};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetCatalogRuleParams {
    pub rule_id: String,
    pub version: String,
    pub snapshot: String,
}

pub async fn get_catalog_rule(
    transport: &Transport,
    input: &GetCatalogRuleParams,
    options: CallOptions,
) -> crate::Result<capsem_api::RuleCatalogDetailResponse> {
    let query = [("version", input.version.to_string()), ("snapshot", input.snapshot.to_string())];
    let path_rule_id = input.rule_id.to_string();
    let request = Request {
        parameters: &[("rule_id", path_rule_id.as_str())],
        query: &query,
        options,
        ..Default::default()
    };
    let bytes = transport
        .request(reqwest::Method::GET, "/rules/{rule_id}/info", request)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}
