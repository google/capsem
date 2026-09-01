use std::path::Path;

use anyhow::{Context, Result};
use capsem_core::net::policy_config::{ProfileCatalog, ProfileConfigFile};
use serde::Deserialize;

use super::{
    atomic_write, has_scheme_authority_prefix, profile_catalog_revision, release_http_get_bytes, validate_blake3_hex,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize)]
struct PublishedProfileCatalogDocument {
    schema: String,
    revision: String,
    #[allow(dead_code)]
    state: Option<String>,
    profiles: Vec<ProfileConfigFile>,
}

pub(super) async fn stage(source: &str, expected_hash: &str, channel_source: &str, target_dir: &Path) -> Result<()> {
    validate_blake3_hex("profile catalog hash", expected_hash)?;
    let catalog_url = resolve_release_channel_artifact_url(channel_source, source)?;
    let bytes = read_profile_catalog_source(&catalog_url).await?;
    let actual_hash = blake3::hash(&bytes).to_hex().to_string();
    if actual_hash != expected_hash {
        anyhow::bail!("profile catalog hash mismatch for {catalog_url}: expected {expected_hash}, got {actual_hash}");
    }
    let document = parse_profile_catalog_document(&bytes, &catalog_url)?;
    let parent = target_dir.parent().context("profile catalog stage has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    if target_dir.exists() {
        std::fs::remove_dir_all(target_dir).with_context(|| format!("replace {}", target_dir.display()))?;
    }
    std::fs::create_dir(target_dir).with_context(|| format!("create {}", target_dir.display()))?;
    materialize_profile_catalog(target_dir, &document, &catalog_url, expected_hash)
}

pub(super) fn resolve_release_channel_artifact_url(channel_source: &str, artifact: &str) -> Result<String> {
    let trimmed = artifact.trim();
    if trimmed.is_empty() {
        anyhow::bail!("release channel profile catalog source is empty");
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") || trimmed.starts_with("file://") {
        let parsed = reqwest::Url::parse(trimmed).with_context(|| format!("parse profile catalog URL {trimmed}"))?;
        return Ok(parsed.to_string());
    }

    let base =
        reqwest::Url::parse(channel_source).with_context(|| format!("parse release channel URL {channel_source}"))?;
    if trimmed.starts_with('/') {
        // A site-root-relative reference, because a generated channel is a
        // website: the manifest sits at `<root>/assets/<channel>/manifest.json`
        // and its artifacts are recorded as `/profiles/...`.
        //
        // Over http(s) the site root is the origin, so replacing the path is
        // exactly right. A `file://` channel is that same tree on disk, and its
        // root is the dist directory rather than the filesystem root -- so
        // `set_path` alone sent every local hydration to `/profiles/...` and
        // failed with ENOENT.
        if base.scheme() == "file" {
            if let Some(dist) = base.path().rfind("/assets/") {
                let mut root = base.clone();
                root.set_path(&format!("{}{trimmed}", &base.path()[..dist]));
                root.set_query(None);
                root.set_fragment(None);
                return Ok(root.to_string());
            }
        }
        let mut root = base;
        root.set_path(trimmed);
        root.set_query(None);
        root.set_fragment(None);
        return Ok(root.to_string());
    }
    base.join(trimmed)
        .with_context(|| format!("resolve profile catalog {trimmed} against {channel_source}"))
        .map(|url| url.to_string())
}

async fn read_profile_catalog_source(source: &str) -> Result<Vec<u8>> {
    let url =
        reqwest::Url::parse(source).with_context(|| format!("profile catalog source must be a URL, got {source}"))?;
    match url.scheme() {
        "file" => {
            if !has_scheme_authority_prefix(source, "file") {
                anyhow::bail!("profile catalog file URL must start with file://: {source}");
            }
            let path = url
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("profile catalog file URL must be absolute: {source}"))?;
            std::fs::read(&path).with_context(|| format!("read profile catalog {}", path.display()))
        }
        "http" | "https" => {
            if !has_scheme_authority_prefix(source, url.scheme()) {
                anyhow::bail!("profile catalog source must use https://, http://, or file:// URLs, got {source}");
            }
            release_http_get_bytes(url.clone(), Some("application/json"), source)
                .await
                .with_context(|| format!("read profile catalog body from {source}"))
        }
        scheme => anyhow::bail!("unsupported profile catalog URL scheme {scheme}: use https://, http://, or file://"),
    }
}

fn parse_profile_catalog_document(bytes: &[u8], source: &str) -> Result<PublishedProfileCatalogDocument> {
    let document: PublishedProfileCatalogDocument =
        serde_json::from_slice(bytes).with_context(|| format!("parse profile catalog from {source}"))?;
    if document.schema != "capsem.profile_catalog.v1" {
        anyhow::bail!("profile catalog schema mismatch");
    }
    if document.profiles.is_empty() {
        anyhow::bail!("profile catalog contains no profiles");
    }
    for profile in &document.profiles {
        profile
            .validate()
            .map_err(|error| anyhow::anyhow!("validate profile {}: {error}", profile.id))?;
    }
    let revision = profile_catalog_revision(document.profiles.iter().collect::<Vec<_>>().as_slice())?;
    if revision != document.revision {
        anyhow::bail!(
            "profile catalog revision mismatch: document advertises {}, profiles resolve to {}",
            document.revision,
            revision
        );
    }
    Ok(document)
}

fn materialize_profile_catalog(
    tmp_dir: &Path,
    document: &PublishedProfileCatalogDocument,
    source: &str,
    hash: &str,
) -> Result<()> {
    for profile in &document.profiles {
        let profile_dir = tmp_dir.join(&profile.id);
        std::fs::create_dir(&profile_dir).with_context(|| format!("create {}", profile_dir.display()))?;
        let bytes = toml::to_string_pretty(profile).with_context(|| format!("serialize profile {}", profile.id))?;
        atomic_write(&profile_dir.join("profile.toml"), bytes.as_bytes())?;
    }
    let origin = serde_json::json!({
        "schema": "capsem.profile_catalog_origin.v1",
        "origin": "update",
        "source": source,
        "hash": hash,
        "revision": document.revision
    });
    atomic_write(
        &tmp_dir.join("catalog-origin.json"),
        &serde_json::to_vec_pretty(&origin)?,
    )?;
    ProfileCatalog::load_from_dir(tmp_dir)
        .map_err(|error| anyhow::anyhow!("validate installed profile catalog: {error}"))?;
    Ok(())
}
