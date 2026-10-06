//! Durable cache metadata is not an authorization or readiness verdict.

use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{ensure, Context, Result};
use oci_client::Reference;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{image_reference, CacheIdentity, CacheKey, Digest, ImageReference, METADATA_LIMIT};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BlobKind {
    Metadata,
    Private,
    ImmutableRoot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BlobRef {
    pub(super) digest: String,
    pub(super) size: u64,
    pub(super) kind: BlobKind,
}

impl BlobRef {
    pub(super) fn new(digest: &str, size: u64, kind: BlobKind) -> Result<Self> {
        Digest::parse(digest)?;
        ensure!(size > 0, "empty required cache blob");
        ensure!(
            kind != BlobKind::Metadata || size <= METADATA_LIMIT as u64,
            "receipt metadata exceeds limit"
        );
        Ok(Self {
            digest: digest.to_owned(),
            size,
            kind,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RootReceipt {
    pub(super) origin: String,
    pub(super) subject: String,
    pub(super) subject_size: u64,
    pub(super) blobs: Vec<BlobRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    key: String,
    image: String,
    origin: String,
    architecture: String,
    runtime_contract: u32,
    native_digest: String,
    verified_at_unix_ns: u64,
    blobs: Vec<BlobRef>,
    root: Option<RootReceipt>,
}

#[derive(Clone, Debug)]
pub struct CacheReceipt {
    record: Record,
    identity: CacheIdentity,
    native_digest: Digest,
}

impl CacheReceipt {
    pub(super) fn new(
        identity: CacheIdentity,
        origin: &Reference,
        native: Digest,
        mut blobs: Vec<BlobRef>,
    ) -> Result<Self> {
        blobs.sort_by(|a, b| (&a.digest, a.kind).cmp(&(&b.digest, b.kind)));
        blobs.dedup();
        let origin = Reference::with_digest(
            origin.registry().into(),
            origin.repository().into(),
            identity.image().digest().to_string(),
        );
        let key = identity.key();
        let receipt = Self {
            record: Record {
                schema_version: 1,
                key: key.as_str().to_owned(),
                image: identity.image().to_string(),
                origin: origin.to_string(),
                architecture: identity.architecture().to_owned(),
                runtime_contract: identity.runtime_contract(),
                native_digest: native.to_string(),
                verified_at_unix_ns: now()?,
                blobs,
                root: None,
            },
            identity,
            native_digest: native,
        };
        receipt.validate(&key)?;
        Ok(receipt)
    }

    pub fn identity(&self) -> &CacheIdentity {
        &self.identity
    }
    pub fn native_digest(&self) -> &Digest {
        &self.native_digest
    }
    pub fn verified_at_unix_ns(&self) -> u64 {
        self.record.verified_at_unix_ns
    }
    pub fn key(&self) -> CacheKey {
        self.identity.key()
    }

    pub fn generation(&self) -> Result<String> {
        Ok(format!("{:x}", Sha256::digest(self.encode()?)))
    }

    pub(super) fn origin(&self) -> &str {
        &self.record.origin
    }
    pub(super) fn blobs(&self) -> &[BlobRef] {
        &self.record.blobs
    }
    pub(super) fn matches_image(&self, verified: &Self) -> bool {
        self.identity == verified.identity
            && self.native_digest == verified.native_digest
            && self.origin() == verified.origin()
            && self.blobs() == verified.blobs()
    }
    pub(super) fn root(&self) -> Option<&RootReceipt> {
        self.record.root.as_ref()
    }

    pub(super) fn with_root(mut self, root: RootReceipt) -> Result<Self> {
        self.record.root = Some(root);
        self.record.verified_at_unix_ns = now()?;
        self.validate(&self.key())?;
        Ok(self)
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(&self.record)?;
        ensure!(bytes.len() <= METADATA_LIMIT, "cache receipt exceeds metadata limit");
        Ok(bytes)
    }

    pub(super) fn decode(bytes: &[u8], expected: &CacheKey) -> Result<Self> {
        ensure!(bytes.len() <= METADATA_LIMIT, "cache receipt exceeds metadata limit");
        let record: Record = serde_json::from_slice(bytes).context("invalid cache receipt")?;
        let receipt = Self {
            identity: CacheIdentity::new(&record.image, &record.architecture, record.runtime_contract)?,
            native_digest: Digest::parse(&record.native_digest)?,
            record,
        };
        receipt.validate(expected)?;
        Ok(receipt)
    }

    fn validate(&self, expected: &CacheKey) -> Result<()> {
        ensure!(self.record.schema_version == 1, "unsupported cache receipt schema");
        ensure!(
            self.key() == *expected && self.record.key == expected.as_str(),
            "cache receipt key mismatch"
        );
        ensure!(
            self.record.verified_at_unix_ns > 0,
            "cache receipt has no verification timestamp"
        );
        let origin = image_reference(self.origin())?;
        let origin = ImageReference::try_from(&origin)?;
        ensure!(
            origin.digest() == Some(self.identity.image().digest())
                && origin.repository() == self.identity.image().repository(),
            "cache receipt origin mismatch"
        );
        validate_blobs(self.blobs())?;
        ensure!(
            self.blobs().iter().all(|blob| blob.kind != BlobKind::ImmutableRoot),
            "filesystem blobs require a bound root receipt"
        );
        for required in [self.identity.image().digest(), self.native_digest()] {
            ensure!(
                self.blobs()
                    .iter()
                    .any(|blob| blob.kind == BlobKind::Metadata && blob.digest == required.as_str()),
                "cache receipt omits required manifest"
            );
        }
        ensure!(
            self.blobs().iter().any(|blob| blob.kind == BlobKind::Private),
            "cache receipt omits image configuration"
        );
        if let Some(root) = self.root() {
            let reference = image_reference(&root.origin)?;
            let identity = ImageReference::try_from(&reference)?;
            ensure!(
                identity.repository() == self.identity.image().repository() && identity.digest().is_some(),
                "root receipt repository or pin mismatch"
            );
            ensure!(
                root.subject == self.native_digest().as_str(),
                "root receipt subject mismatch"
            );
            let subject_size = self
                .blobs()
                .iter()
                .find(|blob| blob.kind == BlobKind::Metadata && blob.digest == root.subject)
                .context("missing native subject metadata")?
                .size;
            ensure!(root.subject_size == subject_size, "root receipt subject size mismatch");
            validate_blobs(&root.blobs)?;
            ensure!(
                root.blobs.len() == 2
                    && root
                        .blobs
                        .iter()
                        .any(|blob| blob.kind == BlobKind::Metadata && Some(blob.digest.as_str()) == reference.digest())
                    && root.blobs.iter().any(|blob| blob.kind == BlobKind::ImmutableRoot),
                "root receipt must describe its manifest and filesystem"
            );
        }
        Ok(())
    }
}

fn validate_blobs(blobs: &[BlobRef]) -> Result<()> {
    let mut seen = HashSet::new();
    for blob in blobs {
        BlobRef::new(&blob.digest, blob.size, blob.kind)?;
        ensure!(seen.insert((&blob.digest, blob.kind)), "duplicate receipt blob");
    }
    Ok(())
}

fn now() -> Result<u64> {
    Ok(u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())?)
}

#[cfg(test)]
mod tests;
