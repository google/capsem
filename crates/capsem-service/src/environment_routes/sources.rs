//! Host-owned resolution capabilities. Unconnected sources refuse resolution;
//! callers cannot replace them with self-attested pins or cached metadata.
use crate::{container_setup::images, AppError, ServiceState};
use axum::http::StatusCode;
use capsem_config::{
    catalogs::CatalogKind,
    environment::{EnvironmentCatalogPin, EnvironmentCatalogSelection},
};
use capsem_core::environment::{EnvironmentPinResolver, EnvironmentResolutionError};
use std::sync::Arc;

pub(crate) mod catalogs;
mod repositories;

pub(super) struct Sources(pub(super) Arc<ServiceState>);
impl EnvironmentPinResolver for Sources {
    async fn image(&self, reference: &str) -> Result<String, EnvironmentResolutionError> {
        let admitted = images::pull_image(
            &self.0,
            capsem_api::ImagePullRequest {
                image: reference.into(),
                registry: None,
            },
        )
        .await
        .map_err(image_error)?;
        Ok(admitted.resolved.digest().to_string())
    }

    async fn repository(&self, repository: &str, reference: &str) -> Result<String, EnvironmentResolutionError> {
        repositories::resolve(&self.0, repository, reference).await
    }

    async fn catalog(
        &self,
        kind: CatalogKind,
        selection: &EnvironmentCatalogSelection,
    ) -> Result<EnvironmentCatalogPin, EnvironmentResolutionError> {
        catalogs::resolve(&self.0, kind, selection).await
    }

    async fn admit_catalog(
        &self,
        kind: CatalogKind,
        pin: &EnvironmentCatalogPin,
    ) -> Result<(), EnvironmentResolutionError> {
        catalogs::admit(&self.0, kind, pin).await
    }
}
fn image_error(error: AppError) -> EnvironmentResolutionError {
    match error.status {
        StatusCode::FORBIDDEN => EnvironmentResolutionError::Refused,
        StatusCode::BAD_REQUEST => EnvironmentResolutionError::InvalidIntent,
        _ => EnvironmentResolutionError::Unavailable,
    }
}
