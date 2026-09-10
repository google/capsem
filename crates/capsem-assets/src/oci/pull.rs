use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{ensure, Context, Result};
use futures::{stream, StreamExt, TryStreamExt};
use oci_client::{
    client::{ClientConfig, ClientProtocol},
    manifest::*,
    secrets::RegistryAuth,
    Client, Reference, RegistryOperation,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::{digest_hex, image_reference, verify_platform};

const METADATA_LIMIT: usize = 4 * 1024 * 1024;
const LAYER_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
const PULL_TIMEOUT: Duration = Duration::from_secs(300);

/// A disposable OCI layout. Keeping this value alive keeps its files alive.
pub struct ImageLayout {
    directory: tempfile::TempDir,
    /// Digest of the original platform manifest, before OCI media normalization.
    pub source_digest: String,
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
}

impl Puller {
    pub fn new(architecture: &str, authentication: RegistryAuth) -> Result<Self> {
        Self::configured(architecture, authentication, ClientProtocol::Https)
    }

    fn configured(architecture: &str, authentication: RegistryAuth, protocol: ClientProtocol) -> Result<Self> {
        ensure!(
            matches!(architecture, "arm64" | "amd64"),
            "unsupported container architecture"
        );
        let secure = matches!(protocol, ClientProtocol::Https);
        let registry = Client::try_from(ClientConfig {
            protocol,
            read_timeout: Some(Duration::from_secs(30)),
            connect_timeout: Some(Duration::from_secs(10)),
            ..ClientConfig::default()
        })?;
        let http = reqwest::Client::builder()
            .https_only(secure)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            registry,
            http,
            authentication,
            architecture: architecture.into(),
            scheme: if secure { "https" } else { "http" },
        })
    }

    pub async fn pull(&self, reference: &str, parent: &Path) -> Result<ImageLayout> {
        tokio::time::timeout(PULL_TIMEOUT, self.pull_inner(reference, parent))
            .await
            .context("OCI pull exceeded five minutes")?
    }

    async fn pull_inner(&self, reference: &str, parent: &Path) -> Result<ImageLayout> {
        let reference = image_reference(reference)?;
        let token = self
            .registry
            .auth(&reference, &self.authentication, RegistryOperation::Pull)
            .await?;
        let first = self.manifest(&reference, token.as_deref()).await?;
        let bytes = match serde_json::from_slice::<OciManifest>(&first)? {
            OciManifest::Image(_) => first,
            OciManifest::ImageIndex(index) => {
                ensure!(index.schema_version == 2, "unsupported OCI index schema");
                let selected = index
                    .manifests
                    .iter()
                    .find(|entry| {
                        entry.platform.as_ref().is_some_and(|p| {
                            p.os.to_string() == "linux"
                                && p.architecture.to_string() == self.architecture
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
        let parent = parent.to_owned();
        let directory =
            tokio::task::spawn_blocking(move || tempfile::Builder::new().prefix("oci-").tempdir_in(parent)).await??;
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
        stream::iter(layers.values().map(|layer| self.blob(&reference, layer, &blob_dir)))
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
        let mut chunks = self.registry.pull_blob_stream(reference, descriptor).await?;
        ensure!(
            chunks.content_length.is_none_or(|size| size == descriptor.size as u64),
            "blob size mismatch"
        );
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(digest_hex(&descriptor.digest)?))
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
        Ok(())
    }
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
