//! A saved image selects immutable content, while current policy admits it.
use super::*;
use capsem_config::environment::{EnvironmentImage, EnvironmentImageUpdatePolicy};

pub(crate) async fn available_update(
    state: &ServiceState,
    image: &EnvironmentImage,
) -> Result<Option<EnvironmentImage>, AppError> {
    if image.update_policy == EnvironmentImageUpdatePolicy::Pinned
        || (!is_catalog_name(&image.reference) && requested(&image.reference)?.identity.digest().is_some())
    {
        return Ok(None);
    }
    let admitted = pull_image_with_catalog(
        state,
        ImagePullRequest {
            image: image.reference.clone(),
            registry: None,
        },
        CatalogRead::RequireFresh,
    )
    .await?;
    let resolved_digest = admitted.resolved.digest().to_string();
    Ok((resolved_digest != image.resolved_digest).then(|| EnvironmentImage {
        resolved_digest,
        ..image.clone()
    }))
}

impl Decision<'_> {
    pub(crate) async fn resolve_saved(&mut self, image: &EnvironmentImage) -> Result<Requested, ImageError> {
        let digest = Digest::parse(&image.resolved_digest)
            .map_err(|error| ImageError::Invalid(format!("invalid saved image digest: {error:#}")))?;
        if !is_catalog_name(&image.reference) {
            let reference = requested(&image.reference)?
                .identity
                .resolve(digest)
                .map_err(|error| ImageError::Invalid(format!("saved image pin mismatch: {error:#}")))?;
            return requested(&reference.to_string());
        }
        let architecture = stage::catalog_architecture().map_err(|error| ImageError::Invalid(format!("{error:#}")))?;
        let catalog = self
            .load(CatalogRead::Cached)
            .await
            .ok_or_else(|| ImageError::Failed("saved image catalog unavailable".into()))?;
        let selected = catalog
            .catalog
            .entry(&image.reference)
            .and_then(|entry| {
                entry.versions().iter().find(|version| {
                    version.image().digest() == &digest
                        && version.contract() <= RUNTIME_CONTRACT
                        && version.platforms().contains(&architecture)
                })
            })
            .ok_or_else(|| ImageError::Refused("saved image is unlisted, revoked or incompatible".into()))?;
        requested(&selected.image().to_string())
    }

    /// A retained local workload still requires current source and digest
    /// admission. This performs no image pull or moving-tag resolution.
    pub(crate) async fn admit_saved(&mut self, selected: &Requested) -> Result<ResolvedImage, ImageError> {
        self.check_source(selected).await?;
        let digest = selected
            .identity
            .digest()
            .cloned()
            .ok_or_else(|| ImageError::Invalid("saved image requires an immutable digest".into()))?;
        let resolved = selected
            .identity
            .clone()
            .resolve(digest)
            .map_err(|error| ImageError::Invalid(format!("saved image pin mismatch: {error:#}")))?;
        if self.policy.admit(&resolved).is_err() {
            self.load(CatalogRead::Cached).await;
            self.policy
                .admit(&resolved)
                .map_err(|error| ImageError::Refused(format!("{error:#}")))?;
        }
        Ok(resolved)
    }
}
