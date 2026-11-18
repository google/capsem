//! Transport trust is scoped to the effective policy's catalog authority.
use super::*;

impl Decision<'_> {
    pub(crate) async fn registry_access(
        &mut self,
        image: &Requested,
        mut access: RegistryAccess,
    ) -> Result<RegistryAccess, ImageError> {
        // Trust never grants source permission, including to explicit overrides.
        self.check_source(image).await?;
        if access.ca_pem.is_some() {
            return Ok(access);
        }
        let Some(source) = self.policy.catalog_source() else {
            return Ok(access);
        };
        let catalog = requested(&source.reference)?;
        if image.registry == catalog.registry {
            access.ca_pem = super::super::registry::read_catalog_ca(source)
                .await
                .map_err(|error| ImageError::Failed(format!("registry trust: {error:#}")))?;
        }
        Ok(access)
    }
}
