//! Discover and read skill versions from the configured catalog.
use crate::client::Client;
use crate::{models, operations as api, Error, Result};
use models::environments::SkillCatalogListQuery;

pub struct Skills<'a>(pub(crate) &'a Client);

impl Skills<'_> {
    /// List published skill versions and the snapshot to use for detail reads.
    pub async fn list(&self, query: SkillCatalogListQuery) -> Result<models::SkillCatalogListResponse> {
        let input = api::ListCatalogSkillsParams {
            search: query.search,
            after: query.after,
            limit: query.limit.map(u32::from),
        };
        api::list_catalog_skills(&self.0.transport, &input, self.0.options).await
    }

    /// Read the complete file dictionary for the chosen version and snapshot.
    pub async fn get(
        &self,
        skill_id: &str,
        version: &str,
        catalog: &models::SkillCatalogSnapshot,
    ) -> Result<models::SkillCatalogDetailResponse> {
        let input = api::GetCatalogSkillParams {
            skill_id: skill_id.into(),
            version: version.into(),
            snapshot: catalog.snapshot.clone(),
        };
        let result = api::get_catalog_skill(&self.0.transport, &input, self.0.options).await?;
        if result.selection.id != skill_id
            || result.selection.version != version
            || result.metadata.id != skill_id
            || result.metadata.version != version
            || result.selection.catalog_reference != catalog.reference
            || result.selection.resolved_digest != catalog.digest
        {
            return Err(Error::InvalidResponse("catalog skill identity changed"));
        }
        Ok(result)
    }
}
