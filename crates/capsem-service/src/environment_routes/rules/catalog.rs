//! Discover published rule templates without selecting or applying them.
use super::*;
use capsem_api::environments::{
    CatalogEntryQuery, CatalogListQuery, CatalogRuleKind, RuleCatalogDetailResponse, RuleCatalogListResponse,
    RuleCatalogSummary,
};
use capsem_config::catalogs::{RuleCatalogDefinition, RuleCatalogEntry};

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Query(query): Query<CatalogListQuery>,
) -> Result<Json<RuleCatalogListResponse>, AppError> {
    let page = catalog_discovery::list(&actor, &state, query, |entry: &RuleCatalogEntry| {
        let kind = match definition(entry)? {
            RuleCatalogDefinition::Access { .. } => CatalogRuleKind::Access,
            RuleCatalogDefinition::Detection { .. } => CatalogRuleKind::Detection,
        };
        Ok(RuleCatalogSummary {
            metadata: entry.metadata.clone(),
            kind,
        })
    })
    .await?;
    Ok(Json(RuleCatalogListResponse {
        catalog: page.catalog,
        rules: page.entries,
        next_cursor: page.next_cursor,
    }))
}

pub(crate) async fn info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<CatalogEntryQuery>,
) -> Result<Json<RuleCatalogDetailResponse>, AppError> {
    let result = catalog_discovery::info(&actor, &state, &id, query, |selection, entry: &RuleCatalogEntry| {
        Ok(RuleCatalogDetailResponse {
            selection,
            metadata: entry.metadata.clone(),
            parameters: entry.parameters.clone(),
            definition: definition(entry)?.clone(),
        })
    })
    .await?;
    Ok(Json(result))
}

fn definition(entry: &RuleCatalogEntry) -> Result<&RuleCatalogDefinition, AppError> {
    entry
        .definition
        .as_ref()
        .ok_or_else(|| catalog_discovery::resolution_error(EnvironmentResolutionError::InvalidPin))
}
