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
//! the explicit grants already make needs no catalog for admission. Filesystem
//! selection still reads it, so a grant cannot bypass a published EROFS binding.

use super::*;
use capsem_api::{
    CatalogInfo, ImageCacheState, ImageInfo, ImageListQuery, ImageListResponse, ImagePullRequest, ImagePullResponse,
};
use capsem_assets::oci::{is_catalog_name, Catalog, Digest, ImageReference, ResolvedImage, RUNTIME_CONTRACT};
use capsem_cache::{CacheMetrics, LoadStatus};
use capsem_core::container::admission::{CatalogSource, ImagePolicy};
use std::collections::{BTreeSet, HashMap, HashSet};
use tokio::time::{Duration, Instant};

/// How long a catalog read is reused before the next request reads it again.
pub(crate) const CATALOG_REFRESH: Duration = Duration::from_secs(capsem_proto::CACHE_IMAGE_CATALOG_REFRESH_SECONDS);
/// How long a failed read waits before the next request tries again, so an
/// offline host does not pay a registry timeout on every request.
pub(crate) const CATALOG_RETRY: Duration = Duration::from_secs(capsem_proto::CACHE_IMAGE_CATALOG_RETRY_SECONDS);

pub(crate) type CatalogFuture = Pin<Box<dyn Future<Output = anyhow::Result<(Digest, Catalog)>> + Send>>;

/// One catalog version as this service read it.
#[derive(Clone)]
pub(crate) struct LoadedCatalog {
    source: CatalogSource,
    digest: Digest,
    catalog: Arc<Catalog>,
}

impl LoadedCatalog {
    fn rootfs_bindings(&self, architecture: capsem_assets::asset_manager::PackageArchitecture) -> RootBindings<'_> {
        let mut bindings = HashMap::new();
        for version in self.catalog.entries().values().flat_map(|entry| entry.versions()) {
            if version.contract() <= RUNTIME_CONTRACT && version.platforms().contains(&architecture) {
                if let Some(root) = version.erofs(architecture) {
                    bindings
                        .entry(version.image().digest())
                        .or_insert_with(HashSet::new)
                        .insert(root);
                }
            }
        }
        bindings
    }

    fn rootfs_for(
        &self,
        image: &Digest,
        architecture: capsem_assets::asset_manager::PackageArchitecture,
    ) -> Result<Option<&Digest>, ImageError> {
        root_binding(&self.rootfs_bindings(architecture), image)
    }
}

type RootBindings<'a> = HashMap<&'a Digest, HashSet<&'a Digest>>;

fn root_binding<'a>(bindings: &RootBindings<'a>, image: &Digest) -> Result<Option<&'a Digest>, ImageError> {
    let Some(roots) = bindings.get(image) else {
        return Ok(None);
    };
    if roots.len() > 1 {
        return Err(ImageError::Failed(
            "catalog binds the image to conflicting published filesystems".into(),
        ));
    }
    Ok(roots.iter().next().copied())
}

#[derive(Default)]
struct CatalogSlot {
    good: Option<(LoadedCatalog, Instant)>,
    failed: Option<(CatalogSource, Instant)>,
}

/// The last good catalog. The async lock makes concurrent requests share one
/// read instead of each asking the registry.
pub(crate) struct CatalogCache {
    slot: tokio::sync::Mutex<CatalogSlot>,
    metrics: CacheMetrics,
}

