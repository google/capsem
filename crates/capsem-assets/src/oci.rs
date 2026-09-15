//! Native Linux OCI image resolution and verified, unextracted image layouts.

use anyhow::{ensure, Context, Result};
use oci_client::Reference;
use serde::Deserialize;

mod cache;
mod pull;
pub use oci_client::secrets::RegistryAuth;
pub use pull::{ImageLayout, Puller};

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
    ensure!(
        hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid SHA-256 digest"
    );
    Ok(hex)
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
