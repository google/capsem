//! Native Linux OCI image resolution and verified, unextracted image layouts.

use anyhow::{ensure, Context, Result};
use oci_client::Reference;
use serde::Deserialize;

mod cache;
mod catalog;
mod identity;
mod owner;
mod worker;
pub use worker::CacheReconciler;
mod removal;
pub use owner::ImageCache;
pub use removal::{RemovalPreview, RemovalResult};
mod inventory;
pub use identity::{CacheIdentity, CacheKey};
pub use inventory::{CacheInventory, CacheUsage, ImageInventory};
mod readiness;
mod receipts;
pub use readiness::{CacheReason, CacheSnapshot, CacheState};
pub use receipts::CacheReceipt;

const METADATA_LIMIT: usize = 4 * 1024 * 1024;
const PULL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
mod pull;
pub use catalog::{
    is_catalog_name, Catalog, CatalogEntry, CatalogEvidence, CatalogVersion, Channel, CATALOG_MEDIA_TYPE,
    DEFAULT_CATALOG, RUNTIME_CONTRACT,
};
mod selector;
pub use oci_client::secrets::RegistryAuth;
pub use pull::{
    BlobProgress, ImageLayout, InventoryPackage, InventoryTool, PackageInventory, ProgressObserver, Puller,
    RootfsLayout, ROOTFS_MEDIA_TYPE,
};
pub use selector::{admits, allows_source, Digest, ImageReference, ImageSelector, Repository, ResolvedImage};

/// Require a qualified registry reference or explicit `docker://` prefix so
/// existing shell-command arguments cannot silently turn into image pulls.
pub fn image_reference(value: &str) -> Result<Reference> {
    let explicit = value.strip_prefix("docker://");
    let value = explicit.unwrap_or(value);
    ensure!(
        !value.is_empty() && !value.chars().any(char::is_whitespace),
        "invalid image reference"
    );
    let registry = value.split('/').next().unwrap_or_default();
    ensure!(
        explicit.is_some()
            || (value.contains('/') && (registry.contains('.') || registry.contains(':') || registry == "localhost")),
        "use a qualified registry/image or docker://image"
    );
    ensure!(!value.contains("://"), "unsupported image URL scheme");
    let reference: Reference = value.parse().context("invalid OCI reference")?;
    if let Some(digest) = reference.digest() {
        digest_hex(digest)?;
    }
    Ok(reference)
}

fn digest_hex(value: &str) -> Result<&str> {
    let hex = value
        .strip_prefix("sha256:")
        .context("only SHA-256 OCI digests are supported")?;
    ensure!(lower_hex(hex, 64), "invalid SHA-256 digest");
    Ok(hex)
}

fn lower_hex(value: &str, width: usize) -> bool {
    value.len() == width && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Owned cache records use their actual persisted JSON bytes, streamed into
/// XXH3 without a temporary document. This is change/corruption detection,
/// never an OCI content address or permission to remove or execute anything.
fn metadata_fingerprint(value: &impl serde::Serialize, domain: &[u8]) -> Result<String> {
    let mut hash = capsem_crypto::xxh3::Hasher::new();
    hash.update(domain);
    let mut bounded = capsem_foundation::limited_writer::LimitedWriter::new(&mut hash, METADATA_LIMIT);
    let encoded = serde_json::to_writer(&mut bounded, value);
    ensure!(!bounded.exceeded(), "cache metadata exceeds metadata limit");
    encoded?;
    Ok(format!("xxh3-128:{:032x}", hash.finalize()))
}

fn verify_platform(config: &[u8], architecture: &str) -> Result<()> {
    #[derive(Deserialize)]
    struct Platform {
        os: String,
        architecture: String,
    }
    let platform: Platform = serde_json::from_slice(config).context("invalid OCI image config")?;
    ensure!(
        platform.os == "linux" && platform.architecture == architecture,
        "image must target linux/{architecture}, got {}/{}",
        platform.os,
        platform.architecture
    );
    Ok(())
}

#[cfg(test)]
mod tests;