impl Default for CatalogCache {
    fn default() -> Self {
        Self {
            slot: tokio::sync::Mutex::new(CatalogSlot::default()),
            metrics: CacheMetrics::new("service.image_catalog"),
        }
    }
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
                    self.metrics.hit();
                    return Some(loaded.clone());
                }
            }
            if matches!(&slot.failed, Some((failed, at)) if failed == source && now.duration_since(*at) < CATALOG_RETRY)
            {
                if last.is_some() {
                    self.metrics.hit();
                } else {
                    self.metrics.miss();
                }
                return last.map(|(loaded, _)| loaded);
            }
        }
        self.metrics.miss();
        let load_started = Instant::now();
        let read = match images.fetch_catalog(source.clone(), parent.to_owned()).await {
            Ok((digest, catalog)) => {
                info!(catalog = %source.reference, digest = %digest, "read the image catalog");
                let loaded = LoadedCatalog {
                    source: source.clone(),
                    digest,
                    catalog: Arc::new(catalog),
                };
                let refresh = slot.good.is_some();
                slot.good = Some((loaded.clone(), now));
                slot.failed = None;
                self.metrics.load(load_started.elapsed(), LoadStatus::Success);
                if refresh {
                    self.metrics.refresh();
                } else {
                    self.metrics.insertion();
                }
                self.metrics.set_entries(1);
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
                self.metrics.load(load_started.elapsed(), LoadStatus::Error);
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
        AppError::new(status, error.to_string())
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
    pub(super) async fn app(&mut self, resolved: &ResolvedImage) -> Option<api::ImageApp> {
        let architecture = stage::catalog_architecture().ok()?;
        let catalog = self.load(false).await?;
        let mut matches = catalog.catalog.entries().iter().flat_map(|(name, entry)| {
            entry
                .versions()
                .iter()
                .filter(move |version| {
                    version.image() == resolved
                        && version.contract() <= RUNTIME_CONTRACT
                        && version.platforms().contains(&architecture)
                })
                .map(move |version| app_metadata(name, entry, version, architecture))
        });
        let app = matches.next()?;
        matches.next().is_none().then_some(app)
    }

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
    /// Admission grants do not bypass a catalog's filesystem binding.
    pub(crate) async fn published_root(
        &mut self,
        requested: &Requested,
        image: &PulledImage,
        resolved: &ResolvedImage,
        access: RegistryAccess,
        options: ImageFetchOptions,
    ) -> Result<Option<capsem_assets::oci::RootfsLayout>, ImageError> {
        let Some(loaded) = self.load(false).await else {
            return Ok(None);
        };
        let architecture = stage::catalog_architecture().map_err(|e| ImageError::Failed(format!("{e:#}")))?;
        let Some(digest) = loaded.rootfs_for(resolved.digest(), architecture)? else {
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
                options,
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
fn app_metadata(
    name: &str,
    entry: &capsem_assets::oci::CatalogEntry,
    version: &capsem_assets::oci::CatalogVersion,
    architecture: capsem_assets::asset_manager::PackageArchitecture,
) -> api::ImageApp {
    let mut evidence = Vec::new();
    if let Some(digest) = version.obom(architecture) {
        evidence.push(api::ImageEvidence {
            kind: api::ImageEvidenceKind::Obom,
            reference: format!("{}@{digest}", version.image().repository()),
            media_type: "application/vnd.cyclonedx+json".into(),
        });
    }
    evidence.extend(version.evidence(architecture).iter().filter_map(|entry| {
        let kind = match entry.kind.as_str() {
            "sbom" => api::ImageEvidenceKind::Sbom,
            "abom" => api::ImageEvidenceKind::Abom,
            "package_inventory" => api::ImageEvidenceKind::PackageInventory,
            _ => return None,
        };
        Some(api::ImageEvidence {
            kind,
            reference: entry.reference.to_string(),
            media_type: entry.media_type.clone(),
        })
    }));
    let interfaces = if entry.interfaces().is_empty() {
        match entry.surface() {
            Some("xpra") => vec![api::ImageInterface::Graphical],
            Some("terminal") => vec![api::ImageInterface::Terminal],
            _ => vec![],
        }
    } else {
        entry
            .interfaces()
            .iter()
            .filter_map(|kind| match kind.as_str() {
                "terminal" => Some(api::ImageInterface::Terminal),
                "graphical" => Some(api::ImageInterface::Graphical),
                "web" => Some(api::ImageInterface::Web),
                "api" => Some(api::ImageInterface::Api),
                _ => None,
            })
            .collect()
    };
    api::ImageApp {
        catalog_name: name.into(),
        image: version.image().to_string(),
        display_name: entry.display_name().unwrap_or(name).into(),
        description: entry.description().into(),
        architecture: architecture.as_str().into(),
        publisher: entry.publisher().map(str::to_owned),
        url: entry.url().map(str::to_owned),
        icon: entry.icon().map(str::to_owned),
        category: entry.category().map(str::to_owned),
        interfaces,
        evidence,
    }
}

pub(crate) async fn handle_list_images(
    State(state): State<Arc<ServiceState>>,
    Query(query): Query<ImageListQuery>,
) -> Result<Json<ImageListResponse>, AppError> {
    let mut decision = Decision::new(&state).await?;
    let Some(loaded) = decision.load(query.refresh).await.cloned() else {
        if let Err(error) = decision.setups.source.observe_cache(&[], &decision.parent) {
            warn!(%error, "cache verification cancellation remains pending");
        }
        if let Some(source) = decision.policy.catalog_source() {
            return Err(ImageError::Failed(format!(
                "Image catalog {} is unavailable. Check registry access and publication, then refresh the catalog.",
                source.reference
            ))
            .into());
        }
        return Ok(Json(ImageListResponse {
            catalog: None,
            images: Vec::new(),
        }));
    };
    let architecture = stage::catalog_architecture()
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let mut images: Vec<ImageInfo> = loaded
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
            previous_image: None,
            app: None,
            progress: None,
            surface: match entry.surface() {
                Some("terminal") => Some(capsem_api::ImageSurface::Terminal),
                Some("xpra") => Some(capsem_api::ImageSurface::Xpra),
                _ => None,
            },
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
    let filesystems = loaded.rootfs_bindings(architecture);
    for (image, key) in images
        .iter_mut()
        .filter(|image| image.image.is_some())
        .zip(keys.clone())
    {
        image.progress = image
            .image
            .as_deref()
            .and_then(|image| decision.setups.downloads.snapshot(image));
        use capsem_assets::oci::CacheState;
        let resolved = loaded
            .catalog
            .resolve(&image.name, architecture, RUNTIME_CONTRACT)
            .map_err(|error| ImageError::Failed(format!("catalog image identity: {error:#}")))?;
        let entry = loaded.catalog.entry(&image.name).expect("listed catalog entry");
        let version = entry
            .versions()
            .iter()
            .find(|version| version.image() == &resolved)
            .expect("selected catalog version");
        image.app = Some(app_metadata(&image.name, entry, version, architecture));
        let required_root = root_binding(&filesystems, resolved.digest())?;
        image.cached = match decision.setups.source.cache_state(&key, required_root) {
            Ok(Some(CacheState::Missing)) => ImageCacheState::Missing,
            Ok(Some(CacheState::Partial)) => ImageCacheState::Partial,
            Ok(Some(CacheState::Ready)) => ImageCacheState::Ready,
            Ok(Some(CacheState::Unknown) | None) => ImageCacheState::Unknown,
            Err(error) => {
                warn!(image = %image.name, %error, "image disk observation unavailable");
                ImageCacheState::Unknown
            }
        };
        if image.cached != ImageCacheState::Ready {
            let entry = loaded.catalog.entry(&image.name).expect("listed catalog entry");
            let selected = entry
                .versions()
                .iter()
                .rposition(|version| version.image() == &resolved)
                .expect("resolved catalog version");
            for version in entry.versions()[..selected].iter().rev().filter(|version| {
                version.contract() <= RUNTIME_CONTRACT
                    && version.platforms().contains(&architecture)
                    && version.image() != &resolved
            }) {
                let previous = version.image().to_string();
                let previous_key =
                    capsem_assets::oci::CacheIdentity::new(&previous, architecture.as_str(), RUNTIME_CONTRACT)
                        .map_err(|error| ImageError::Failed(format!("cache identity: {error:#}")))?
                        .key();
                // Observation remains owned and asynchronous. A missing selected
                // version alone says nothing about whether this is first use.
                if matches!(image.cached, ImageCacheState::Missing | ImageCacheState::Partial)
                    && image.previous_image.is_none()
                    && decision
                        .setups
                        .source
                        .cache_state(&previous_key, root_binding(&filesystems, version.image().digest())?)
                        .is_ok_and(|state| state == Some(CacheState::Ready))
                {
                    image.previous_image = Some(previous);
                }
                keys.push(previous_key);
            }
        }
    }
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
    let budget = capsem_api::IMAGE_PULL_TIMEOUT_SECS;
    tokio::time::timeout(Duration::from_secs(budget), pull_image(&state, request))
        .await
        .map_err(|_| {
            AppError::new(
                StatusCode::GATEWAY_TIMEOUT,
                format!("image pull exceeded {budget} seconds"),
            )
            .with_timeout_secs(budget)
        })?
}

async fn pull_image(state: &ServiceState, request: ImagePullRequest) -> Result<Json<ImagePullResponse>, AppError> {
    let mut decision = Decision::new(state).await?;
    let requested = decision.resolve(&request.image).await?;
    decision.check_source(&requested).await?;
    let download = state.containers.downloads.begin(&requested.pull);
    let access = request.registry.unwrap_or_default();
    let image = state
        .containers
        .source
        .pull(
            requested.pull.clone(),
            access.clone(),
            decision.parent.clone(),
            ImageFetch::Fresh.with_progress(download.observer(false)),
        )
        .await
        .map_err(|e| ImageError::Failed(format!("pull {}: {e:#}", requested.pull)))?;
    let resolved = decision.admit(&requested, &image).await?;
    decision
        .published_root(
            &requested,
            &image,
            &resolved,
            access,
            ImageFetch::Fresh.with_progress(download.observer(true)),
        )
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
