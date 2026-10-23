//! Bounded, verified package metadata; never fetch or unpack application layers.
use super::*;
use serde::{Deserialize, Serialize};

pub const INVENTORY_MEDIA_TYPE: &str = "application/vnd.capsem.package-inventory.v1+json";
const LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryPackage {
    pub name: String,
    pub version: String,
    pub architecture: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryTool {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageInventory {
    pub schema: String,
    pub subject: String,
    pub architecture: String,
    pub packages: Vec<InventoryPackage>,
    pub tools: Vec<InventoryTool>,
}

impl PackageInventory {
    fn validate(&self, subject: &str, architecture: &str) -> Result<()> {
        ensure!(
            self.schema == "capsem.package_inventory.v1",
            "unsupported package inventory schema"
        );
        ensure!(self.subject == subject, "inventory document subject mismatch");
        ensure!(
            self.architecture == architecture,
            "inventory document platform mismatch"
        );
        ensure!(
            !self.packages.is_empty() && self.packages.len() <= 50000 && self.tools.len() <= 4096,
            "inventory row count outside limits"
        );
        for (name, version) in self
            .packages
            .iter()
            .map(|p| (&p.name, &p.version))
            .chain(self.tools.iter().map(|p| (&p.name, &p.version)))
        {
            ensure!(
                !name.is_empty()
                    && name.len() <= 256
                    && !version.is_empty()
                    && version.len() <= 1024
                    && !name.chars().chain(version.chars()).any(char::is_control),
                "invalid inventory name or version"
            );
        }
        ensure!(
            self.packages
                .iter()
                .all(|p| matches!(p.architecture.as_str(), "arm64" | "amd64" | "all")),
            "unsupported package architecture"
        );
        Ok(())
    }
}

impl Puller {
    /// Read only catalog-authorized, pinned evidence in the selected image repository.
    /// The immutable image/index and artifact must both be supplied by the catalog owner.
    pub async fn fetch_inventory(&self, image: &str, artifact: &str, parent: &Path) -> Result<PackageInventory> {
        tokio::time::timeout(Duration::from_secs(30), self.inventory_inner(image, artifact, parent))
            .await
            .context("inventory read exceeded 30 seconds")?
    }

    async fn inventory_inner(&self, image: &str, artifact: &str, parent: &Path) -> Result<PackageInventory> {
        let image = image_reference(image)?;
        let artifact = image_reference(artifact)?;
        ensure!(
            image.digest().is_some() && artifact.digest().is_some(),
            "inventory requires immutable image and artifact"
        );
        ensure!(
            image.registry() == artifact.registry() && image.repository() == artifact.repository(),
            "inventory artifact must share the selected image repository"
        );
        if let Some(cache) = &self.cache {
            cache.prepare().await?;
        }
        let token = self
            .registry
            .auth(&image, &self.authentication, RegistryOperation::Pull)
            .await?;
        let first = self.manifest(&image, token.as_deref()).await?;
        let bytes = self.platform_manifest(&image, &first, token.as_deref(), false).await?;
        let platform_digest = sha256(&bytes);
        let platform_size = bytes.len() as i64;
        let mut image_manifest: OciImageManifest = serde_json::from_slice(&bytes)?;
        validate_manifest(&mut image_manifest)?;
        let directory = staging(parent).await?;
        self.blob(&image, &image_manifest.config, directory.path(), BlobUse::Private, None)
            .await?;
        let config = tokio::fs::read(directory.path().join(digest_hex(&image_manifest.config.digest)?)).await?;
        verify_platform(&config, &self.architecture)?;

        let bytes = self.manifest(&artifact, token.as_deref()).await?;
        let manifest: OciImageManifest = serde_json::from_slice(&bytes)?;
        ensure!(
            manifest.schema_version == 2
                && manifest.media_type.as_deref() == Some(OCI_IMAGE_MEDIA_TYPE)
                && manifest.artifact_type.as_deref() == Some(INVENTORY_MEDIA_TYPE),
            "unsupported inventory manifest media type"
        );
        let subject = manifest.subject.context("inventory artifact has no subject")?;
        ensure!(
            subject.digest == platform_digest
                && subject.size == platform_size
                && matches!(
                    subject.media_type.as_str(),
                    OCI_IMAGE_MEDIA_TYPE | IMAGE_MANIFEST_MEDIA_TYPE
                )
                && subject.urls.as_ref().is_none_or(Vec::is_empty),
            "inventory artifact subject mismatch"
        );
        ensure!(
            manifest.config.media_type == "application/vnd.oci.empty.v1+json"
                && manifest.config.size == 2
                && manifest.config.digest == sha256(b"{}")
                && manifest.config.urls.as_ref().is_none_or(Vec::is_empty),
            "invalid inventory empty config"
        );
        let [layer] = manifest.layers.as_slice() else {
            anyhow::bail!("inventory requires exactly one JSON layer");
        };
        ensure!(
            layer.media_type == INVENTORY_MEDIA_TYPE
                && layer.size > 0
                && layer.size as u64 <= LIMIT
                && layer.urls.as_ref().is_none_or(Vec::is_empty),
            "invalid inventory layer media, size or external URLs"
        );
        self.blob(&artifact, layer, directory.path(), BlobUse::Private, None)
            .await?;
        let bytes = tokio::fs::read(directory.path().join(digest_hex(&layer.digest)?)).await?;
        let inventory: PackageInventory = serde_json::from_slice(&bytes)?;
        let subject = Reference::with_digest(image.registry().into(), image.repository().into(), platform_digest);
        inventory.validate(&subject.whole(), &self.architecture)?;
        Ok(inventory)
    }
}
