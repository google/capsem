//! The image catalog in the service, the one resolver every image request
//! goes through, and the `/images` routes.
//!
//! **Names.** A request that has a catalog name's shape (`codex-cli`, see
//! [`is_catalog_name`]) is looked up in the catalog first and, when listed,
//! becomes the `repository@digest` of the newest version this host's
//! architecture and runtime contract can run. Everything else is parsed as a
//! reference. A catalog name can look like a Docker Hub short name (`redis`),
//! and catalog names win; a bare name that is not in the catalog is refused,
//! never sent to Docker Hub: a Hub image is `docker://redis`, and the default
//! policy refuses that source anyway.
//!
//! **Catalog.** The service keeps the last catalog it read in memory and
//! reads it again at most every [`CATALOG_REFRESH`], or on demand
//! (`GET /images?refresh=true`). A failed read keeps the last good catalog and
//! is not retried for [`CATALOG_RETRY`]; with none ever read, the catalog is
//! empty, so only the explicit `[images]` grants admit anything. A decision
//! the explicit grants already make never reads the catalog at all.

use super::*;
use capsem_api::{
    CatalogInfo, ImageCacheState, ImageInfo, ImageListQuery, ImageListResponse, ImagePullRequest, ImagePullResponse,
};
use capsem_assets::oci::{is_catalog_name, Catalog, Digest, ImageReference, ResolvedImage, RUNTIME_CONTRACT};
use capsem_core::container::admission::{CatalogSource, ImagePolicy};
use std::collections::BTreeSet;
use tokio::time::{Duration, Instant};

/// How long a catalog read is reused before the next request reads it again.
pub(crate) const CATALOG_REFRESH: Duration = Duration::from_secs(30 * 60);
/// How long a failed read waits before the next request tries again, so an
/// offline host does not pay a registry timeout on every request.
pub(crate) const CATALOG_RETRY: Duration = Duration::from_secs(60);

pub(crate) type CatalogFuture = Pin<Box<dyn Future<Output = anyhow::Result<(Digest, Catalog)>> + Send>>;

/// One catalog version as this service read it.
#[derive(Clone)]
pub(crate) struct LoadedCatalog {
    source: CatalogSource,
    digest: Digest,
    catalog: Arc<Catalog>,
}

#[derive(Default)]
struct CatalogSlot {
    good: Option<(LoadedCatalog, Instant)>,
    failed: Option<(CatalogSource, Instant)>,
}

/// The last good catalog. The async lock makes concurrent requests share one
/// read instead of each asking the registry.
#[derive(Default)]
pub(crate) struct CatalogCache {
    slot: tokio::sync::Mutex<CatalogSlot>,
}

impl CatalogCache {
    /// The catalog `source` names. A last good catalog read from another
    /// source is never returned: an administrator who moved the catalog, to
    /// a mirror or a narrower one, gets that catalog or none.
    async fn get(
        &self,
        images: &dyn ImageSource,
        source: &CatalogSource,
        parent: &StdPath,
        refresh: bool,
    ) -> Option<LoadedCatalog> {
        let mut slot = self.slot.lock().await;
        let now = Instant::now();
        let last = slot
            .good
            .as_ref()
            .filter(|(loaded, _)| loaded.source == *source)
            .map(|(loaded, read)| (loaded.clone(), *read));
        if !refresh {
            if let Some((loaded, read)) = &last {
                if now.duration_since(*read) < CATALOG_REFRESH {
                    return Some(loaded.clone());
                }
            }
            if matches!(&slot.failed, Some((failed, at)) if failed == source && now.duration_since(*at) < CATALOG_RETRY)
            {
                return last.map(|(loaded, _)| loaded);
            }
        }
        let read = match images.fetch_catalog(source.clone(), parent.to_owned()).await {
            Ok((digest, catalog)) => {
                info!(catalog = %source.reference, digest = %digest, "read the image catalog");
                let loaded = LoadedCatalog {
                    source: source.clone(),
                    digest,
                    catalog: Arc::new(catalog),
                };
                slot.good = Some((loaded.clone(), now));
                slot.failed = None;
                Some(loaded)
            }
            Err(error) => {
                warn!(
                    catalog = %source.reference,
                    error = %format!("{error:#}"),
                    kept = last.is_some(),
                    "image catalog read failed; keeping the last good catalog"
                );
                slot.failed = Some((source.clone(), now));
                last.map(|(loaded, _)| loaded)
            }
        };
        // Held across the read on purpose: concurrent requests share it.
        drop(slot);
        read
    }
}

