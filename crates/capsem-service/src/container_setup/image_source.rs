use super::*;

/// Where images come from, and which may be fetched and run. Production
/// pulls from registries under the installation's policy; tests substitute.
pub(crate) trait ImageSource: Send + Sync {
    /// Read the local owner's cheap snapshot without fetching or verifying.
    fn cache_state(
        &self,
        _key: &capsem_assets::oci::CacheKey,
        _root: Option<&capsem_assets::oci::Digest>,
    ) -> anyhow::Result<Option<capsem_assets::oci::CacheState>> {
        Ok(None)
    }

    /// Schedule local owner proof; polling never waits for hashes or registry.
    fn observe_cache(&self, _keys: &[capsem_assets::oci::CacheKey], _parent: &StdPath) -> anyhow::Result<()> {
        Ok(())
    }

    fn pull(&self, image: String, access: RegistryAccess, parent: PathBuf, options: ImageFetchOptions) -> PullFuture;

    /// Read the catalog `source` names, anonymously: catalogs are public.
    fn fetch_catalog(&self, source: CatalogSource, parent: PathBuf) -> images::CatalogFuture;

    fn fetch_inventory(
        &self,
        _catalog: CatalogSource,
        _image: String,
        _artifact: String,
        _parent: PathBuf,
    ) -> images::inventory::InventoryFuture {
        Box::pin(async { anyhow::bail!("image source does not provide package inventory") })
    }

    /// Fetch a filesystem bound to an already admitted platform image.
    fn fetch_rootfs(
        &self,
        _reference: String,
        _subject: String,
        _access: RegistryAccess,
        _parent: PathBuf,
        _options: ImageFetchOptions,
        _cache_key: Option<capsem_assets::oci::CacheKey>,
    ) -> RootfsFuture {
        Box::pin(async { anyhow::bail!("image source does not provide published filesystems") })
    }

    /// The image policy for one decision, read fresh each time.
    fn policy(&self) -> PolicyFuture {
        Box::pin(async {
            tokio::task::spawn_blocking(|| {
                let (settings, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
                capsem_core::container::admission::ImagePolicy::from_files(&settings, &corp)
            })
            .await?
        })
    }
}
