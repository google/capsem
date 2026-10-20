use crate::client::Client;
use crate::{models, operations as api, Error, Registry, Result};

/// Registry catalog and prefetch through the service's admission policy.
pub struct Images<'a>(pub(crate) &'a Client);

impl Images<'_> {
    /// Catalog entries with compatible pins and current local disk observations.
    pub async fn list(&self, refresh: bool) -> Result<models::ImageListResponse> {
        api::list_images(
            &self.0.transport,
            &api::ListImagesParams { refresh: Some(refresh) },
            self.0.options,
        )
        .await
    }

    /// Prefetch an image without creating a VM. Registry access lasts one call.
    pub async fn pull(&self, image: &str, registry: Option<Registry>) -> Result<models::ImagePullResponse> {
        if image.is_empty() {
            return Err(Error::InvalidInput("image must be a nonempty string"));
        }
        api::pull_image(
            &self.0.transport,
            &api::PullImageParams {
                body: models::ImagePullRequest {
                    image: image.into(),
                    registry: registry.map(Into::into),
                },
            },
            self.0.options,
        )
        .await
    }
}

#[cfg(test)]
mod tests;