/// Why an image request did not go through, and the HTTP status that says so.
#[derive(Debug)]
pub(crate) enum ImageError {
    /// The request names no image this service can resolve.
    Invalid(String),
    /// The image policy refuses it.
    Refused(String),
    /// The registry or the host failed.
    Failed(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::Refused(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

impl From<ImageError> for String {
    fn from(error: ImageError) -> Self {
        error.to_string()
    }
}

impl From<ImageError> for AppError {
    fn from(error: ImageError) -> Self {
        let status = match &error {
            ImageError::Invalid(_) => StatusCode::BAD_REQUEST,
            ImageError::Refused(_) => StatusCode::FORBIDDEN,
            ImageError::Failed(_) => StatusCode::BAD_GATEWAY,
        };
        AppError(status, error.to_string())
    }
}

/// An image request resolved to what will be pulled.
pub(crate) struct Requested {
    /// The reference to pull: the request itself, or the `repository@digest`
    /// a catalog name resolved to.
    pub(crate) pull: String,
    /// The registry the pull contacts.
    pub(crate) registry: String,
    /// The normalized identity the policy decides on.
    pub(crate) identity: ImageReference,
}

/// One image decision: the policy read from the files, with the catalog
/// joined only once something the explicit grants do not decide needs it.
pub(crate) struct Decision<'a> {
    setups: &'a ContainerSetups,
    /// Private staging for registry reads, shared by pulls and catalog reads.
    pub(crate) parent: PathBuf,
    policy: ImagePolicy,
    /// Whether this decision asked for the catalog yet; `catalog` is what it got.
    asked: bool,
    catalog: Option<LoadedCatalog>,
}

impl<'a> Decision<'a> {
    pub(crate) async fn new(state: &'a ServiceState) -> Result<Self, ImageError> {
        let setups = &state.containers;
        let policy = setups
            .source
            .policy()
            .await
            .map_err(|e| ImageError::Failed(format!("image policy: {e:#}")))?;
        let parent = state.run_dir.join("container-pulls");
        tokio::task::spawn_blocking({
            let parent = parent.clone();
            move || capsem_foundation::unix::fs::ensure_private_dir(&parent)
        })
        .await
        .map_err(|e| ImageError::Failed(format!("prepare pull directory: {e}")))?
        .map_err(|e| ImageError::Failed(format!("prepare pull directory: {e}")))?;
        Ok(Self {
            setups,
            parent,
            policy,
            asked: false,
            catalog: None,
        })
    }

    /// The catalog the policy names, read once per decision (from the
    /// service's copy unless `refresh`) and joined to the policy.
    async fn load(&mut self, refresh: bool) -> Option<&LoadedCatalog> {
        if refresh || !self.asked {
            let loaded = match self.policy.catalog_source() {
                Some(source) => {
                    self.setups
                        .catalog
                        .get(&*self.setups.source, source, &self.parent, refresh)
                        .await
                }
                None => None,
            };
            if let Some(loaded) = &loaded {
                self.policy = std::mem::take(&mut self.policy).with_catalog(loaded.catalog.supported().cloned());
            }
            self.asked = true;
            self.catalog = loaded;
        }
        self.catalog.as_ref()
    }

    /// Resolve `image` -- a catalog name or a reference -- to what to pull.
    pub(crate) async fn resolve(&mut self, image: &str) -> Result<Requested, ImageError> {
        if is_catalog_name(image) {
            if let Some(loaded) = self.load(false).await {
                if loaded.catalog.entry(image).is_some() {
                    let architecture =
                        stage::catalog_architecture().map_err(|e| ImageError::Invalid(format!("{e:#}")))?;
                    let pinned = loaded
                        .catalog
                        .resolve(image, architecture, RUNTIME_CONTRACT)
                        .map_err(|e| ImageError::Invalid(format!("{e:#}")))?;
                    return requested(&pinned.to_string());
                }
            }
            // A name has no registry, so it is never a reference either.
            let why = match (self.policy.catalog_source(), &self.catalog) {
                (None, _) => "the image catalog is turned off by [images] catalog",
                (Some(_), None) => "no image catalog could be read",
                (Some(_), Some(_)) => "it is not in the image catalog",
            };
            return requested(image).map_err(|_| {
                ImageError::Invalid(format!(
                    "{image:?} names no image: {why}; name an image as registry/repository:tag or docker://IMAGE"
                ))
            });
        }
        requested(image)
    }

    /// May bytes for `requested` be fetched? The explicit grants answer
    /// first; the catalog is read only when they refuse.
    pub(crate) async fn check_source(&mut self, requested: &Requested) -> Result<(), ImageError> {
        if !self.policy.grants_source(&requested.identity) {
            self.load(false).await;
        }
        self.policy
            .check_source(&requested.identity)
            .map_err(|e| ImageError::Refused(format!("{e:#}")))
    }

    /// May the pulled image run? Answers with the identity it resolved to.
    pub(crate) async fn admit(
        &mut self,
        requested: &Requested,
        image: &PulledImage,
    ) -> Result<ResolvedImage, ImageError> {
        if let Ok(resolved) = admit(&self.policy, &requested.identity, image) {
            return Ok(resolved);
        }
        self.load(false).await;
        admit(&self.policy, &requested.identity, image).map_err(ImageError::Refused)
    }

    /// A compatible filesystem for this admitted image, fetched only from
    /// the same repository and with its original platform manifest as subject.
    /// Explicit grants that never loaded a catalog stay on the universal path.
    pub(crate) async fn published_root(
        &self,
        requested: &Requested,
        image: &PulledImage,
        resolved: &ResolvedImage,
        access: RegistryAccess,
        mode: ImageFetch,
    ) -> Result<Option<capsem_assets::oci::RootfsLayout>, ImageError> {
        let Some(loaded) = &self.catalog else {
            return Ok(None);
        };
        let architecture = stage::catalog_architecture().map_err(|e| ImageError::Failed(format!("{e:#}")))?;
        let roots: BTreeSet<_> = loaded
            .catalog
            .entries()
            .values()
            .flat_map(|entry| entry.versions())
            .filter(|version| {
                version.image().digest() == resolved.digest()
                    && version.contract() <= RUNTIME_CONTRACT
                    && version.platforms().contains(&architecture)
            })
            .filter_map(|version| version.erofs(architecture).map(ToString::to_string))
            .collect();
        if roots.len() > 1 {
            return Err(ImageError::Failed(
                "catalog binds the image to conflicting published filesystems".into(),
            ));
        }
        let Some(digest) = roots.first() else {
            return Ok(None);
        };
        let reference = format!("{}@{digest}", requested.identity.repository());
        let artifact = super::images::requested(&reference)?;
        self.policy
            .check_source(&artifact.identity)
            .map_err(|e| ImageError::Refused(format!("{e:#}")))?;
        self.setups
            .source
            .fetch_rootfs(
                reference.clone(),
                image.digest.clone(),
                access,
                self.parent.clone(),
                mode,
                image.cache_key.clone(),
            )
            .await
            .map(Some)
            .map_err(|e| ImageError::Failed(format!("fetch published filesystem {reference}: {e:#}")))
    }
}

fn requested(image: &str) -> Result<Requested, ImageError> {
    let invalid = |e: anyhow::Error| {
        ImageError::Invalid(format!(
            "container image expects docker://IMAGE or registry/repository:tag: {e:#}"
        ))
    };
    let reference = capsem_assets::oci::image_reference(image).map_err(invalid)?;
    let identity = ImageReference::try_from(&reference).map_err(invalid)?;
    Ok(Requested {
        pull: image.to_owned(),
        registry: reference.resolve_registry().to_owned(),
        identity,
    })
}

/// `GET /images` -- the catalog entries the current policy permits. Every
/// catalog digest is admitted by construction, so the policy decides per
/// catalog: one that turns the catalog off lists nothing and reads nothing.
pub(crate) async fn handle_list_images(
    State(state): State<Arc<ServiceState>>,
    Query(query): Query<ImageListQuery>,
) -> Result<Json<ImageListResponse>, AppError> {
    let mut decision = Decision::new(&state).await?;
    let Some(loaded) = decision.load(query.refresh).await.cloned() else {
        if let Err(error) = decision.setups.source.observe_cache(&[], &decision.parent) {
            warn!(%error, "cache verification cancellation remains pending");
        }
        return Ok(Json(ImageListResponse {
            catalog: None,
            images: Vec::new(),
        }));
    };
    let architecture =
        stage::catalog_architecture().map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let images: Vec<ImageInfo> = loaded
        .catalog
        .entries()
        .iter()
        .map(|(name, entry)| ImageInfo {
            name: name.clone(),
            description: entry.description().to_owned(),
            architectures: entry
                .versions()
                .iter()
                .flat_map(|version| version.platforms().iter().map(|platform| platform.as_str()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            image: loaded
                .catalog
                .resolve(name, architecture, RUNTIME_CONTRACT)
                .ok()
                .map(|image| image.to_string()),
            cached: ImageCacheState::Unknown,
        })
        .collect();
    let mut keys = images
        .iter()
        .filter_map(|image| image.image.as_deref())
        .map(|image| {
            capsem_assets::oci::CacheIdentity::new(image, architecture.as_str(), RUNTIME_CONTRACT)
                .map(|identity| identity.key())
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(|error| ImageError::Failed(format!("cache identity: {error:#}")))?;
    keys.extend(
        state
            .containers
            .active_cache_owners(&state)
            .into_iter()
            .map(|owner| owner.key),
    );
    keys.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
    keys.dedup();
    if let Err(error) = decision.setups.source.observe_cache(&keys, &decision.parent) {
        warn!(%error, "catalog cache verification remains pending");
    }
    Ok(Json(ImageListResponse {
        catalog: Some(CatalogInfo {
            reference: loaded.source.reference.clone(),
            digest: loaded.digest.to_string(),
            channel: loaded.catalog.channel().to_string(),
            generated_at: loaded.catalog.generated_at().map(str::to_owned),
        }),
        images,
    }))
}

/// `POST /images/pull` -- resolve, check the source, pull into the host's
/// image cache, admit. A refused source answers 403 before any registry is
/// contacted.
pub(crate) async fn handle_pull_image(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<ImagePullRequest>,
) -> Result<Json<ImagePullResponse>, AppError> {
    let mut decision = Decision::new(&state).await?;
    let requested = decision.resolve(&request.image).await?;
    decision.check_source(&requested).await?;
    let access = request.registry.unwrap_or_default();
    let image = state
        .containers
        .source
        .pull(
            requested.pull.clone(),
            access.clone(),
            decision.parent.clone(),
            ImageFetch::Fresh,
        )
        .await
        .map_err(|e| ImageError::Failed(format!("pull {}: {e:#}", requested.pull)))?;
    let resolved = decision.admit(&requested, &image).await?;
    decision
        .published_root(&requested, &image, &resolved, access, ImageFetch::Fresh)
        .await?;
    info!(image = %request.image, resolved = %resolved, "pulled image into the host cache");
    Ok(Json(ImagePullResponse {
        image: request.image,
        resolved: resolved.to_string(),
        digest: image.digest.clone(),
    }))
}

#[cfg(test)]
mod tests;
