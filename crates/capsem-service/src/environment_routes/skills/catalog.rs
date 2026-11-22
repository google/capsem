//! Discover skills from the currently configured, verified OCI catalog.
use super::*;
use capsem_api::environments::{
    CatalogEntryQuery, CatalogListQuery, SkillCatalogDetailResponse, SkillCatalogListResponse,
};
use capsem_config::catalogs::SkillCatalogEntry;

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Query(query): Query<CatalogListQuery>,
) -> Result<Json<SkillCatalogListResponse>, AppError> {
    let page = catalog_discovery::list(&actor, &state, query, |entry: &SkillCatalogEntry| {
        Ok(entry.metadata.clone())
    })
    .await?;
    Ok(Json(SkillCatalogListResponse {
        catalog: page.catalog,
        skills: page.entries,
        next_cursor: page.next_cursor,
    }))
}

pub(crate) async fn info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<CatalogEntryQuery>,
) -> Result<Json<SkillCatalogDetailResponse>, AppError> {
    let result = catalog_discovery::info(&actor, &state, &id, query, |selection, entry: &SkillCatalogEntry| {
        Ok(SkillCatalogDetailResponse {
            selection,
            metadata: entry.metadata.clone(),
            files: skill_files(entry)?,
        })
    })
    .await?;
    Ok(Json(result))
}
