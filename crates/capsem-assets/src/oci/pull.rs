use std::{
    collections::BTreeMap,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{ensure, Context, Result};
use futures::{stream, StreamExt, TryStreamExt};
use oci_client::{
    client::{Certificate, CertificateEncoding, ClientConfig, ClientProtocol},
    manifest::*,
    secrets::RegistryAuth,
    Client, Reference, RegistryOperation,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::{
    cache::BlobCache,
    catalog::{Catalog, CATALOG_MEDIA_TYPE},
    digest_hex, image_reference,
    selector::Digest as ContentDigest,
    verify_platform,
};

const METADATA_LIMIT: usize = 4 * 1024 * 1024;
const LAYER_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
const PULL_TIMEOUT: Duration = Duration::from_secs(300);
const CATALOG_LIMIT: u64 = 1024 * 1024;
/// The CI-produced filesystem artifact and its sole layer use the same type.
pub const ROOTFS_MEDIA_TYPE: &str = "application/vnd.capsem.rootfs.erofs.v1";

/// Verified, opaque filesystem bytes. Dropping the layout reclaims its staging.
pub struct RootfsLayout {
    directory: tempfile::TempDir,
    digest: ContentDigest,
}

impl RootfsLayout {
    /// The payload remains private to this layout; the host never unpacks it.
    pub fn path(&self) -> PathBuf {
        self.directory
            .path()
            .join(self.digest.as_str().trim_start_matches("sha256:"))
    }

    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// A disposable OCI layout. Keeping this value alive keeps its files alive.
pub struct ImageLayout {
    directory: tempfile::TempDir,
    /// Digest of the original platform manifest, before OCI media normalization.
    pub source_digest: String,
    /// Digest of what the reference resolved to: the multi-platform index
    /// when the registry served one (the identity catalogs and pinned
    /// references name), otherwise the platform manifest itself.
    pub image_digest: String,
    files: Vec<PathBuf>,
}

impl ImageLayout {
    pub fn path(&self) -> &Path {
        self.directory.path()
    }
    /// Relative paths; all blob names have been validated as SHA-256 digests.
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }
}

/// Registry transport remains on the host; no registry credentials enter a VM.
pub struct Puller {
    registry: Client,
    http: reqwest::Client,
    authentication: RegistryAuth,
    architecture: String,
    scheme: &'static str,
    cache: Option<BlobCache>,
}

impl Puller {
    pub fn new(architecture: &str, authentication: RegistryAuth) -> Result<Self> {
        Self::new_with_root_certificate(architecture, authentication, None)
    }

    pub fn new_with_root_certificate(
        architecture: &str,
        authentication: RegistryAuth,
        certificate: Option<&[u8]>,
    ) -> Result<Self> {
        let mut puller = Self::configured(architecture, authentication, ClientProtocol::Https, certificate)?;
        puller.cache = Some(BlobCache::installed()?);
        Ok(puller)
    }

    fn configured(
        architecture: &str,
        authentication: RegistryAuth,
        protocol: ClientProtocol,
        certificate: Option<&[u8]>,
    ) -> Result<Self> {
        ensure!(
            matches!(architecture, "arm64" | "amd64"),
            "unsupported container architecture"
        );
        let secure = matches!(protocol, ClientProtocol::Https);
        let roots = certificate
            .map(reqwest::Certificate::from_pem_bundle)
            .transpose()?
            .unwrap_or_default();
        ensure!(
            certificate.is_none() || !roots.is_empty(),
            "registry CA file contains no certificates"
        );
        let registry = Client::try_from(ClientConfig {
            protocol,
            read_timeout: Some(Duration::from_secs(30)),
            connect_timeout: Some(Duration::from_secs(10)),
            extra_root_certificates: certificate
                .map(|pem| {
                    vec![Certificate {
                        encoding: CertificateEncoding::Pem,
                        data: pem.to_vec(),
                    }]
                })
                .unwrap_or_default(),
            ..ClientConfig::default()
        })?;
        let mut http = reqwest::Client::builder()
            .https_only(secure)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10));
        for root in roots {
            http = http.add_root_certificate(root);
        }
        let http = http.build()?;
        Ok(Self {
            registry,
            http,
            authentication,
            architecture: architecture.into(),
            scheme: if secure { "https" } else { "http" },
            cache: None,
        })
    }

    pub async fn pull(&self, reference: &str, parent: &Path) -> Result<ImageLayout> {
        tokio::time::timeout(PULL_TIMEOUT, self.pull_inner(reference, parent))
            .await
            .context("OCI pull exceeded five minutes")?
    }

    /// Fetch and validate the image catalog at `reference`, usually
    /// `ghcr.io/google/capsem/catalog:<channel>`. The manifest is re-read on
    /// every call because the channel tag moves; the returned digest names
    /// this catalog version. The layer is verified against its descriptor and
    /// shares the image blob cache. `parent` holds the disposable staging.
    pub async fn fetch_catalog(&self, reference: &str, parent: &Path) -> Result<(ContentDigest, Catalog)> {
        tokio::time::timeout(PULL_TIMEOUT, self.fetch_catalog_inner(reference, parent))
            .await
            .context("catalog fetch exceeded five minutes")?
    }

    /// Fetch a catalog-pinned filesystem attached to the selected platform
    /// image. The caller supplies that image's verified original manifest
    /// digest, rather than its multi-platform index or normalized layout.
    pub async fn fetch_rootfs(&self, reference: &str, subject: &ContentDigest, parent: &Path) -> Result<RootfsLayout> {
        tokio::time::timeout(PULL_TIMEOUT, self.fetch_rootfs_inner(reference, subject, parent))
            .await
            .context("rootfs fetch exceeded five minutes")?
    }

    async fn fetch_rootfs_inner(
        &self,
        reference: &str,
        expected_subject: &ContentDigest,
        parent: &Path,
    ) -> Result<RootfsLayout> {
        let reference = image_reference(reference)?;
        ensure!(reference.digest().is_some(), "rootfs artifact must be pinned by digest");
        if let Some(cache) = &self.cache {
            cache.prepare().await?;
        }
        let token = self
            .registry
            .auth(&reference, &self.authentication, RegistryOperation::Pull)
            .await?;
        let bytes = self.manifest(&reference, token.as_deref()).await?;
        let manifest: OciImageManifest = serde_json::from_slice(&bytes).context("expected rootfs artifact manifest")?;
        ensure!(
            manifest.schema_version == 2
                && manifest.media_type.as_deref() == Some(OCI_IMAGE_MEDIA_TYPE)
                && manifest.artifact_type.as_deref() == Some(ROOTFS_MEDIA_TYPE),
            "unsupported rootfs artifact schema or media type"
        );
        let subject = manifest.subject.context("rootfs artifact has no image subject")?;
        ensure!(
            matches!(
                subject.media_type.as_str(),
                OCI_IMAGE_MEDIA_TYPE | IMAGE_MANIFEST_MEDIA_TYPE
            ) && subject.size > 0
                && subject.size <= METADATA_LIMIT as i64
                && subject.urls.as_ref().is_none_or(Vec::is_empty)
                && subject.digest == expected_subject.as_str(),
            "rootfs artifact subject does not match the selected platform image"
        );
        ensure!(
            manifest.config.media_type == "application/vnd.oci.empty.v1+json"
                && manifest.config.size == 2
                && manifest.config.digest == sha256(b"{}")
                && manifest.config.urls.as_ref().is_none_or(Vec::is_empty),
            "rootfs artifact must have the empty OCI config"
        );
        let [layer] = manifest.layers.as_slice() else {
            anyhow::bail!("rootfs artifact must contain exactly one layer");
        };
        ensure!(
            layer.media_type == ROOTFS_MEDIA_TYPE
                && layer.size > 0
                && layer.size as u64 <= IMAGE_LIMIT
                && layer.urls.as_ref().is_none_or(Vec::is_empty),
            "unsupported rootfs layer media type, size or external URLs"
        );
        let digest = ContentDigest::parse(&layer.digest)?;
        let directory = staging(parent).await?;
        self.blob(&reference, layer, directory.path()).await?;
        Ok(RootfsLayout { directory, digest })
    }

    async fn fetch_catalog_inner(&self, reference: &str, parent: &Path) -> Result<(ContentDigest, Catalog)> {
        let reference = image_reference(reference)?;
        if let Some(cache) = &self.cache {
            cache.prepare().await?;
        }
        let token = self
            .registry
            .auth(&reference, &self.authentication, RegistryOperation::Pull)
            .await?;
        let bytes = self.manifest(&reference, token.as_deref()).await?;
        let identity = ContentDigest::parse(&sha256(&bytes))?;
        let manifest: OciImageManifest =
            serde_json::from_slice(&bytes).context("catalog must be a single OCI artifact manifest")?;
        ensure!(manifest.schema_version == 2, "unsupported catalog manifest schema");
        let [layer] = manifest.layers.as_slice() else {
            anyhow::bail!(
                "catalog manifest must have exactly one layer, found {}",
                manifest.layers.len()
            );
        };
        ensure!(
            layer.media_type == CATALOG_MEDIA_TYPE,
            "catalog layer media type is {:?}, expected {CATALOG_MEDIA_TYPE}",
            layer.media_type
        );
        ensure!(
            layer.size > 0 && layer.size as u64 <= CATALOG_LIMIT,
            "catalog layer size {} is outside the 1 MiB limit",
            layer.size
        );
        ensure!(
            layer.urls.as_ref().is_none_or(Vec::is_empty),
            "external catalog URLs are unsupported"
        );
        let directory = staging(parent).await?;
        self.blob(&reference, layer, directory.path()).await?;
        let document = tokio::fs::read(directory.path().join(digest_hex(&layer.digest)?)).await?;
        Ok((identity, Catalog::parse(&document)?))
    }

    async fn pull_inner(&self, reference: &str, parent: &Path) -> Result<ImageLayout> {
        let reference = image_reference(reference)?;
        if let Some(cache) = &self.cache {
            cache.prepare().await?;
        }
        let token = self
            .registry
            .auth(&reference, &self.authentication, RegistryOperation::Pull)
            .await?;
        let first = self.manifest(&reference, token.as_deref()).await?;
        let image_digest = sha256(&first);
        let bytes = match serde_json::from_slice::<OciManifest>(&first)? {
            OciManifest::Image(_) => first,
            OciManifest::ImageIndex(index) => {
                ensure!(index.schema_version == 2, "unsupported OCI index schema");
                let selected = index
                    .manifests
                    .iter()
                    .find(|entry| {
                        entry.platform.as_ref().is_some_and(|p| {
                            p.os == "linux"
                                && p.architecture == self.architecture
                                && p.variant
                                    .as_deref()
                                    .is_none_or(|v| self.architecture == "arm64" && v == "v8")
                                && p.os_features.as_ref().is_none_or(Vec::is_empty)
                                && p.features.as_ref().is_none_or(Vec::is_empty)
                        })
                    })
                    .context("registry has no compatible native Linux image")?;
                digest_hex(&selected.digest)?;
                ensure!(
                    selected.size > 0 && selected.size <= METADATA_LIMIT as i64,
                    "manifest size exceeds limit"
                );
                let pinned = Reference::with_digest(
                    reference.registry().into(),
                    reference.repository().into(),
                    selected.digest.clone(),
                );
                let bytes = self.manifest(&pinned, token.as_deref()).await?;
                ensure!(bytes.len() == selected.size as usize, "manifest size mismatch");
                bytes
            }
        };
        let source_digest = sha256(&bytes);
        let mut manifest: OciImageManifest =
            serde_json::from_slice(&bytes).context("expected platform image manifest")?;
        validate_manifest(&mut manifest)?;
        let directory = staging(parent).await?;
        let blob_dir = directory.path().join("blobs/sha256");
        tokio::fs::create_dir_all(&blob_dir).await?;
        self.blob(&reference, &manifest.config, &blob_dir).await?;
        let config = tokio::fs::read(blob_dir.join(digest_hex(&manifest.config.digest)?)).await?;
        verify_platform(&config, &self.architecture)?;
        // Deduplicate shared layer descriptors before concurrent, create-new writes.
        let layers: BTreeMap<_, _> = manifest
            .layers
            .iter()
            .map(|layer| (layer.digest.clone(), layer))
            .collect();
        let downloads: Vec<_> = layers
            .values()
            .map(|layer| self.blob(&reference, layer, &blob_dir))
            .collect();
        stream::iter(downloads)
            .buffer_unordered(4)
            .try_collect::<Vec<_>>()
            .await?;
        let normalized = serde_json::to_vec(&manifest)?;
        let normalized_digest = sha256(&normalized);
        tokio::fs::write(blob_dir.join(digest_hex(&normalized_digest)?), &normalized).await?;
        let index = json!({"schemaVersion":2,"manifests":[{"mediaType":OCI_IMAGE_MEDIA_TYPE,"digest":normalized_digest,"size":normalized.len(),"annotations":{"org.opencontainers.image.ref.name":"image"}}]});
        tokio::fs::write(directory.path().join("index.json"), serde_json::to_vec(&index)?).await?;
        tokio::fs::write(
            directory.path().join("oci-layout"),
            br#"{"imageLayoutVersion":"1.0.0"}"#,
        )
        .await?;
        let mut files = vec![PathBuf::from("index.json"), PathBuf::from("oci-layout")];
        for digest in std::iter::once(&manifest.config.digest)
            .chain(layers.keys())
            .chain(std::iter::once(&normalized_digest))
        {
            files.push(PathBuf::from("blobs/sha256").join(digest_hex(digest)?));
        }
        Ok(ImageLayout {
            directory,
            source_digest,
            image_digest,
            files,
        })
    }

    // oci-client owns distribution authentication and blob transport. Its raw
    // manifest helper buffers the body without a cap; use bounded HTTP here.
    async fn manifest(&self, reference: &Reference, token: Option<&str>) -> Result<Vec<u8>> {
        let selector = reference
            .digest()
            .or(reference.tag())
            .context("image needs a tag or digest")?;
        let url = format!(
            "{}://{}/v2/{}/manifests/{selector}",
            self.scheme,
            reference.resolve_registry(),
            reference.repository()
        );
        let mut request = self.http.get(url).header(
            "Accept",
            [
                OCI_IMAGE_INDEX_MEDIA_TYPE,
                OCI_IMAGE_MEDIA_TYPE,
                IMAGE_MANIFEST_LIST_MEDIA_TYPE,
                IMAGE_MANIFEST_MEDIA_TYPE,
            ]
            .join(", "),
        );
        if let Some(token) = token {
            request = request.bearer_auth(token);
        } else if let RegistryAuth::Basic(username, password) = &self.authentication {
            request = request.basic_auth(username, Some(password));
        }
        let response = request.send().await?.error_for_status()?;
        ensure!(
            response
                .content_length()
                .is_none_or(|size| size <= METADATA_LIMIT as u64),
            "manifest size exceeds limit"
        );
        let header = response
            .headers()
            .get("docker-content-digest")
            .map(|v| v.to_str().map(str::to_owned))
            .transpose()?;
        let mut body = Vec::new();
        let mut chunks = response.bytes_stream();
        while let Some(chunk) = chunks.try_next().await? {
            ensure!(
                chunk.len() <= METADATA_LIMIT - body.len(),
                "manifest size exceeds limit"
            );
            body.extend_from_slice(&chunk);
        }
        let actual = sha256(&body);
        for expected in reference.digest().into_iter().chain(header.as_deref()) {
            digest_hex(expected)?;
            ensure!(actual == expected, "manifest digest mismatch");
        }
        Ok(body)
    }

    async fn blob(&self, reference: &Reference, descriptor: &OciDescriptor, directory: &Path) -> Result<()> {
        let destination = directory.join(digest_hex(&descriptor.digest)?);
        let cache = self.cache.as_ref().map(|cache| cache.for_repository(reference));
        let _lease = match &cache {
            Some(cache) => {
                let lease = cache.lease(&descriptor.digest).await?;
                if cache
                    .copy_hit(&descriptor.digest, descriptor.size as u64, &destination)
                    .await?
                {
                    tracing::debug!(digest = %descriptor.digest, "OCI blob cache hit");
                    return Ok(());
                }
                Some(lease)
            }
            None => None,
        };
        let mut chunks = self.registry.pull_blob_stream(reference, descriptor).await?;
        ensure!(
            chunks.content_length.is_none_or(|size| size == descriptor.size as u64),
            "blob size mismatch"
        );
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .await?;
        let mut count = 0u64;
        let mut digest = Sha256::new();
        while let Some(chunk) = chunks.try_next().await? {
            count += chunk.len() as u64;
            ensure!(count <= descriptor.size as u64, "blob exceeds declared size");
            digest.update(&chunk);
            file.write_all(&chunk).await?;
        }
        ensure!(count == descriptor.size as u64, "blob size mismatch");
        ensure!(
            format!("sha256:{:x}", digest.finalize()) == descriptor.digest,
            "blob digest mismatch"
        );
        file.flush().await?;
        drop(file);
        if let Some(cache) = &cache {
            cache.publish(&descriptor.digest, &destination).await?;
        }
        Ok(())
    }
}

