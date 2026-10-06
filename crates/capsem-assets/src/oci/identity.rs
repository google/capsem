//! Cache identity describes retained bytes, never permission to execute them.

use anyhow::{ensure, Context, Result};
use sha2::{Digest as _, Sha256};

use super::{image_reference, Digest, ImageReference, ResolvedImage};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheIdentity {
    image: ResolvedImage,
    architecture: String,
    runtime_contract: u32,
}

impl CacheIdentity {
    pub fn new(reference: &str, architecture: &str, runtime_contract: u32) -> Result<Self> {
        let reference = image_reference(reference)?;
        let digest = Digest::parse(
            reference
                .digest()
                .context("cache identities require an immutable image pin")?,
        )?;
        let image = ImageReference::try_from(&reference)?.resolve(digest)?;
        Self::from_resolved(image, architecture, runtime_contract)
    }

    pub(super) fn from_resolved(image: ResolvedImage, architecture: &str, runtime_contract: u32) -> Result<Self> {
        ensure!(
            matches!(architecture, "amd64" | "arm64"),
            "invalid native cache architecture"
        );
        ensure!(runtime_contract > 0, "invalid cache runtime contract");
        Ok(Self {
            image,
            architecture: architecture.to_owned(),
            runtime_contract,
        })
    }

    pub fn image(&self) -> &ResolvedImage {
        &self.image
    }

    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    pub fn runtime_contract(&self) -> u32 {
        self.runtime_contract
    }

    pub fn key(&self) -> CacheKey {
        let mut digest = Sha256::new();
        digest.update(b"capsem-oci-cache-key-v1\0");
        digest.update(self.image.to_string().as_bytes());
        digest.update([0]);
        digest.update(self.architecture.as_bytes());
        digest.update([0]);
        digest.update(self.runtime_contract.to_be_bytes());
        CacheKey(format!("oci-{:x}", digest.finalize()))
    }
}

/// Opaque owner lookup key. Parsing a key does not grant removal authority.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey(String);

impl CacheKey {
    pub fn parse(value: &str) -> Result<Self> {
        let hex = value.strip_prefix("oci-").context("invalid OCI cache key")?;
        Digest::parse(&format!("sha256:{hex}"))?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests;
