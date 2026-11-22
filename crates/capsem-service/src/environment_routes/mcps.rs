//! Published MCP catalog inspection has no runtime or credential capability.
use super::*;
use axum::extract::Query;
use capsem_api::environments::{
    CatalogEntryQuery, CatalogListQuery, McpCatalogDetailResponse, McpCatalogListResponse, McpCatalogSummary,
};
use capsem_config::catalogs::{McpCatalogDefinition, McpCatalogEntry};

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Query(query): Query<CatalogListQuery>,
) -> Result<Json<McpCatalogListResponse>, AppError> {
    let page = catalog_discovery::list(&actor, &state, query, |entry: &McpCatalogEntry| {
        Ok(McpCatalogSummary {
            metadata: entry.metadata.clone(),
            kind: definition(entry)?.kind(),
        })
    })
    .await?;
    Ok(Json(McpCatalogListResponse {
        catalog: page.catalog,
        mcps: page.entries,
        next_cursor: page.next_cursor,
    }))
}

pub(crate) async fn info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<CatalogEntryQuery>,
) -> Result<Json<McpCatalogDetailResponse>, AppError> {
    let detail = catalog_discovery::info(&actor, &state, &id, query, |selection, entry: &McpCatalogEntry| {
        Ok(McpCatalogDetailResponse {
            selection,
            metadata: entry.metadata.clone(),
            definition: definition(entry)?.clone(),
            published_tools: entry.published_tools.clone(),
        })
    })
    .await?;
    Ok(Json(detail))
}

fn definition(entry: &McpCatalogEntry) -> Result<&McpCatalogDefinition, AppError> {
    entry
        .definition
        .as_ref()
        .ok_or_else(|| catalog_discovery::resolution_error(EnvironmentResolutionError::InvalidPin))
}