/// A private, disposable directory under `parent` for verified downloads.
async fn staging(parent: &Path) -> Result<tempfile::TempDir> {
    let parent = parent.to_owned();
    Ok(tokio::task::spawn_blocking(move || {
        tempfile::Builder::new()
            .prefix("oci-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(parent)
    })
    .await??)
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn validate_manifest(manifest: &mut OciImageManifest) -> Result<()> {
    ensure!(
        manifest.schema_version == 2 && manifest.layers.len() <= 128,
        "unsupported manifest schema or layer count"
    );
    ensure!(
        manifest.artifact_type.is_none() && manifest.subject.is_none(),
        "expected runnable image, not an OCI artifact"
    );
    ensure!(
        matches!(
            manifest.config.media_type.as_str(),
            IMAGE_CONFIG_MEDIA_TYPE | IMAGE_DOCKER_CONFIG_MEDIA_TYPE
        ),
        "unsupported image config type"
    );
    ensure!(
        manifest.config.size <= METADATA_LIMIT as i64,
        "config size exceeds limit"
    );
    manifest.config.media_type = IMAGE_CONFIG_MEDIA_TYPE.into();
    manifest.media_type = Some(OCI_IMAGE_MEDIA_TYPE.into());
    let mut total = 0u64;
    for descriptor in std::iter::once(&manifest.config).chain(&manifest.layers) {
        digest_hex(&descriptor.digest)?;
        ensure!(
            descriptor.size >= 0 && descriptor.size as u64 <= LAYER_LIMIT,
            "layer size exceeds limit"
        );
        ensure!(
            descriptor.urls.as_ref().is_none_or(Vec::is_empty),
            "external layer URLs are unsupported"
        );
        total += descriptor.size as u64;
        ensure!(total <= IMAGE_LIMIT, "image size exceeds limit");
    }
    for layer in &mut manifest.layers {
        layer.media_type = match layer.media_type.as_str() {
            IMAGE_LAYER_MEDIA_TYPE | IMAGE_DOCKER_LAYER_TAR_MEDIA_TYPE => IMAGE_LAYER_MEDIA_TYPE,
            IMAGE_LAYER_GZIP_MEDIA_TYPE | IMAGE_DOCKER_LAYER_GZIP_MEDIA_TYPE => IMAGE_LAYER_GZIP_MEDIA_TYPE,
            _ => anyhow::bail!("unsupported image layer compression or media type"),
        }
        .into();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
