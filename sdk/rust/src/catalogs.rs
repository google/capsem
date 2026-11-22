//! Discover exact published versions from the configured catalogs.
use crate::client::Client;
use crate::{models, operations as api, Error, Result};
use models::environments::CatalogListQuery;

pub struct Skills<'a>(pub(crate) &'a Client);

impl Skills<'_> {
    /// List published skill versions and the snapshot to use for detail reads.
    pub async fn list(&self, query: CatalogListQuery) -> Result<models::SkillCatalogListResponse> {
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
        catalog: &models::CatalogSnapshot,
    ) -> Result<models::SkillCatalogDetailResponse> {
        let input = api::GetCatalogSkillParams {
            skill_id: skill_id.into(),
            version: version.into(),
            snapshot: catalog.snapshot.clone(),
        };
        let result = api::get_catalog_skill(&self.0.transport, &input, self.0.options).await?;
        verify_entry(skill_id, version, catalog, &result.selection, &result.metadata)?;
        Ok(result)
    }
}

pub struct Rules<'a>(pub(crate) &'a Client);

impl Rules<'_> {
    /// List the published access and detection rules in the configured catalog.
    pub async fn list(&self, query: CatalogListQuery) -> Result<models::RuleCatalogListResponse> {
        let input = api::ListCatalogRulesParams {
            search: query.search,
            after: query.after,
            limit: query.limit.map(u32::from),
        };
        api::list_catalog_rules(&self.0.transport, &input, self.0.options).await
    }

    /// Read the typed definition and parameters of a listed rule version.
    pub async fn get(
        &self,
        rule_id: &str,
        version: &str,
        catalog: &models::CatalogSnapshot,
    ) -> Result<models::RuleCatalogDetailResponse> {
        let input = api::GetCatalogRuleParams {
            rule_id: rule_id.into(),
            version: version.into(),
            snapshot: catalog.snapshot.clone(),
        };
        let result = api::get_catalog_rule(&self.0.transport, &input, self.0.options).await?;
        verify_entry(rule_id, version, catalog, &result.selection, &result.metadata)?;
        Ok(result)
    }
}

pub struct Mcps<'a>(pub(crate) &'a Client);

impl Mcps<'_> {
    /// List published server templates from the configured catalog.
    pub async fn list(&self, query: CatalogListQuery) -> Result<models::McpCatalogListResponse> {
        let input = api::ListCatalogMcpsParams {
            search: query.search,
            after: query.after,
            limit: query.limit.map(u32::from),
        };
        api::list_catalog_mcps(&self.0.transport, &input, self.0.options).await
    }

    /// Read published configuration requirements and tool metadata for one version.
    pub async fn get(
        &self,
        mcp_id: &str,
        version: &str,
        catalog: &models::CatalogSnapshot,
    ) -> Result<models::McpCatalogDetailResponse> {
        let input = api::GetCatalogMcpParams {
            mcp_id: mcp_id.into(),
            version: version.into(),
            snapshot: catalog.snapshot.clone(),
        };
        let result = api::get_catalog_mcp(&self.0.transport, &input, self.0.options).await?;
        verify_entry(mcp_id, version, catalog, &result.selection, &result.metadata)?;
        Ok(result)
    }
}

fn verify_entry(
    id: &str,
    version: &str,
    catalog: &models::CatalogSnapshot,
    selection: &models::environments::EnvironmentCatalogPin,
    metadata: &models::environments::CatalogEntryMetadata,
) -> Result<()> {
    if selection.id != id
        || selection.version != version
        || metadata.id != id
        || metadata.version != version
        || selection.catalog_reference != catalog.reference
        || selection.resolved_digest != catalog.digest
    {
        return Err(Error::InvalidResponse("catalog entry identity changed"));
    }
    Ok(())
}
