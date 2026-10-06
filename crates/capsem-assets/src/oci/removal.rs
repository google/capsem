//! Removal planning describes exact cache ownership, never arbitrary paths.

use std::{collections::HashSet, fs::Metadata, os::unix::fs::MetadataExt};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{receipts::BlobKind, CacheKey, ImageCache, METADATA_LIMIT};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileState {
    pub(super) dev: u64,
    pub(super) ino: u64,
    pub(super) allocated: u64,
    pub(super) links: u64,
    size: u64,
    mode: u32,
    uid: u32,
    mtime: [i64; 2],
    ctime: [i64; 2],
}

impl FileState {
    pub(super) fn from_metadata(metadata: &Metadata) -> Result<Self> {
        Ok(Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            allocated: metadata
                .blocks()
                .checked_mul(512)
                .context("OCI removal allocation overflow")?,
            links: metadata.nlink(),
            size: metadata.len(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            mtime: [metadata.mtime(), metadata.mtime_nsec()],
            ctime: [metadata.ctime(), metadata.ctime_nsec()],
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BlobState {
    pub(super) name: String,
    pub(super) kind: BlobKind,
    pub(super) file: Option<FileState>,
    pub(super) shared: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Witness {
    pub(super) key: String,
    pub(super) generation: String,
    references: Vec<(String, String)>,
    pub(super) receipt: FileState,
    pub(super) blobs: Vec<BlobState>,
    materializing: bool,
}

/// A nonsecret preview token binds only current receipt/reference/inode
/// facts. It grants no authorization and contains no inferred session IDs.
#[derive(Clone, Debug)]
pub struct RemovalPreview {
    key: CacheKey,
    token: String,
    pub(super) witness: Witness,
    reclaimable_bytes: u64,
}

impl RemovalPreview {
    pub(super) fn new(
        key: CacheKey,
        generation: String,
        references: Vec<(String, String)>,
        receipt: FileState,
        blobs: Vec<BlobState>,
        materializing: bool,
    ) -> Result<Self> {
        let mut seen = HashSet::new();
        let mut reclaimable_bytes = 0u64;
        for file in std::iter::once(&receipt).chain(
            blobs
                .iter()
                .filter(|blob| !blob.shared)
                .filter_map(|blob| blob.file.as_ref()),
        ) {
            if file.links == 1 && seen.insert((file.dev, file.ino)) {
                reclaimable_bytes = reclaimable_bytes
                    .checked_add(file.allocated)
                    .context("OCI removal allocation overflow")?;
            }
        }
        let witness = Witness {
            key: key.as_str().into(),
            generation,
            references,
            receipt,
            blobs,
            materializing,
        };
        let token = preview_token(&witness)?;
        Ok(Self {
            key,
            token,
            witness,
            reclaimable_bytes,
        })
    }

    pub fn key(&self) -> &CacheKey {
        &self.key
    }
    pub fn token(&self) -> &str {
        &self.token
    }
    pub fn reclaimable_bytes(&self) -> u64 {
        self.reclaimable_bytes
    }
    pub fn protected_roots(&self) -> usize {
        self.witness
            .blobs
            .iter()
            .filter(|blob| {
                blob.kind == BlobKind::ImmutableRoot && blob.file.as_ref().is_some_and(|file| file.links > 1)
            })
            .count()
    }
    pub fn allowed(&self) -> bool {
        !self.witness.materializing && self.protected_roots() == 0
    }
}

pub(super) fn preview_token(witness: &Witness) -> Result<String> {
    let bytes = serde_json::to_vec(witness)?;
    ensure!(
        bytes.len() <= METADATA_LIMIT,
        "OCI removal preview exceeds metadata limit"
    );
    let mut digest = Sha256::new();
    digest.update(b"capsem-oci-removal-preview-v1\0");
    digest.update(&bytes);
    Ok(format!("remove-{:x}", digest.finalize()))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemovalResult {
    pub removed_allocated_bytes: u64,
    pub removed_entries: u64,
    pub already_missing_entries: u64,
    pub retained_entries: u64,
    pub complete: bool,
}

impl ImageCache {
    /// Read current ownership without creating controls or changing payloads.
    pub async fn preview_removal(&self, key: &CacheKey) -> Result<RemovalPreview> {
        self.inner.preview_removal(key).await
    }
    pub async fn apply_removal(&self, key: &CacheKey, token: &str, reason: &str) -> Result<RemovalResult> {
        super::Digest::parse(&format!(
            "sha256:{}",
            token.strip_prefix("remove-").context("invalid removal token")?
        ))?;
        ensure!(!reason.trim().is_empty(), "removal requires a reason");
        ensure!(reason.len() <= METADATA_LIMIT, "removal reason exceeds metadata limit");
        self.inner.apply_removal(key, token, reason).await
    }
}
