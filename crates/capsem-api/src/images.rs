//! The image catalog as the service offers it, and pulls ahead of a create.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::RegistryAccess;

/// `GET /images`: the catalog entries the current image policy permits.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ImageListResponse {
    /// The catalog version listed; absent when the policy turns the catalog
    /// off or no catalog could ever be read, and then `images` is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<CatalogInfo>,
    /// Entries in name order.
    pub images: Vec<ImageInfo>,
}

/// One catalog version, identified by its manifest digest.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct CatalogInfo {
    /// The reference the catalog was read from, usually a moving channel tag.
    pub reference: String,
    /// The catalog version's manifest digest.
    pub digest: String,
    /// `stable` or `nightly`.
    pub channel: String,
    /// When the publishing workflow wrote this version, as it recorded it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
}

/// One catalog entry.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ImageInfo {
    /// The catalog name, as `capsem create --image` and `POST /images/pull` take it.
    pub name: String,
    pub description: String,
    /// Every architecture some version is built for (`arm64`, `amd64`).
    pub architectures: Vec<String>,
    /// The newest version this host can run, as `repository@digest`; absent
    /// when no version is built for this host's architecture and runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub cached: ImageCacheState,
}

/// The service cache owner's current observation of the selected image's
/// local bytes. This does not grant execution or describe registry freshness.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageCacheState {
    /// The owner has no current proof, including startup or invalidation.
    Unknown,
    /// No complete retained image receipt is present.
    Missing,
    /// Retained image metadata or bytes are incomplete or known invalid.
    Partial,
    /// The owner verified all required local bytes for the retained image.
    Ready,
}

/// `GET /images` query.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ImageListQuery {
    /// Read the catalog from its registry now instead of from the copy the
    /// service keeps for up to 30 minutes.
    #[serde(default)]
    pub refresh: bool,
}

/// `POST /images/pull`: fetch an image into the host cache ahead of a create.
/// `Debug` redacts registry access the way [`RegistryAccess`] does.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ImagePullRequest {
    /// A catalog name, `docker://IMAGE`, or a registry-qualified
    /// `registry/repository:tag` or `@sha256:` reference.
    pub image: String,
    /// Access to a private registry, used for this pull only and never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryAccess>,
}

/// A pulled and admitted image.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ImagePullResponse {
    /// The image as requested.
    pub image: String,
    /// What the request resolved to, as `repository@digest`: the pin a
    /// session records, never a tag.
    pub resolved: String,
    /// The platform manifest pulled for this host.
    pub digest: String,
}

#[cfg(test)]
mod tests;
