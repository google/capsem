//! Request registry transports borrow persistent credential-free cache owners.
use super::*;

// Bound queue metadata independently of the cache's content capacity.
const CACHE_VERIFICATION_BATCH_KEYS: usize = 1024;

#[derive(Clone, Default)]
pub(crate) struct RegistryImages {
    cache: Arc<std::sync::OnceLock<capsem_assets::oci::ImageCache>>,
    worker: Arc<std::sync::OnceLock<(PathBuf, capsem_assets::oci::CacheReconciler)>>,
}

impl RegistryImages {
    pub(super) fn cache(&self) -> anyhow::Result<capsem_assets::oci::ImageCache> {
        if self.cache.get().is_none() {
            // Concurrent initializers use the same installed contract. Only
            // the winner owns the service lifetime; losers borrow that owner.
            let _ = self.cache.set(capsem_assets::oci::ImageCache::installed()?);
        }
        self.cache
            .get()
            .cloned()
            .context("image cache owner initialization failed")
    }

    pub(super) fn registry_puller(&self, access: RegistryAccess) -> anyhow::Result<capsem_assets::oci::Puller> {
        let authentication = match (access.username, access.password) {
            (Some(username), Some(password)) => capsem_assets::oci::RegistryAuth::Basic(username, password),
            (None, None) => capsem_assets::oci::RegistryAuth::Anonymous,
            _ => anyhow::bail!("registry access needs both username and password"),
        };
        Ok(capsem_assets::oci::Puller::new_with_root_certificate(
            stage::oci_architecture()?,
            authentication,
            access.ca_pem.as_deref().map(str::as_bytes),
        )?
        .with_cache(&self.cache()?))
    }
}

impl ImageSource for RegistryImages {
    fn observe_cache(&self, keys: &[capsem_assets::oci::CacheKey], parent: &StdPath) -> anyhow::Result<()> {
        anyhow::ensure!(
            keys.len() <= CACHE_VERIFICATION_BATCH_KEYS,
            "catalog cache observation exceeds capacity"
        );
        if self.worker.get().is_none() {
            if keys.is_empty() {
                return Ok(());
            }
            let worker = self.cache()?.reconciler(
                stage::oci_architecture()?,
                parent.to_owned(),
                CACHE_VERIFICATION_BATCH_KEYS,
            )?;
            let _ = self.worker.set((parent.to_owned(), worker));
        }
        let (bound, worker) = self
            .worker
            .get()
            .context("cache verification worker initialization failed")?;
        anyhow::ensure!(bound == parent, "cache verification staging owner changed");
        worker.request(keys)?;
        Ok(())
    }

    fn pull(&self, image: String, access: RegistryAccess, parent: PathBuf, mode: ImageFetch) -> PullFuture {
        let source = self.clone();
        Box::pin(async move {
            capsem_assets::oci::image_reference(&image)
                .context("container image expects docker://IMAGE or registry/repository:tag")?;
            let puller = source.registry_puller(access)?;
            let layout = match mode {
                ImageFetch::Fresh => puller.pull(&image, &parent).await?,
                ImageFetch::PreferCached => puller.pull_prefer_cached(&image, &parent).await?,
            };
            Ok(PulledImage {
                root: layout.path().to_path_buf(),
                files: layout.files().to_vec(),
                digest: layout.source_digest.clone(),
                image_digest: layout.image_digest.clone(),
                cache_key: Some(layout.cache_identity().key()),
                _hold: Box::new(layout),
            })
        })
    }

    fn fetch_catalog(&self, source: CatalogSource, parent: PathBuf) -> images::CatalogFuture {
        let images = self.clone();
        Box::pin(async move {
            let ca = match &source.ca {
                Some(path) => Some(
                    tokio::fs::read(path)
                        .await
                        .with_context(|| format!("read [images] catalog_ca {}", path.display()))?,
                ),
                None => None,
            };
            let puller = capsem_assets::oci::Puller::new_with_root_certificate(
                stage::oci_architecture()?,
                capsem_assets::oci::RegistryAuth::Anonymous,
                ca.as_deref(),
            )?
            .with_cache(&images.cache()?);
            puller.fetch_catalog(&source.reference, &parent).await
        })
    }

    fn fetch_rootfs(
        &self,
        reference: String,
        subject: String,
        access: RegistryAccess,
        parent: PathBuf,
        mode: ImageFetch,
        cache_key: Option<capsem_assets::oci::CacheKey>,
    ) -> RootfsFuture {
        let source = self.clone();
        Box::pin(async move {
            let subject = capsem_assets::oci::Digest::parse(&subject)?;
            let puller = source.registry_puller(access)?;
            let root = match mode {
                ImageFetch::Fresh => puller.fetch_rootfs(&reference, &subject, &parent).await?,
                ImageFetch::PreferCached => puller.fetch_rootfs_prefer_cached(&reference, &subject, &parent).await?,
            };
            if let Some(key) = cache_key {
                puller.retain_cached_root(&key, &root).await?;
            }
            Ok(root)
        })
    }
}
