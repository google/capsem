//! Self-update: check the release manifest for binary and VM asset versions.
//!
//! The release manifest URL is the source of truth for freshness. The binary
//! path selects a platform installer from manifest metadata when the release
//! publishes one; the privileged installer apply step is intentionally separate
//! from VM asset hydration.

#[cfg(test)]
mod runtime_contract_tests;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::platform::{self, InstallLayout};
use capsem_core::net::policy_config::{ProfileCatalog, ProfileConfigFile};

const RELEASE_HTTP_ATTEMPTS: usize = 4;
const RELEASE_HTTP_INITIAL_BACKOFF_MS: u64 = 250;

/// Cached update check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheck {
    /// Unix timestamp of when we last checked.
    pub checked_at: u64,
    /// Latest version available (None if check failed).
    #[serde(default)]
    pub latest_version: Option<String>,
    /// Whether an update is available.
    #[serde(default)]
    pub update_available: bool,
    /// Platform installer selected from the release channel, when one matches
    /// this installation layout.
    #[serde(default)]
    pub binary_installer: Option<BinaryInstaller>,
    /// Latest VM asset set advertised by the release channel.
    #[serde(default)]
    pub latest_assets: Option<String>,
    /// Installed VM asset set derived from the local manifest, when known.
    #[serde(default)]
    pub current_assets: Option<String>,
    /// Whether the advertised asset set differs from the installed manifest.
    #[serde(default)]
    pub assets_update_available: bool,
    /// Release-channel state for VM asset updates.
    #[serde(default)]
    pub assets_state: Option<String>,
    /// Why the advertised VM asset set cannot be applied by this install.
    #[serde(default)]
    pub assets_blocked_reason: Option<String>,
    /// Latest profile catalog advertised by the release channel, when published.
    #[serde(default)]
    pub latest_profiles: Option<String>,
    /// Installed profile catalog revision derived from the local catalog.
    #[serde(default)]
    pub current_profiles: Option<String>,
    /// Whether the advertised profile catalog differs from the installed one.
    #[serde(default)]
    pub profiles_update_available: bool,
    /// Release-channel state for profile updates.
    #[serde(default)]
    pub profiles_state: Option<String>,
    /// Why the advertised profile catalog cannot be applied by this install.
    #[serde(default)]
    pub profiles_blocked_reason: Option<String>,
    /// URL or release-channel artifact path for the advertised profile catalog.
    #[serde(default)]
    pub profile_catalog_source: Option<String>,
    /// BLAKE3 digest of the advertised profile catalog payload.
    #[serde(default)]
    pub profile_catalog_hash: Option<String>,
    /// Latest VM image catalog advertised by the release channel, when published.
    #[serde(default)]
    pub latest_images: Option<String>,
    /// Whether the advertised image catalog differs from the installed one.
    #[serde(default)]
    pub images_update_available: bool,
    /// Release-channel state for image updates.
    #[serde(default)]
    pub images_state: Option<String>,
    /// Why the advertised image track cannot be applied by this install.
    #[serde(default)]
    pub images_blocked_reason: Option<String>,
    /// Machine-readable release channel index used for this check.
    #[serde(default, rename = "checked_url")]
    pub source: Option<String>,
    /// SHA-256 of the last valid release-channel payload.
    #[serde(default)]
    pub channel_hash: Option<String>,
    /// Validation state for the last release-channel refresh attempt.
    #[serde(default)]
    pub validation_status: Option<String>,
    /// Validation or fetch error for the last release-channel refresh attempt.
    #[serde(default)]
    pub validation_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BinaryInstaller {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub install_layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BinaryInstallerApplyPlan {
    commands: Vec<BinaryInstallerApplyCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChannelTransition {
    Preserve,
    Public(String),
    Corporate,
}

impl BinaryInstallerApplyPlan {
    fn command_lines(&self) -> Vec<String> {
        self.commands
            .iter()
            .map(BinaryInstallerApplyCommand::command_line)
            .collect()
    }
}

async fn apply_binary_installer_plan(plan: &BinaryInstallerApplyPlan) -> Result<()> {
    for command in &plan.commands {
        let line = command.command_line();
        info!("applying binary update with package manager: {line}");
        let status = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .status()
            .await
            .with_context(|| format!("run binary update apply command: {line}"))?;
        if !status.success() {
            anyhow::bail!(
                "binary update apply command failed with {status}: {line}. Current installation was left for the package manager to preserve or repair."
            );
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BinaryInstallerApplyCommand {
    program: String,
    args: Vec<String>,
}

impl BinaryInstallerApplyCommand {
    fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

const CACHE_TTL_SECS: u64 = 24 * 3600; // 24 hours
const DEFAULT_RELEASE_MANIFEST_URL: &str = "https://release.capsem.org/assets/stable/manifest.json";
const DEFAULT_RELEASE_CHANNELS_URL: &str = "https://release.capsem.org/channels.json";
const RELEASE_MANIFEST_URL_ENV: &str = "CAPSEM_RELEASE_MANIFEST_URL";
const RELEASE_CHANNELS_URL_ENV: &str = "CAPSEM_RELEASE_CHANNELS_URL";
const LEGACY_RELEASE_HEALTH_URL_ENV: &str = "CAPSEM_RELEASE_HEALTH_URL";

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct ReleaseChannelHealth {
    schema: String,
    updates: ReleaseChannelUpdates,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct ReleaseChannelUpdates {
    binary: ReleaseChannelUpdateTarget,
    assets: ReleaseChannelUpdateTarget,
    #[serde(default)]
    profiles: Option<ReleaseChannelUpdateTarget>,
    #[serde(default)]
    images: Option<ReleaseChannelUpdateTarget>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct ReleaseChannelUpdateTarget {
    #[serde(default)]
    latest: Option<String>,
    #[serde(default)]
    current: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    hash: Option<String>,
    #[serde(default)]
    compatibility: Option<ReleaseChannelCompatibility>,
    #[serde(default)]
    requires_newer: Option<ReleaseChannelRequiresNewer>,
    #[serde(default)]
    files: Vec<ReleaseChannelBinaryFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelCompatibility {
    #[serde(default)]
    min_binary: Option<String>,
    #[serde(default)]
    min_assets: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelRequiresNewer {
    #[serde(default)]
    binary: bool,
    #[serde(default)]
    assets: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelBinaryFile {
    name: String,
    url: String,
    sha256: String,
    #[serde(default)]
    blake3: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct PublishedProfileCatalogDocument {
    schema: String,
    revision: String,
    #[allow(dead_code)]
    state: Option<String>,
    profiles: Vec<ProfileConfigFile>,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannelProfileManifest {
    #[allow(dead_code)]
    version: String,
    #[serde(default)]
    profiles: BTreeMap<String, ReleaseChannelProfileDocument>,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannelProfileDocument {
    revision: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    architectures: Vec<ReleaseChannelProfileArchitecture>,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannelProfileArchitecture {
    architecture: String,
    #[serde(default)]
    image_revision: Option<String>,
    #[serde(default)]
    config: Vec<ReleaseChannelProfileConfig>,
    #[serde(default, rename = "images")]
    artifacts: Vec<ReleaseChannelProfileImage>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelProfileConfig {
    path: String,
    url: String,
    #[serde(rename = "bytes")]
    size: u64,
    digest: ReleaseChannelProfileDigest,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelProfileImage {
    kind: String,
    name: String,
    url: String,
    #[serde(rename = "bytes")]
    size: u64,
    digest: ReleaseChannelProfileDigest,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelProfileDigest {
    sha256: String,
    blake3: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseGraphManifest {
    #[serde(default)]
    packages: Vec<ReleaseGraphPackage>,
    #[serde(default)]
    profiles: BTreeMap<String, ReleaseChannelProfileDocument>,
}

#[derive(Debug, Deserialize)]
struct ReleaseGraphPackage {
    name: String,
    url: String,
    version: String,
    kind: String,
    platform: String,
    architecture: String,
    #[serde(default)]
    status: String,
    #[serde(rename = "bytes")]
    size: u64,
    digest: ReleaseGraphDigest,
}

#[derive(Debug, Deserialize)]
struct ReleaseGraphDigest {
    sha256: String,
    blake3: String,
}

#[derive(Debug, Clone)]
struct ReleaseChannelAssetDownload {
    logical_name: String,
    url: String,
    size: u64,
    sha256: String,
    blake3: String,
}

#[derive(Debug, Clone)]
struct ReleaseChannelProfileConfigDownload {
    profile_id: String,
    relative_path: std::path::PathBuf,
    url: String,
    size: u64,
    sha256: String,
    blake3: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelsCatalog {
    version: u64,
    channels: BTreeMap<String, ReleaseChannelRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelRecord {
    manifests: Vec<ReleaseManifestRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseManifestRecord {
    version: String,
    status: ReleaseManifestStatus,
    url: String,
    digest: ReleaseManifestDigest,
    #[serde(default)]
    min_capsem_version: Option<String>,
    #[serde(default)]
    max_capsem_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReleaseManifestStatus {
    Current,
    Supported,
    Deprecated,
    Revoked,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseManifestDigest {
    sha256: String,
    blake3: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedReleaseChannelManifest {
    channel: String,
    url: String,
    sha256: String,
    blake3: String,
}

impl ReleaseChannelUpdateTarget {
    #[allow(dead_code)]
    fn latest_version(&self) -> Option<String> {
        self.latest.clone().or_else(|| self.current.clone())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn manifest_metadata_path() -> Option<PathBuf> {
    capsem_core::asset_manager::default_assets_dir()
        .map(|assets_dir| assets_dir.join("manifest-metadata.json"))
}

/// Validate user-provided update source flags.
///
/// The CLI intentionally accepts URL strings only. Local files must be written
/// as `file://` so normal release channels and corporate channels share one
/// update/provenance mechanism.
pub fn validate_source_url_arg(flag: &str, value: &str) -> std::result::Result<String, String> {
    let parsed = reqwest::Url::parse(value).map_err(|_| {
        format!(
            "{flag} must be a URL: use https://..., http://..., or file:///absolute/path, got {value}"
        )
    })?;
    match parsed.scheme() {
        "https" | "http" => {
            if !has_scheme_authority_prefix(value, parsed.scheme()) {
                return Err(format!(
                    "{flag} must use https://, http://, or file:// URLs, got {value}"
                ));
            }
            Ok(value.to_string())
        }
        "file" => {
            if !has_scheme_authority_prefix(value, "file") {
                return Err(format!("{flag} file URL must start with file://: {value}"));
            }
            parsed
                .to_file_path()
                .map(|_| value.to_string())
                .map_err(|_| format!("{flag} file URL must be absolute: {value}"))
        }
        scheme => Err(format!(
            "unsupported {flag} URL scheme {scheme}: use https://, http://, or file://"
        )),
    }
}

fn has_scheme_authority_prefix(value: &str, scheme: &str) -> bool {
    let prefix = format!("{scheme}://");
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&prefix))
}

/// Read cached update notice. Sync file read, no latency.
/// Returns a message to display if an update is available and cache is fresh.
pub fn read_cached_update_notice() -> Option<String> {
    let source = release_manifest_url().ok()?;
    let check = read_cache_for_source(&source).ok()?;

    if !check.update_available
        && !check.assets_update_available
        && !check.profiles_update_available
        && check.profiles_blocked_reason.is_none()
    {
        return None;
    }

    // Only show if cache is still fresh
    let age = now_secs().saturating_sub(check.checked_at);
    if age > CACHE_TTL_SECS {
        return None;
    }

    let current = env!("CARGO_PKG_VERSION");
    if let Some(latest) = check.latest_version {
        if is_newer(&latest, current) {
            return Some(format!(
                "Update available: {} -> {}. Run `capsem update` to inspect.",
                current, latest
            ));
        }
    }

    if check.assets_update_available {
        if let Some(latest_assets) = check.latest_assets {
            return Some(format!(
                "VM asset update available: {latest_assets}. Run `capsem update --assets` to refresh."
            ));
        }
    }

    if check.profiles_update_available {
        if let Some(latest_profiles) = check.latest_profiles {
            return Some(format!(
                "Profile catalog update available: {latest_profiles}. Run `capsem update` to refresh."
            ));
        }
    }

    if let Some(reason) = check.profiles_blocked_reason {
        return Some(format!(
            "Profile catalog update blocked: {reason}. Run `capsem update --check` for details."
        ));
    }

    None
}

/// Write update check cache atomically (write tmp + rename).
fn write_cache(check: &UpdateCheck) -> Result<()> {
    check
        .source
        .as_deref()
        .context("update check source missing")?;
    let path = manifest_metadata_path().context("HOME not set")?;
    write_cache_to_path(&path, check)
}

fn write_cache_to_path(path: &Path, check: &UpdateCheck) -> Result<()> {
    let mut metadata = read_manifest_metadata_value(path)?.unwrap_or_else(|| {
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1"
        })
    });
    let object = metadata
        .as_object_mut()
        .context("manifest metadata must be a JSON object")?;
    object.insert(
        "schema".to_string(),
        serde_json::json!("capsem.manifest_metadata.v1"),
    );
    let check_value = serde_json::to_value(check).context("serialize update check")?;
    for (key, value) in check_value
        .as_object()
        .context("serialized update check must be an object")?
    {
        object.insert(key.clone(), value.clone());
    }
    let json = serde_json::to_string_pretty(&metadata)?;
    write_json_atomic(path, &json)
}

fn write_json_atomic(path: &Path, json: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

fn read_cache_for_source(source: &str) -> Result<UpdateCheck> {
    let path = manifest_metadata_path().context("HOME not set")?;
    let check = read_cache_file(&path)?;
    if check.source.as_deref() == Some(source) {
        Ok(check)
    } else {
        anyhow::bail!(
            "manifest metadata {} was last checked against {:?}, not {source}",
            path.display(),
            check.source
        )
    }
}

fn read_cache_file(path: &Path) -> Result<UpdateCheck> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))
}

fn read_manifest_metadata_value(path: &Path) -> Result<Option<serde_json::Value>> {
    let Some(bytes) = read_optional_file(path)? else {
        return Ok(None);
    };
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    if value.get("schema").and_then(serde_json::Value::as_str)
        != Some("capsem.manifest_metadata.v1")
    {
        anyhow::bail!(
            "{} must use schema capsem.manifest_metadata.v1",
            path.display()
        );
    }
    Ok(Some(value))
}

fn channel_payload_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn failed_update_check_from_previous(
    previous: Option<UpdateCheck>,
    checked_at: u64,
    source: &str,
    validation_status: &str,
    validation_error: String,
) -> UpdateCheck {
    let mut check = previous.unwrap_or(UpdateCheck {
        checked_at,
        latest_version: None,
        update_available: false,
        binary_installer: None,
        latest_assets: None,
        current_assets: None,
        assets_update_available: false,
        assets_state: None,
        assets_blocked_reason: None,
        latest_profiles: None,
        current_profiles: None,
        profiles_update_available: false,
        profiles_state: None,
        profiles_blocked_reason: None,
        profile_catalog_source: None,
        profile_catalog_hash: None,
        latest_images: None,
        images_update_available: false,
        images_state: None,
        images_blocked_reason: None,
        source: Some(source.to_string()),
        channel_hash: None,
        validation_status: None,
        validation_error: None,
    });
    check.checked_at = checked_at;
    check.source = Some(source.to_string());
    check.validation_status = Some(validation_status.to_string());
    check.validation_error = Some(validation_error);
    check
}

/// Background refresh: check the release channel for updates if cache is stale.
/// Fire-and-forget via tokio::spawn.
pub async fn refresh_update_cache_if_stale() {
    let manifest_url = match release_manifest_url() {
        Ok(url) => url,
        Err(e) => {
            warn!(error = %e, "update check: invalid release manifest URL");
            return;
        }
    };
    let path = match manifest_metadata_path() {
        Some(p) => p,
        None => return,
    };

    // The single manifest metadata file owns the last check for this install.
    let previous_check = read_cache_file(&path)
        .ok()
        .filter(|check| check.source.as_deref() == Some(manifest_url.as_str()));
    if let Some(check) = previous_check.as_ref() {
        let age = now_secs().saturating_sub(check.checked_at);
        if age < CACHE_TTL_SECS {
            return; // Still fresh
        }
    }

    info!(source = %manifest_url, "update cache stale, checking for updates");

    let client = reqwest::Client::new();
    let resp = match client
        .get(&manifest_url)
        .header("Accept", "application/json")
        .header("User-Agent", "capsem")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            warn!(status = %r.status(), url = %manifest_url, "update check: release manifest error");
            return;
        }
        Err(e) => {
            warn!(error = %e, "update check failed");
            let check = failed_update_check_from_previous(
                previous_check,
                now_secs(),
                &manifest_url,
                "fetch_error",
                e.to_string(),
            );
            let _ = write_cache(&check);
            return;
        }
    };

    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(error = %e, url = %manifest_url, "update check: failed to read release manifest body");
            let check = failed_update_check_from_previous(
                previous_check,
                now_secs(),
                &manifest_url,
                "fetch_error",
                e.to_string(),
            );
            let _ = write_cache(&check);
            return;
        }
    };
    let channel_hash = channel_payload_hash(&bytes);
    let check = match update_check_from_release_payload(
        &bytes,
        &platform::detect_install_layout(),
        &manifest_url,
        Some(channel_hash),
    ) {
        Ok(check) => check,
        Err(e) => {
            warn!(error = %e, url = %manifest_url, "update check: invalid release manifest contract");
            let check = failed_update_check_from_previous(
                previous_check,
                now_secs(),
                &manifest_url,
                "invalid_contract",
                e.to_string(),
            );
            let _ = write_cache(&check);
            return;
        }
    };
    let _ = write_cache(&check);
}

fn release_manifest_url() -> Result<String> {
    release_manifest_url_for_layout(&platform::detect_install_layout())
}

fn release_manifest_url_for_layout(layout: &InstallLayout) -> Result<String> {
    if let Ok(value) = std::env::var(RELEASE_MANIFEST_URL_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            if !matches!(layout, InstallLayout::Development) {
                anyhow::bail!(
                    "{RELEASE_MANIFEST_URL_ENV} is a development-only override; installed Capsem must use manifest-metadata.json provenance"
                );
            }
            return validate_release_manifest_url(trimmed);
        }
    }
    if let Ok(value) = std::env::var(LEGACY_RELEASE_HEALTH_URL_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            if !matches!(layout, InstallLayout::Development) {
                anyhow::bail!(
                    "{LEGACY_RELEASE_HEALTH_URL_ENV} is a development-only override; installed Capsem must use manifest-metadata.json provenance"
                );
            }
            return validate_release_manifest_url(trimmed);
        }
    }

    if let Some(url) = release_manifest_url_from_manifest_metadata()? {
        return Ok(url);
    }

    if matches!(layout, InstallLayout::Development) {
        return Ok(DEFAULT_RELEASE_MANIFEST_URL.to_string());
    }

    let metadata_path = manifest_metadata_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<CAPSEM_HOME>/assets/manifest-metadata.json".to_string());
    anyhow::bail!(
        "installed Capsem requires {metadata_path}; refusing to silently select the stable channel"
    )
}

fn select_channel_manifest_url(
    catalog: &ReleaseChannelsCatalog,
    channel: &str,
    capsem_version: &str,
) -> Result<String> {
    if catalog.version == 0 {
        anyhow::bail!("channels catalog version must be non-zero");
    }
    let capsem_version = semver::Version::parse(capsem_version)
        .with_context(|| format!("parse Capsem version {capsem_version}"))?;
    let record = catalog
        .channels
        .get(channel)
        .ok_or_else(|| anyhow::anyhow!("channel {channel} is not listed"))?;
    record
        .manifests
        .iter()
        .filter(|manifest| manifest.status != ReleaseManifestStatus::Revoked)
        .filter(|manifest| manifest_is_compatible_with_capsem(manifest, &capsem_version))
        .map(|manifest| {
            validate_channel_manifest_record(channel, manifest)?;
            Ok(manifest)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .min_by_key(|manifest| manifest.status.selection_rank())
        .map(|manifest| manifest.url.clone())
        .ok_or_else(|| anyhow::anyhow!("channel {channel} has no compatible selectable manifest"))
}

fn manifest_is_compatible_with_capsem(
    manifest: &ReleaseManifestRecord,
    capsem_version: &semver::Version,
) -> bool {
    if let Some(min) = &manifest.min_capsem_version {
        let Ok(min) = semver::Version::parse(min) else {
            return false;
        };
        if capsem_version < &min {
            return false;
        }
    }
    if let Some(max) = &manifest.max_capsem_version {
        let Ok(max) = semver::Version::parse(max) else {
            return false;
        };
        if capsem_version > &max {
            return false;
        }
    }
    true
}

fn validate_channel_manifest_record(channel: &str, manifest: &ReleaseManifestRecord) -> Result<()> {
    if manifest.version.trim().is_empty() {
        anyhow::bail!("channel {channel} manifest version must not be empty");
    }
    if manifest.url.trim().is_empty() {
        anyhow::bail!(
            "channel {channel} manifest {} url must not be empty",
            manifest.version
        );
    }
    if !(manifest.url.starts_with('/')
        || manifest.url.starts_with("https://")
        || manifest.url.starts_with("http://"))
    {
        anyhow::bail!(
            "channel {channel} manifest {} url must be release-site relative or http(s): {}",
            manifest.version,
            manifest.url
        );
    }
    validate_hex_digest(&manifest.digest.sha256, 64, "manifest digest sha256")?;
    validate_hex_digest(&manifest.digest.blake3, 64, "manifest digest blake3")?;
    Ok(())
}

impl ReleaseManifestStatus {
    fn selection_rank(self) -> u8 {
        match self {
            ReleaseManifestStatus::Current => 0,
            ReleaseManifestStatus::Supported => 1,
            ReleaseManifestStatus::Deprecated => 2,
            ReleaseManifestStatus::Revoked => 255,
        }
    }
}

pub fn validate_channel_name(channel: &str) -> Result<String> {
    let channel = channel.trim();
    if channel.is_empty()
        || !channel
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        anyhow::bail!(
            "--channel must contain only lowercase letters, digits, and hyphens, got {channel:?}"
        );
    }
    Ok(channel.to_string())
}

async fn resolve_release_channel_manifest(channel: &str) -> Result<ResolvedReleaseChannelManifest> {
    let channel = validate_channel_name(channel)?;
    let channels_url = release_channels_url()?;
    let catalog_url = reqwest::Url::parse(&channels_url).context("parse release channels URL")?;
    let bytes =
        release_http_get_bytes(catalog_url.clone(), Some("application/json"), &channels_url)
            .await
            .with_context(|| format!("read release channels from {channels_url}"))?;
    let catalog: ReleaseChannelsCatalog = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse release channels from {channels_url}"))?;
    let selected_url = select_channel_manifest_url(&catalog, &channel, env!("CARGO_PKG_VERSION"))?;
    let record = catalog
        .channels
        .get(&channel)
        .and_then(|entry| {
            entry
                .manifests
                .iter()
                .find(|manifest| manifest.url == selected_url)
        })
        .ok_or_else(|| anyhow::anyhow!("selected manifest disappeared from channel {channel}"))?;
    let url = catalog_url
        .join(&selected_url)
        .with_context(|| format!("resolve channel {channel} manifest URL {selected_url}"))?;
    if !matches!(url.scheme(), "https" | "http") {
        anyhow::bail!("channel {channel} resolved to unsupported manifest URL {url}");
    }
    Ok(ResolvedReleaseChannelManifest {
        channel,
        url: url.to_string(),
        sha256: record.digest.sha256.clone(),
        blake3: record.digest.blake3.clone(),
    })
}

fn release_channels_url() -> Result<String> {
    let value = std::env::var(RELEASE_CHANNELS_URL_ENV)
        .unwrap_or_else(|_| DEFAULT_RELEASE_CHANNELS_URL.to_string());
    let parsed = reqwest::Url::parse(value.trim()).with_context(|| {
        format!("{RELEASE_CHANNELS_URL_ENV} must be an https:// or http:// URL, got {value}")
    })?;
    if !matches!(parsed.scheme(), "https" | "http") {
        anyhow::bail!(
            "unsupported {RELEASE_CHANNELS_URL_ENV} scheme {}: use https:// or http://",
            parsed.scheme()
        );
    }
    Ok(parsed.to_string())
}

fn verify_selected_channel_manifest(
    selection: &ResolvedReleaseChannelManifest,
    bytes: &[u8],
) -> Result<()> {
    let actual_sha256 = sha256_hex(bytes);
    if actual_sha256 != selection.sha256 {
        anyhow::bail!(
            "channel {} manifest SHA-256 mismatch: expected {}, got {}",
            selection.channel,
            selection.sha256,
            actual_sha256
        );
    }
    let actual_blake3 = blake3::hash(bytes).to_hex().to_string();
    if actual_blake3 != selection.blake3 {
        anyhow::bail!(
            "channel {} manifest BLAKE3 mismatch: expected {}, got {}",
            selection.channel,
            selection.blake3,
            actual_blake3
        );
    }
    Ok(())
}

fn validate_release_manifest_url(url: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(url).with_context(|| {
        format!(
            "{RELEASE_MANIFEST_URL_ENV} must be a URL: use https://... or http://..., got {url}"
        )
    })?;
    if !matches!(parsed.scheme(), "https" | "http") {
        anyhow::bail!(
            "unsupported {RELEASE_MANIFEST_URL_ENV} scheme {}: use https:// or http://",
            parsed.scheme()
        );
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn release_manifest_url_from_manifest_metadata() -> Result<Option<String>> {
    let Some(metadata_path) = manifest_metadata_path() else {
        return Ok(None);
    };
    let Some(value) = read_manifest_metadata_value(&metadata_path)? else {
        return Ok(None);
    };
    let source = value
        .get("manifest_url")
        .and_then(serde_json::Value::as_str)
        .context("manifest-metadata.json must contain string field manifest_url")?;
    let source = release_manifest_url_from_manifest_url(source).with_context(|| {
        format!(
            "manifest-metadata.json manifest_url must be an http(s) channel manifest URL, got {source}"
        )
    })?;
    Ok(Some(source))
}

fn release_manifest_url_from_manifest_url(manifest_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(manifest_url).ok()?;
    if !matches!(url.scheme(), "https" | "http") {
        return None;
    }
    let segments = url.path_segments().map(|segments| {
        segments
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
    })?;
    let assets_pos = segments.iter().position(|segment| *segment == "assets")?;
    if segments.last().map(String::as_str) != Some("manifest.json")
        || segments.len() < assets_pos + 3
    {
        return None;
    }
    Some(url.to_string())
}

fn local_current_asset_version() -> Option<String> {
    let assets_dir = capsem_core::asset_manager::default_assets_dir()?;
    let manifest_path = assets_dir.join("manifest.json");
    let manifest_bytes = std::fs::read_to_string(manifest_path).ok()?;
    let manifest = capsem_core::asset_manager::ManifestV2::from_json(&manifest_bytes).ok()?;
    Some(manifest.assets.current)
}

fn local_current_binary_version() -> String {
    let package_version = capsem_core::asset_manager::default_assets_dir()
        .and_then(|assets_dir| std::fs::read(assets_dir.join("manifest-metadata.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|origin| {
            origin
                .get("package_version")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        });
    package_version.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

fn local_current_profile_catalog_revision() -> Option<String> {
    let catalog = capsem_core::net::policy_config::ProfileCatalog::load_default().ok()?;
    profile_catalog_revision(catalog.profiles().collect::<Vec<_>>().as_slice()).ok()
}

fn profile_catalog_revision(
    profiles: &[&capsem_core::net::policy_config::ProfileConfigFile],
) -> Result<String> {
    let mut revisions = profiles
        .iter()
        .map(|profile| profile.revision.as_str())
        .collect::<BTreeSet<_>>();
    if revisions.len() == 1 {
        let revision = revisions
            .pop_first()
            .ok_or_else(|| anyhow::anyhow!("profile catalog revision set is empty"))?;
        return Ok(revision.to_string());
    }
    let bytes = serde_json::to_vec(profiles).context("serialize profile catalog for hashing")?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    Ok(format!("catalog-{}", &hash[..16]))
}

fn update_target_blocked_reason(
    target: &ReleaseChannelUpdateTarget,
    current_binary: &str,
    current_assets: Option<&str>,
) -> Option<String> {
    let requires = target.requires_newer.as_ref();
    let compatibility = target.compatibility.as_ref();
    let mut reasons = Vec::new();

    if requires.is_some_and(|requires| requires.binary)
        || compatibility
            .and_then(|compatibility| compatibility.min_binary.as_deref())
            .is_some_and(|min_binary| is_newer(min_binary, current_binary))
    {
        let version = compatibility
            .and_then(|compatibility| compatibility.min_binary.as_deref())
            .unwrap_or("a newer version");
        reasons.push(format!("requires binary {version} or newer"));
    }

    if requires.is_some_and(|requires| requires.assets)
        || compatibility
            .and_then(|compatibility| compatibility.min_assets.as_deref())
            .zip(current_assets)
            .is_some_and(|(min_assets, current_assets)| min_assets != current_assets)
    {
        let version = compatibility
            .and_then(|compatibility| compatibility.min_assets.as_deref())
            .unwrap_or("a newer version");
        reasons.push(format!("requires assets {version} or newer"));
    }
    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join(" and "))
    }
}

#[allow(clippy::too_many_arguments)]
fn update_check_from_release_manifest(
    manifest: &capsem_core::asset_manager::ManifestV2,
    checked_at: u64,
    current_binary: &str,
    current_assets: Option<&str>,
    current_profiles: Option<&str>,
    install_layout: &InstallLayout,
    source: &str,
    channel_hash: Option<String>,
) -> Result<UpdateCheck> {
    let latest_version = Some(manifest.binaries.current.clone());
    let latest_assets = Some(manifest.assets.current.clone());
    let asset_release = manifest.assets.releases.get(&manifest.assets.current);
    let assets_state = asset_release.map(|release| {
        if release.deprecated {
            "deprecated".to_string()
        } else {
            "current".to_string()
        }
    });
    let binary_release = manifest.binaries.releases.get(&manifest.binaries.current);
    let binary_files =
        binary_release_files_from_manifest(manifest, binary_release, source).unwrap_or_default();
    let update_available = latest_version
        .as_deref()
        .is_some_and(|latest| is_newer(latest, current_binary));
    let binary_installer = if update_available {
        binary_installer_for_layout(&binary_files, install_layout)
    } else {
        None
    };
    let assets_differ = match (latest_assets.as_deref(), current_assets) {
        (Some(latest), Some(current)) => latest != current,
        _ => false,
    };
    let asset_target = ReleaseChannelUpdateTarget {
        latest: latest_assets.clone(),
        current: latest_assets.clone(),
        state: assets_state.clone(),
        source: Some(source.to_string()),
        hash: channel_hash.clone(),
        compatibility: asset_release.map(|release| ReleaseChannelCompatibility {
            min_binary: if release.min_binary.is_empty() {
                None
            } else {
                Some(release.min_binary.clone())
            },
            min_assets: None,
        }),
        requires_newer: None,
        files: Vec::new(),
    };
    let assets_blocked_reason = if assets_differ
        && assets_state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("deprecated"))
    {
        Some("latest VM asset release is deprecated".to_string())
    } else if assets_differ {
        update_target_blocked_reason(&asset_target, current_binary, current_assets)
    } else {
        None
    };
    Ok(UpdateCheck {
        checked_at,
        latest_version,
        update_available,
        binary_installer,
        latest_assets,
        current_assets: current_assets.map(ToOwned::to_owned),
        assets_update_available: assets_differ && assets_blocked_reason.is_none(),
        assets_state,
        assets_blocked_reason,
        latest_profiles: None,
        current_profiles: current_profiles.map(ToOwned::to_owned),
        profiles_update_available: false,
        profiles_state: None,
        profiles_blocked_reason: None,
        profile_catalog_source: None,
        profile_catalog_hash: None,
        latest_images: None,
        images_update_available: false,
        images_state: None,
        images_blocked_reason: None,
        source: Some(source.to_string()),
        channel_hash,
        validation_status: Some("valid".to_string()),
        validation_error: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn update_check_from_release_graph_manifest(
    manifest: &ReleaseGraphManifest,
    checked_at: u64,
    current_binary: &str,
    current_assets: Option<&str>,
    current_profiles: Option<&str>,
    install_layout: &InstallLayout,
    source: &str,
    channel_hash: Option<String>,
) -> Result<UpdateCheck> {
    let latest_version = graph_current_binary_version(&manifest.packages)?;
    let update_available = latest_version
        .as_deref()
        .is_some_and(|latest| is_newer(latest, current_binary));
    let binary_installer = if update_available {
        graph_binary_installer_for_layout(&manifest.packages, install_layout, source)
    } else {
        None
    };
    let latest_assets = graph_current_image_revision(manifest);
    let assets_differ = match (latest_assets.as_deref(), current_assets) {
        (Some(latest), Some(current)) => latest != current,
        _ => false,
    };
    Ok(UpdateCheck {
        checked_at,
        latest_version,
        update_available,
        binary_installer,
        latest_assets: latest_assets.clone(),
        current_assets: current_assets.map(ToOwned::to_owned),
        assets_update_available: assets_differ,
        assets_state: latest_assets.as_ref().map(|_| "current".to_string()),
        assets_blocked_reason: None,
        latest_profiles: graph_current_profile_revision(manifest),
        current_profiles: current_profiles.map(ToOwned::to_owned),
        profiles_update_available: false,
        profiles_state: None,
        profiles_blocked_reason: None,
        profile_catalog_source: None,
        profile_catalog_hash: None,
        latest_images: latest_assets,
        images_update_available: false,
        images_state: None,
        images_blocked_reason: None,
        source: Some(source.to_string()),
        channel_hash,
        validation_status: Some("valid".to_string()),
        validation_error: None,
    })
}

fn graph_current_binary_version(packages: &[ReleaseGraphPackage]) -> Result<Option<String>> {
    let versions: BTreeSet<String> = packages
        .iter()
        .filter(|package| graph_package_is_current(package))
        .map(|package| package.version.clone())
        .collect();
    match versions.len() {
        0 => Ok(None),
        1 => Ok(versions.into_iter().next()),
        _ => anyhow::bail!(
            "release graph current package versions disagree: {}",
            versions.into_iter().collect::<Vec<_>>().join(", ")
        ),
    }
}

fn graph_current_image_revision(manifest: &ReleaseGraphManifest) -> Option<String> {
    let revisions: BTreeSet<String> = manifest
        .profiles
        .values()
        .flat_map(|profile| profile.architectures.iter())
        .filter_map(|architecture| architecture.image_revision.clone())
        .collect();
    if revisions.len() == 1 {
        revisions.into_iter().next()
    } else {
        None
    }
}

fn graph_current_profile_revision(manifest: &ReleaseGraphManifest) -> Option<String> {
    let revisions: BTreeSet<String> = manifest
        .profiles
        .values()
        .filter(|profile| profile.status.is_empty() || profile.status == "current")
        .map(|profile| profile.revision.clone())
        .collect();
    if revisions.len() == 1 {
        revisions.into_iter().next()
    } else {
        None
    }
}

fn graph_binary_installer_for_layout(
    packages: &[ReleaseGraphPackage],
    install_layout: &InstallLayout,
    source: &str,
) -> Option<BinaryInstaller> {
    packages
        .iter()
        .filter(|package| graph_package_is_current(package))
        .filter(|package| graph_package_matches_layout(package, install_layout))
        .filter_map(|package| {
            let installer = BinaryInstaller {
                name: package.name.clone(),
                url: graph_package_url(source, &package.url).ok()?,
                sha256: package.digest.sha256.clone(),
                size: package.size,
                install_layout: graph_install_layout_name(install_layout)?.to_string(),
            };
            let graph_digest_valid = package.digest.blake3.len() == 64
                && package.digest.blake3.chars().all(|c| c.is_ascii_hexdigit());
            if graph_digest_valid && validate_binary_installer_metadata(&installer).is_ok() {
                Some(installer)
            } else {
                None
            }
        })
        .min_by(|left, right| left.name.cmp(&right.name))
}

fn graph_package_is_current(package: &ReleaseGraphPackage) -> bool {
    package.status == "current"
}

fn graph_package_matches_layout(
    package: &ReleaseGraphPackage,
    install_layout: &InstallLayout,
) -> bool {
    match install_layout {
        InstallLayout::MacosPkg => {
            package.kind == "macos_pkg"
                && package.platform == "macos"
                && package.name.ends_with(".pkg")
        }
        InstallLayout::LinuxDeb => {
            package.kind == "debian_package"
                && package.platform == "linux"
                && package.architecture == deb_graph_arch()
                && package.name.ends_with(&format!("_{}.deb", deb_arch()))
        }
        InstallLayout::UserDir | InstallLayout::Development => false,
    }
}

fn graph_install_layout_name(install_layout: &InstallLayout) -> Option<&'static str> {
    match install_layout {
        InstallLayout::MacosPkg => Some("macos_pkg"),
        InstallLayout::LinuxDeb => Some("linux_deb"),
        InstallLayout::UserDir | InstallLayout::Development => None,
    }
}

fn graph_package_url(source: &str, raw: &str) -> Result<String> {
    reqwest::Url::parse(raw)
        .or_else(|_| reqwest::Url::parse(source)?.join(raw))
        .map(|url| url.to_string())
        .with_context(|| format!("resolve package URL {raw} against {source}"))
}

fn binary_release_files_from_manifest(
    manifest: &capsem_core::asset_manager::ManifestV2,
    release: Option<&capsem_core::asset_manager::BinaryRelease>,
    source: &str,
) -> Result<Vec<ReleaseChannelBinaryFile>> {
    let Some(release) = release else {
        return Ok(Vec::new());
    };
    release
        .files
        .iter()
        .map(|file| {
            let url = manifest_binary_file_url(manifest, source, &file.name)?;
            Ok(ReleaseChannelBinaryFile {
                name: file.name.clone(),
                url,
                sha256: file.sha256.clone(),
                blake3: file.blake3.clone(),
                size: file.size,
            })
        })
        .collect()
}

fn manifest_binary_file_url(
    manifest: &capsem_core::asset_manager::ManifestV2,
    source: &str,
    name: &str,
) -> Result<String> {
    if let Some(asset_base) = manifest
        .asset_base
        .as_deref()
        .filter(|base| !base.is_empty())
    {
        let base = reqwest::Url::parse(asset_base)
            .or_else(|_| reqwest::Url::parse(source)?.join(asset_base))
            .with_context(|| format!("resolve binary file asset_base {asset_base}"))?;
        return base
            .join(name)
            .map(|url| url.to_string())
            .with_context(|| format!("resolve binary file {name} against {base}"));
    }
    let source_url = reqwest::Url::parse(source)
        .with_context(|| format!("parse release manifest URL {source}"))?;
    source_url
        .join(name)
        .map(|url| url.to_string())
        .with_context(|| format!("resolve binary file {name} against manifest URL {source}"))
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
fn update_check_from_release_health(
    legacy: &ReleaseChannelHealth,
    checked_at: u64,
    current_binary: &str,
    current_assets: Option<&str>,
    current_profiles: Option<&str>,
    install_layout: &InstallLayout,
    source: &str,
    channel_hash: Option<String>,
) -> Result<UpdateCheck> {
    if legacy.schema != "capsem.assets_channel.legacy.v1" {
        anyhow::bail!("release channel legacy schema mismatch");
    }
    let latest_version = legacy.updates.binary.latest_version();
    let latest_assets = legacy.updates.assets.latest_version();
    let assets_state = legacy.updates.assets.state.clone();
    let latest_profiles = legacy
        .updates
        .profiles
        .as_ref()
        .and_then(ReleaseChannelUpdateTarget::latest_version);
    let profiles_state = legacy
        .updates
        .profiles
        .as_ref()
        .and_then(|target| target.state.clone());
    let profile_catalog_source = legacy
        .updates
        .profiles
        .as_ref()
        .and_then(|target| target.source.clone());
    let profile_catalog_hash = legacy
        .updates
        .profiles
        .as_ref()
        .and_then(|target| target.hash.clone());
    let latest_images = legacy
        .updates
        .images
        .as_ref()
        .and_then(ReleaseChannelUpdateTarget::latest_version);
    let images_state = legacy
        .updates
        .images
        .as_ref()
        .and_then(|target| target.state.clone());
    let update_available = latest_version
        .as_deref()
        .is_some_and(|latest| is_newer(latest, current_binary));
    let binary_installer = if update_available {
        binary_installer_for_layout(&legacy.updates.binary.files, install_layout)
    } else {
        None
    };
    let assets_differ = match (latest_assets.as_deref(), current_assets) {
        (Some(latest), Some(current)) => latest != current,
        _ => false,
    };
    let assets_blocked_reason = if assets_differ
        && assets_state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("deprecated"))
    {
        Some("latest VM asset release is deprecated".to_string())
    } else if assets_differ {
        update_target_blocked_reason(&legacy.updates.assets, current_binary, current_assets)
    } else {
        None
    };
    let assets_update_available = assets_differ && assets_blocked_reason.is_none();
    let profiles_differ = match (latest_profiles.as_deref(), current_profiles) {
        (Some(latest), Some(current)) => latest != current,
        _ => false,
    };
    let profiles_blocked_reason = legacy
        .updates
        .profiles
        .as_ref()
        .and_then(|target| update_target_blocked_reason(target, current_binary, current_assets))
        .or_else(|| {
            if profiles_differ && profile_catalog_source.is_none() {
                Some("release channel did not advertise a profile catalog source".to_string())
            } else if profiles_differ && profile_catalog_hash.is_none() {
                Some("release channel did not advertise a profile catalog hash".to_string())
            } else {
                None
            }
        });
    let images_blocked_reason =
        legacy.updates.images.as_ref().and_then(|target| {
            update_target_blocked_reason(target, current_binary, current_assets)
        });
    let profiles_update_available = profiles_blocked_reason.is_none()
        && profile_catalog_source.is_some()
        && profile_catalog_hash.is_some()
        && profiles_differ;
    Ok(UpdateCheck {
        checked_at,
        latest_version,
        update_available,
        binary_installer,
        latest_assets,
        current_assets: current_assets.map(ToOwned::to_owned),
        assets_update_available,
        assets_state,
        assets_blocked_reason,
        latest_profiles,
        current_profiles: current_profiles.map(ToOwned::to_owned),
        profiles_update_available,
        profiles_state,
        profiles_blocked_reason,
        profile_catalog_source,
        profile_catalog_hash,
        latest_images,
        images_update_available: false,
        images_state,
        images_blocked_reason,
        source: Some(source.to_string()),
        channel_hash,
        validation_status: Some("valid".to_string()),
        validation_error: None,
    })
}

fn binary_installer_for_layout(
    files: &[ReleaseChannelBinaryFile],
    install_layout: &InstallLayout,
) -> Option<BinaryInstaller> {
    enum LayoutMatcher {
        MacosPkg,
        LinuxDeb { suffix: String },
    }

    impl LayoutMatcher {
        fn name(&self) -> &'static str {
            match self {
                Self::MacosPkg => "macos_pkg",
                Self::LinuxDeb { .. } => "linux_deb",
            }
        }

        fn matches(&self, name: &str) -> bool {
            match self {
                Self::MacosPkg => name.ends_with(".pkg"),
                Self::LinuxDeb { suffix } => name.ends_with(suffix),
            }
        }
    }

    let matcher = match install_layout {
        InstallLayout::MacosPkg => LayoutMatcher::MacosPkg,
        InstallLayout::LinuxDeb => {
            let suffix = format!("_{}.deb", deb_arch());
            LayoutMatcher::LinuxDeb { suffix }
        }
        InstallLayout::UserDir | InstallLayout::Development => return None,
    };

    files
        .iter()
        .filter(|file| matcher.matches(&file.name))
        .filter(|file| {
            let installer = BinaryInstaller {
                name: file.name.clone(),
                url: file.url.clone(),
                sha256: file.sha256.clone(),
                size: file.size,
                install_layout: matcher.name().to_string(),
            };
            validate_binary_installer_metadata(&installer).is_ok()
                && validate_release_channel_binary_file_metadata(file).is_ok()
        })
        .min_by(|left, right| left.name.cmp(&right.name))
        .map(|file| BinaryInstaller {
            name: file.name.clone(),
            url: file.url.clone(),
            sha256: file.sha256.clone(),
            size: file.size,
            install_layout: matcher.name().to_string(),
        })
}

fn validate_release_channel_binary_file_metadata(file: &ReleaseChannelBinaryFile) -> Result<()> {
    if file.blake3.len() != 64 || !file.blake3.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("binary release file blake3 must be 64 hex characters");
    }
    Ok(())
}

fn deb_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    }
}

fn deb_graph_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    }
}

async fn fetch_release_update_check(
    layout: &InstallLayout,
    selected_channel: Option<&ResolvedReleaseChannelManifest>,
) -> Result<UpdateCheck> {
    if let Some(selection) = selected_channel {
        return fetch_selected_channel_update_check(layout, selection)
            .await
            .map(|(_, check)| check);
    }
    let manifest_url = release_manifest_url()?;
    let url = reqwest::Url::parse(&manifest_url)
        .with_context(|| format!("parse release manifest URL {manifest_url}"))?;
    let body = release_http_get_bytes(url, Some("application/json"), &manifest_url)
        .await
        .with_context(|| format!("read release manifest from {manifest_url}"))?;
    let channel_hash = channel_payload_hash(&body);
    update_check_from_release_payload(&body, layout, &manifest_url, Some(channel_hash))
}

async fn fetch_selected_channel_update_check(
    layout: &InstallLayout,
    selection: &ResolvedReleaseChannelManifest,
) -> Result<(Vec<u8>, UpdateCheck)> {
    let url = reqwest::Url::parse(&selection.url)
        .with_context(|| format!("parse release manifest URL {}", selection.url))?;
    let body = release_http_get_bytes(url, Some("application/json"), &selection.url)
        .await
        .with_context(|| format!("read release manifest from {}", selection.url))?;
    verify_selected_channel_manifest(selection, &body)?;
    let channel_hash = channel_payload_hash(&body);
    let mut check =
        update_check_from_release_payload(&body, layout, &selection.url, Some(channel_hash))?;
    let current = local_current_binary_version();
    check.update_available = check
        .latest_version
        .as_deref()
        .is_some_and(|latest| is_different_semver(latest, &current));
    if check.update_available && check.binary_installer.is_none() {
        check.binary_installer =
            binary_installer_from_release_payload(&body, layout, &selection.url)?;
    }
    Ok((body, check))
}

fn update_check_from_release_payload(
    body: &[u8],
    layout: &InstallLayout,
    manifest_url: &str,
    channel_hash: Option<String>,
) -> Result<UpdateCheck> {
    let current_binary = local_current_binary_version();
    if let Ok(graph) = serde_json::from_slice::<ReleaseGraphManifest>(body) {
        if !graph.packages.is_empty() || !graph.profiles.is_empty() {
            if !graph.profiles.is_empty() {
                let text = std::str::from_utf8(body)
                    .context("release graph manifest is not valid UTF-8")?;
                capsem_core::asset_manager::ManifestV2::from_json(text)
                    .context("validate release graph through the runtime manifest parser")?;
            }
            return update_check_from_release_graph_manifest(
                &graph,
                now_secs(),
                &current_binary,
                local_current_asset_version().as_deref(),
                local_current_profile_catalog_revision().as_deref(),
                layout,
                manifest_url,
                channel_hash,
            );
        }
    }
    let manifest: capsem_core::asset_manager::ManifestV2 = serde_json::from_slice(body)
        .with_context(|| format!("parse release manifest from {manifest_url}"))?;
    update_check_from_release_manifest(
        &manifest,
        now_secs(),
        &current_binary,
        local_current_asset_version().as_deref(),
        local_current_profile_catalog_revision().as_deref(),
        layout,
        manifest_url,
        channel_hash,
    )
}

fn binary_installer_from_release_payload(
    body: &[u8],
    layout: &InstallLayout,
    manifest_url: &str,
) -> Result<Option<BinaryInstaller>> {
    if let Ok(graph) = serde_json::from_slice::<ReleaseGraphManifest>(body) {
        if !graph.packages.is_empty() || !graph.profiles.is_empty() {
            return Ok(graph_binary_installer_for_layout(
                &graph.packages,
                layout,
                manifest_url,
            ));
        }
    }
    let manifest: capsem_core::asset_manager::ManifestV2 = serde_json::from_slice(body)
        .with_context(|| format!("parse release manifest from {manifest_url}"))?;
    let release = manifest.binaries.releases.get(&manifest.binaries.current);
    let files = binary_release_files_from_manifest(&manifest, release, manifest_url)?;
    Ok(binary_installer_for_layout(&files, layout))
}

async fn download_binary_installer(installer: &BinaryInstaller) -> Result<PathBuf> {
    validate_binary_installer_metadata(installer)?;
    let target = binary_installer_cache_path(installer)?;
    if target.exists() {
        verify_binary_installer_file(&target, installer)?;
        return Ok(target);
    }

    let url = reqwest::Url::parse(&installer.url)
        .with_context(|| format!("parse installer URL {}", installer.url))?;
    let bytes = release_http_get_bytes(url, None, &installer.url)
        .await
        .with_context(|| format!("read installer body from {}", installer.url))?;
    verify_binary_installer_bytes(&bytes, installer)?;

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create installer cache {}", parent.display()))?;
    }
    let tmp = target.with_extension("download.tmp");
    std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &target).with_context(|| format!("replace {}", target.display()))?;
    Ok(target)
}

fn binary_installer_cache_path(installer: &BinaryInstaller) -> Result<PathBuf> {
    validate_binary_installer_metadata(installer)?;
    Ok(crate::paths::capsem_home()?
        .join("updates")
        .join("installers")
        .join(&installer.name))
}

fn binary_installer_apply_plan(
    installer: &BinaryInstaller,
    path: &Path,
) -> Result<BinaryInstallerApplyPlan> {
    validate_binary_installer_metadata(installer)?;
    if !path.is_file() {
        anyhow::bail!("verified installer package is missing: {}", path.display());
    }
    let package_path = path.display().to_string();
    let commands = match installer.install_layout.as_str() {
        "macos_pkg" => {
            if !installer.name.ends_with(".pkg") {
                anyhow::bail!("macOS installer must be a .pkg file");
            }
            vec![BinaryInstallerApplyCommand {
                program: "sudo".to_string(),
                args: vec![
                    "/usr/sbin/installer".to_string(),
                    "-pkg".to_string(),
                    package_path,
                    "-target".to_string(),
                    "/".to_string(),
                ],
            }]
        }
        "linux_deb" => {
            if !installer.name.ends_with(".deb") {
                anyhow::bail!("Linux installer must be a .deb file");
            }
            vec![BinaryInstallerApplyCommand {
                program: "sudo".to_string(),
                args: vec![
                    "apt-get".to_string(),
                    "install".to_string(),
                    "--yes".to_string(),
                    "--allow-downgrades".to_string(),
                    package_path,
                ],
            }]
        }
        other => anyhow::bail!("unsupported binary installer layout {other}"),
    };
    Ok(BinaryInstallerApplyPlan { commands })
}

fn validate_binary_installer_metadata(installer: &BinaryInstaller) -> Result<()> {
    let parsed = reqwest::Url::parse(&installer.url)
        .with_context(|| format!("binary installer URL must be valid: {}", installer.url))?;
    if !matches!(parsed.scheme(), "https" | "http") {
        anyhow::bail!(
            "unsupported binary installer URL scheme {}: use https:// or http://",
            parsed.scheme()
        );
    }
    if !is_safe_installer_name(&installer.name) {
        anyhow::bail!("binary installer name must be a plain filename");
    }
    if installer.sha256.len() != 64 || !installer.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("binary installer sha256 must be 64 hex characters");
    }
    Ok(())
}

fn is_safe_installer_name(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name)
            .file_name()
            .and_then(|file_name| file_name.to_str())
            == Some(name)
        && !name.contains('\\')
}

fn verify_binary_installer_file(path: &Path, installer: &BinaryInstaller) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    verify_binary_installer_bytes(&bytes, installer)
}

fn verify_binary_installer_bytes(bytes: &[u8], installer: &BinaryInstaller) -> Result<()> {
    validate_binary_installer_metadata(installer)?;
    if bytes.len() as u64 != installer.size {
        anyhow::bail!(
            "binary installer size mismatch for {}: expected {}, got {}",
            installer.name,
            installer.size,
            bytes.len()
        );
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(&installer.sha256) {
        anyhow::bail!(
            "binary installer sha256 mismatch for {}: expected {}, got {}",
            installer.name,
            installer.sha256,
            actual
        );
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'+' | b':'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Compare versions: is `latest` newer than `current`?
/// Returns false for malformed versions (conservative: don't prompt for bad data).
fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

fn is_different_semver(candidate: &str, current: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(current),
    ) {
        (Ok(candidate), Ok(current)) => candidate != current,
        _ => false,
    }
}

/// Run the update flow.
///
/// With `assets = true`, refresh only the VM asset files referenced by the
/// locally-installed manifest. Binary updates download the matching verified
/// package and hand it to the platform package manager when `--yes` is set.
pub async fn run_update(
    yes: bool,
    check_only: bool,
    assets: bool,
    channel: Option<&str>,
    manifest_source: Option<&str>,
    corp_source: Option<&str>,
) -> Result<()> {
    let layout = platform::detect_install_layout();
    if check_only && (yes || assets || manifest_source.is_some() || corp_source.is_some()) {
        anyhow::bail!("--check cannot be combined with mutating update options");
    }
    if assets && corp_source.is_some() {
        anyhow::bail!(
            "--assets cannot be combined with --corp; use --manifest for corporate asset channels"
        );
    }
    if channel.is_some() && manifest_source.is_some() {
        anyhow::bail!("--channel cannot be combined with --manifest");
    }

    if let Some(channel) = channel {
        let assets_dir = capsem_core::asset_manager::default_assets_dir()
            .context("cannot resolve CAPSEM_HOME -- set $HOME or $CAPSEM_HOME")?;
        channel_transition_for_request(&assets_dir, Some(channel), None)?;
    }

    let selected_channel = match channel {
        Some(channel) => Some(resolve_release_channel_manifest(channel).await?),
        None => None,
    };

    let mut did_work = false;
    if let Some(source) = corp_source {
        provision_corp_config(source).await?;
        did_work = true;
    }

    if assets || manifest_source.is_some() {
        let selected_source = selected_channel
            .as_ref()
            .map(|selection| selection.url.as_str());
        if let Some(selection) = selected_channel.as_ref() {
            let (body, check) = fetch_selected_channel_update_check(&layout, selection).await?;
            refresh_assets(
                manifest_source.or(selected_source),
                Some(selection),
                Some(&body),
            )
            .await?;
            write_cache(&check).context("write selected channel status cache")?;
        } else {
            refresh_assets(manifest_source, None, None).await?;
        }
        return Ok(());
    }

    if did_work {
        return Ok(());
    }

    if layout == InstallLayout::Development {
        println!("Development build detected. Update from source with `git pull && just install`.");
        return Ok(());
    }

    let check = match fetch_release_update_check(&layout, selected_channel.as_ref()).await {
        Ok(check) => check,
        Err(error) => {
            if check_only || channel.is_some() {
                return Err(error).context("release channel check failed");
            }
            println!("Binary update check failed: {error:#}");
            println!("Run `capsem update --assets` to refresh VM assets, or retry later.");
            return Ok(());
        }
    };
    let _ = write_cache(&check);

    let current = local_current_binary_version();
    if check_only {
        print_update_check_summary(&check, &current, &layout);
        return Ok(());
    }

    let mut did_update = false;
    match check.latest_version.as_deref() {
        Some(latest) if check.update_available => {
            println!("Binary update available: {current} -> {latest}");
            if let Some(installer) = check.binary_installer.as_ref() {
                let mb = installer.size as f64 / 1_048_576.0;
                println!("Installer: {}", installer.url);
                println!("Package:   {} ({mb:.1} MB)", installer.name);
                println!("SHA-256:   {}", installer.sha256);
                if yes {
                    append_update_audit(serde_json::json!({
                        "event": "binary_update_start",
                        "action": "binary_update",
                        "outcome": "started",
                        "source": check.source.as_deref(),
                        "channel": check.source.as_deref().and_then(channel_from_source),
                        "old_version": current.as_str(),
                        "new_version": latest,
                        "package": {
                            "name": &installer.name,
                            "url": &installer.url,
                            "sha256": &installer.sha256,
                            "size": installer.size,
                            "layout": &installer.install_layout
                        }
                    }));
                    let update_result: Result<()> = async {
                        let path = download_binary_installer(installer).await?;
                        println!("Verified installer: {}", path.display());
                        let plan = binary_installer_apply_plan(installer, &path)?;
                        println!("Apply command:");
                        for command in plan.command_lines() {
                            println!("  {command}");
                        }
                        apply_binary_installer_plan(&plan).await?;
                        append_update_audit(serde_json::json!({
                            "event": "binary_update_complete",
                            "action": "binary_update",
                            "outcome": "success",
                            "source": check.source.as_deref(),
                            "channel": check.source.as_deref().and_then(channel_from_source),
                            "old_version": current.as_str(),
                            "new_version": latest,
                            "package": {
                                "name": &installer.name,
                                "url": &installer.url,
                                "sha256": &installer.sha256,
                                "size": installer.size,
                                "layout": &installer.install_layout,
                                "path": path.display().to_string()
                            }
                        }));
                        Ok(())
                    }
                    .await;
                    match update_result {
                        Ok(()) => {
                            println!("Binary update applied. Restart Capsem to use {latest}.");
                            did_update = true;
                        }
                        Err(error) => {
                            append_update_audit(serde_json::json!({
                                "event": "binary_update_failed",
                                "action": "binary_update",
                                "outcome": "failure",
                                "source": check.source.as_deref(),
                                "channel": check.source.as_deref().and_then(channel_from_source),
                                "old_version": current.as_str(),
                                "new_version": latest,
                                "package": {
                                    "name": &installer.name,
                                    "url": &installer.url,
                                    "sha256": &installer.sha256,
                                    "size": installer.size,
                                    "layout": &installer.install_layout
                                },
                                "error": format!("{error:#}")
                            }));
                            return Err(error);
                        }
                    }
                } else {
                    println!("Re-run with --yes to download and verify the installer package.");
                }
            } else {
                println!(
                    "No installer package in release manifest matches this install layout ({layout:?})."
                );
            }
        }
        Some(_) => println!("Capsem binary is current ({current})."),
        None => println!("Release channel did not advertise a binary version."),
    }

    if let Some(reason) = check.profiles_blocked_reason.as_deref() {
        println!("Profile catalog update blocked: {reason}.");
    } else if check.profiles_update_available {
        let current_profiles = check.current_profiles.as_deref().unwrap_or("unknown");
        let latest_profiles = check.latest_profiles.as_deref().unwrap_or("unknown");
        println!("Profile catalog update available: {current_profiles} -> {latest_profiles}");
        if yes {
            apply_profile_catalog_update(&check).await?;
            println!("Profile catalog update applied. New sessions will use {latest_profiles}.");
            did_update = true;
        } else {
            println!("Re-run with --yes to apply the profile catalog update.");
        }
    }

    if let Some(reason) = check.assets_blocked_reason.as_deref() {
        println!("VM asset update blocked: {reason}.");
    }

    if yes {
        if let Some(selection) = selected_channel.as_ref() {
            refresh_assets(Some(&selection.url), Some(selection), None).await?;
            println!(
                "Release channel switched to {} and its VM assets were verified.",
                selection.channel
            );
            did_update = true;
        }
    }

    if check.update_available || check.assets_update_available {
        println!("Run `capsem update --assets` separately to refresh VM assets.");
    }
    print_image_update_status(&check);

    let has_blocked_update = check.profiles_blocked_reason.is_some()
        || check.assets_blocked_reason.is_some()
        || check.images_blocked_reason.is_some();

    if !check.update_available
        && !check.profiles_update_available
        && !check.assets_update_available
        && !check.images_update_available
        && !has_blocked_update
    {
        println!("Capsem is current ({current}).");
    } else if !did_update && !check.update_available && !check.assets_update_available {
        if !has_blocked_update {
            println!("No local update action was needed.");
        } else if check.profiles_blocked_reason.is_some() {
            println!(
                "Capsem binary is current; profile catalog update requires a newer dependency."
            );
        } else {
            println!("Capsem binary is current; one or more update tracks are blocked.");
        }
    }
    Ok(())
}

fn print_update_check_summary(check: &UpdateCheck, current: &str, layout: &InstallLayout) {
    match check.latest_version.as_deref() {
        Some(latest) if check.update_available => {
            println!("Binary update available: {current} -> {latest}");
            if let Some(installer) = check.binary_installer.as_ref() {
                let mb = installer.size as f64 / 1_048_576.0;
                println!("Installer: {}", installer.url);
                println!("Package:   {} ({mb:.1} MB)", installer.name);
                println!("SHA-256:   {}", installer.sha256);
            } else {
                println!(
                    "No installer package in release manifest matches this install layout ({layout:?})."
                );
            }
        }
        Some(_) => println!("Capsem binary is current ({current})."),
        None => println!("Release channel did not advertise a binary version."),
    }

    if let Some(reason) = check.profiles_blocked_reason.as_deref() {
        println!("Profile catalog update blocked: {reason}.");
    } else if check.profiles_update_available {
        let current_profiles = check.current_profiles.as_deref().unwrap_or("unknown");
        let latest_profiles = check.latest_profiles.as_deref().unwrap_or("unknown");
        println!("Profile catalog update available: {current_profiles} -> {latest_profiles}");
    }

    print_asset_update_status(check);
    print_image_update_status(check);

    let has_blocked_update = check.profiles_blocked_reason.is_some()
        || check.assets_blocked_reason.is_some()
        || check.images_blocked_reason.is_some();

    if !check.update_available
        && !check.profiles_update_available
        && !check.assets_update_available
        && !check.images_update_available
        && !has_blocked_update
    {
        println!("Capsem is current ({current}).");
    }
}

fn print_asset_update_status(check: &UpdateCheck) {
    if let Some(reason) = check.assets_blocked_reason.as_deref() {
        println!("VM asset update blocked: {reason}.");
    } else if check.assets_update_available {
        let current_assets = check.current_assets.as_deref().unwrap_or("unknown");
        let latest_assets = check.latest_assets.as_deref().unwrap_or("unknown");
        println!("VM asset update available: {current_assets} -> {latest_assets}.");
    } else if check.latest_assets.is_some() && check.current_assets.is_none() {
        let latest_assets = check.latest_assets.as_deref().unwrap_or("unknown");
        println!(
            "VM asset state unknown: installed manifest not found; latest release is {latest_assets}."
        );
    }
}

fn print_image_update_status(check: &UpdateCheck) {
    if let Some(reason) = check.images_blocked_reason.as_deref() {
        println!("VM image update blocked: {reason}.");
    } else if check.images_update_available {
        let latest_images = check.latest_images.as_deref().unwrap_or("unknown");
        println!("VM image update available: {latest_images}.");
    } else if check.images_state.as_deref() == Some("not_published") {
        println!("VM image update track not published.");
    } else if let Some(latest_images) = check.latest_images.as_deref() {
        println!("VM image track latest: {latest_images}.");
    }
}

async fn apply_profile_catalog_update(check: &UpdateCheck) -> Result<()> {
    let source = check
        .profile_catalog_source
        .as_deref()
        .context("release channel did not advertise a profile catalog source")?;
    let expected_hash = check
        .profile_catalog_hash
        .as_deref()
        .context("release channel did not advertise a profile catalog hash")?;
    validate_blake3_hex("profile catalog hash", expected_hash)?;
    let channel_source = check
        .source
        .as_deref()
        .context("release channel update is missing its manifest source")?;
    let catalog_url = resolve_release_channel_artifact_url(channel_source, source)?;
    let bytes = read_profile_catalog_source(&catalog_url).await?;
    let actual_hash = blake3::hash(&bytes).to_hex().to_string();
    if actual_hash != expected_hash {
        anyhow::bail!(
            "profile catalog hash mismatch for {catalog_url}: expected {expected_hash}, got {actual_hash}"
        );
    }
    let document = parse_profile_catalog_document(&bytes, &catalog_url)?;
    let target_dir = crate::paths::capsem_home()?.join("profiles");
    install_profile_catalog_document(&target_dir, &document, &catalog_url, expected_hash)?;
    Ok(())
}

fn resolve_release_channel_artifact_url(channel_source: &str, artifact: &str) -> Result<String> {
    let trimmed = artifact.trim();
    if trimmed.is_empty() {
        anyhow::bail!("release channel profile catalog source is empty");
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("file://")
    {
        let parsed = reqwest::Url::parse(trimmed)
            .with_context(|| format!("parse profile catalog URL {trimmed}"))?;
        return Ok(parsed.to_string());
    }

    let base = reqwest::Url::parse(channel_source)
        .with_context(|| format!("parse release channel URL {channel_source}"))?;
    if trimmed.starts_with('/') {
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
    let url = reqwest::Url::parse(source)
        .with_context(|| format!("profile catalog source must be a URL, got {source}"))?;
    match url.scheme() {
        "file" => {
            if !has_scheme_authority_prefix(source, "file") {
                anyhow::bail!("profile catalog file URL must start with file://: {source}");
            }
            let path = url.to_file_path().map_err(|_| {
                anyhow::anyhow!("profile catalog file URL must be absolute: {source}")
            })?;
            std::fs::read(&path).with_context(|| format!("read profile catalog {}", path.display()))
        }
        "http" | "https" => {
            if !has_scheme_authority_prefix(source, url.scheme()) {
                anyhow::bail!(
                    "profile catalog source must use https://, http://, or file:// URLs, got {source}"
                );
            }
            release_http_get_bytes(url.clone(), Some("application/json"), source)
                .await
                .with_context(|| format!("read profile catalog body from {source}"))
        }
        scheme => anyhow::bail!(
            "unsupported profile catalog URL scheme {scheme}: use https://, http://, or file://"
        ),
    }
}

fn parse_profile_catalog_document(
    bytes: &[u8],
    source: &str,
) -> Result<PublishedProfileCatalogDocument> {
    let document: PublishedProfileCatalogDocument = serde_json::from_slice(bytes)
        .with_context(|| format!("parse profile catalog from {source}"))?;
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
    let revision =
        profile_catalog_revision(document.profiles.iter().collect::<Vec<_>>().as_slice())?;
    if revision != document.revision {
        anyhow::bail!(
            "profile catalog revision mismatch: document advertises {}, profiles resolve to {}",
            document.revision,
            revision
        );
    }
    Ok(document)
}

fn install_profile_catalog_document(
    target_dir: &Path,
    document: &PublishedProfileCatalogDocument,
    source: &str,
    hash: &str,
) -> Result<()> {
    let parent = target_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("profile catalog target has no parent"))?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let tmp_dir = parent.join(format!(".profiles.{unique}.tmp"));
    let backup_dir = parent.join(format!(".profiles.{unique}.backup"));
    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir)
            .with_context(|| format!("remove stale {}", tmp_dir.display()))?;
    }
    std::fs::create_dir(&tmp_dir).with_context(|| format!("create {}", tmp_dir.display()))?;
    let result = materialize_profile_catalog(&tmp_dir, document, source, hash)
        .and_then(|_| replace_profile_catalog_dir(target_dir, &tmp_dir, &backup_dir));
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
    result
}

fn materialize_profile_catalog(
    tmp_dir: &Path,
    document: &PublishedProfileCatalogDocument,
    source: &str,
    hash: &str,
) -> Result<()> {
    for profile in &document.profiles {
        let profile_dir = tmp_dir.join(&profile.id);
        std::fs::create_dir(&profile_dir)
            .with_context(|| format!("create {}", profile_dir.display()))?;
        let bytes = toml::to_string_pretty(profile)
            .with_context(|| format!("serialize profile {}", profile.id))?;
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

fn replace_profile_catalog_dir(target_dir: &Path, tmp_dir: &Path, backup_dir: &Path) -> Result<()> {
    if target_dir.exists() {
        if !target_dir.is_dir() {
            anyhow::bail!(
                "profile catalog target is not a directory: {}",
                target_dir.display()
            );
        }
        std::fs::rename(target_dir, backup_dir).with_context(|| {
            format!(
                "move existing profile catalog {} to {}",
                target_dir.display(),
                backup_dir.display()
            )
        })?;
        if let Err(error) = std::fs::rename(tmp_dir, target_dir) {
            let _ = std::fs::rename(backup_dir, target_dir);
            return Err(error)
                .with_context(|| format!("replace profile catalog {}", target_dir.display()));
        }
        let _ = std::fs::remove_dir_all(backup_dir);
    } else {
        std::fs::rename(tmp_dir, target_dir)
            .with_context(|| format!("install profile catalog {}", target_dir.display()))?;
    }
    Ok(())
}

fn validate_blake3_hex(field: &str, value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("{field} must be a 64-character BLAKE3 hex digest");
    }
    Ok(())
}

fn validate_hex_digest(value: &str, expected_len: usize, field: &str) -> Result<()> {
    if value.len() != expected_len || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("{field} must be a {expected_len}-character hex digest");
    }
    Ok(())
}

/// Pull any missing / hash-mismatched VM assets from the release URL.
async fn refresh_assets(
    manifest_source: Option<&str>,
    selected_channel: Option<&ResolvedReleaseChannelManifest>,
    selected_payload: Option<&[u8]>,
) -> Result<()> {
    let assets_dir = capsem_core::asset_manager::default_assets_dir()
        .context("cannot resolve CAPSEM_HOME -- set $HOME or $CAPSEM_HOME")?;
    let transition = channel_transition_for_request(
        &assets_dir,
        selected_channel.map(|selection| selection.channel.as_str()),
        if selected_channel.is_none() {
            manifest_source
        } else {
            None
        },
    )?;
    let refresh_source = if let Some(source) = manifest_source {
        Some(source.to_string())
    } else {
        remote_manifest_asset_source(&assets_dir)?
    };
    if let Some(source) = refresh_source {
        let previous = InstalledManifestSnapshot::capture(&assets_dir)?;
        let previous_state = installed_asset_audit_state(&assets_dir);
        append_update_audit(serde_json::json!({
            "event": "asset_update_start",
            "action": "asset_update",
            "outcome": "started",
            "source": source,
            "channel": channel_from_source(&source),
            "previous": previous_state
        }));
        let refresh_result: Result<()> = async {
            if let Some(selection) = selected_channel {
                let bytes = match selected_payload {
                    Some(bytes) => bytes.to_vec(),
                    None => read_manifest_source(&source).await?,
                };
                verify_selected_channel_manifest(selection, &bytes)?;
                install_manifest_bytes(&assets_dir, &source, &bytes).await?;
            } else {
                install_manifest_source(&assets_dir, &source).await?;
            }
            hydrate_installed_assets(&assets_dir).await?;
            persist_channel_transition(&assets_dir, &transition)?;
            Ok(())
        }
        .await;
        if let Err(error) = refresh_result {
            let _ = previous.restore(&assets_dir);
            append_update_audit(serde_json::json!({
                "event": "asset_update_failed",
                "action": "asset_update",
                "outcome": "failure",
                "source": source,
                "channel": channel_from_source(&source),
                "previous": previous_state,
                "current": installed_asset_audit_state(&assets_dir),
                "error": format!("{error:#}")
            }));
            return Err(error)
                .context("asset refresh failed; restored previous installed manifest");
        }
        let current_state = installed_asset_audit_state(&assets_dir);
        append_update_audit(serde_json::json!({
            "event": "asset_update_complete",
            "action": "asset_update",
            "outcome": "success",
            "source": source,
            "channel": channel_from_source(&source),
            "previous": previous_state,
            "current": current_state,
            "changed_fields": changed_asset_audit_fields(&previous_state, &current_state)
        }));
        return Ok(());
    }

    hydrate_installed_assets(&assets_dir).await
}

fn append_update_audit(mut event: serde_json::Value) {
    let now = now_secs();
    if let Some(object) = event.as_object_mut() {
        object.insert(
            "schema".to_string(),
            serde_json::Value::String("capsem.update_audit.v1".to_string()),
        );
        object.insert("timestamp".to_string(), serde_json::Value::from(now));
    }
    if let Err(error) = append_update_audit_inner(&event) {
        warn!("failed to write update audit log: {error:#}");
    }
}

fn append_update_audit_inner(event: &serde_json::Value) -> Result<()> {
    let home = crate::paths::capsem_home()?;
    let log_dir = home.join("logs");
    std::fs::create_dir_all(&log_dir).with_context(|| format!("create {}", log_dir.display()))?;
    let path = log_dir.join("update.log");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    serde_json::to_writer(&mut file, event).context("serialize update audit event")?;
    file.write_all(b"\n")
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn installed_asset_audit_state(assets_dir: &Path) -> serde_json::Value {
    let manifest_path = assets_dir.join("manifest.json");
    let metadata_path = assets_dir.join("manifest-metadata.json");
    let manifest_bytes = read_optional_file(&manifest_path).ok().flatten();
    let metadata = read_optional_file(&metadata_path)
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let manifest = manifest_bytes
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok());
    let manifest_sha256 = manifest_bytes.as_deref().map(sha256_hex);
    serde_json::json!({
        "source": metadata.as_ref().and_then(|value| value.get("manifest_url")).and_then(|value| value.as_str()),
        "origin": metadata.as_ref().and_then(|value| value.get("origin")).and_then(|value| value.as_str()),
        "channel": metadata.as_ref().and_then(|value| value.get("channel")).and_then(|value| value.as_str()),
        "channel_kind": metadata.as_ref().and_then(|value| value.get("channel_kind")).and_then(|value| value.as_str()),
        "channel_locked": metadata.as_ref().and_then(|value| value.get("channel_locked")).and_then(|value| value.as_bool()),
        "package_version": metadata.as_ref().and_then(|value| value.get("package_version")).and_then(|value| value.as_str()),
        "manifest_sha256": manifest_sha256,
        "asset_version": manifest.as_ref()
            .and_then(|value| value.get("assets"))
            .and_then(|value| value.get("current"))
            .and_then(|value| value.as_str()),
        "binary_version": manifest.as_ref()
            .and_then(|value| value.get("binaries"))
            .and_then(|value| value.get("current"))
            .and_then(|value| value.as_str())
    })
}

fn changed_asset_audit_fields(
    previous: &serde_json::Value,
    current: &serde_json::Value,
) -> Vec<&'static str> {
    [
        "source",
        "origin",
        "channel",
        "channel_kind",
        "channel_locked",
        "package_version",
        "manifest_sha256",
        "asset_version",
        "binary_version",
    ]
    .into_iter()
    .filter(|field| previous.get(field) != current.get(field))
    .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn channel_from_source(source: &str) -> Option<String> {
    let url = reqwest::Url::parse(source).ok()?;
    let segments: Vec<&str> = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
    for window in segments.windows(3) {
        if window[0] == "assets" && window[2] == "manifest.json" {
            return Some(window[1].to_string());
        }
    }
    if segments.last() == Some(&"manifest.json") {
        return segments
            .get(segments.len().saturating_sub(2))
            .map(|segment| (*segment).to_string());
    }
    None
}

fn installed_manifest_metadata(assets_dir: &Path) -> Result<Option<serde_json::Value>> {
    let path = assets_dir.join("manifest-metadata.json");
    let Some(bytes) = read_optional_file(&path)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", path.display()))
        .map(Some)
}

fn channel_transition_for_request(
    assets_dir: &Path,
    public_channel: Option<&str>,
    explicit_manifest: Option<&str>,
) -> Result<ChannelTransition> {
    let metadata = installed_manifest_metadata(assets_dir)?;
    let locked = metadata
        .as_ref()
        .and_then(|value| value.get("channel_locked"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let current_source = metadata
        .as_ref()
        .and_then(|value| value.get("manifest_url"))
        .and_then(serde_json::Value::as_str);

    if locked {
        if let Some(channel) = public_channel {
            anyhow::bail!(
                "installed corporate channel is locked; cannot switch to public channel {channel}"
            );
        }
        if let Some(source) = explicit_manifest {
            if current_source != Some(source) {
                anyhow::bail!(
                    "installed corporate channel is locked to {}; cannot switch to {source}",
                    current_source.unwrap_or("its configured manifest")
                );
            }
        }
        return Ok(ChannelTransition::Preserve);
    }

    if let Some(channel) = public_channel {
        return Ok(ChannelTransition::Public(channel.to_string()));
    }
    if let Some(source) = explicit_manifest {
        let package_hydration = metadata.as_ref().is_some_and(|value| {
            value.get("origin").and_then(serde_json::Value::as_str) == Some("package")
                && current_source == Some(source)
        });
        if package_hydration {
            return Ok(ChannelTransition::Preserve);
        }
        return Ok(ChannelTransition::Corporate);
    }
    Ok(ChannelTransition::Preserve)
}

fn persist_channel_transition(assets_dir: &Path, transition: &ChannelTransition) -> Result<()> {
    if matches!(transition, ChannelTransition::Preserve) {
        return Ok(());
    }
    let path = assets_dir.join("manifest-metadata.json");
    let mut metadata = installed_manifest_metadata(assets_dir)?
        .context("installed manifest metadata disappeared while persisting channel selection")?;
    let object = metadata
        .as_object_mut()
        .context("installed manifest metadata must be a JSON object")?;
    match transition {
        ChannelTransition::Public(channel) => {
            object.insert("channel".to_string(), serde_json::json!(channel));
            object.insert("channel_kind".to_string(), serde_json::json!("public"));
            object.insert("channel_locked".to_string(), serde_json::json!(false));
        }
        ChannelTransition::Corporate => {
            object.insert("channel".to_string(), serde_json::json!("corp"));
            object.insert("channel_kind".to_string(), serde_json::json!("corporate"));
            object.insert("channel_locked".to_string(), serde_json::json!(true));
        }
        ChannelTransition::Preserve => unreachable!(),
    }
    let bytes = serde_json::to_vec_pretty(&metadata).context("serialize manifest metadata")?;
    atomic_write(&path, &bytes)
}

async fn hydrate_installed_assets(assets_dir: &Path) -> Result<()> {
    let manifest_path = assets_dir.join("manifest.json");
    let manifest_bytes = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest = capsem_core::asset_manager::ManifestV2::from_json(&manifest_bytes)
        .with_context(|| format!("parse {}", manifest_path.display()))?;

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let binary_version = env!("CARGO_PKG_VERSION");

    println!("Refreshing VM assets into {}...", assets_dir.display());
    if let Some(local_source) = local_manifest_asset_source(assets_dir)? {
        println!("Using local asset source {}...", local_source.display());
        let copied = capsem_core::asset_manager::copy_missing_local_assets(
            &manifest,
            binary_version,
            arch,
            &local_source,
            assets_dir,
            |p| {
                if p.done {
                    let mb = p.bytes_done as f64 / 1_048_576.0;
                    println!("  {} ({:.1} MB)", p.logical_name, mb);
                }
            },
        )
        .context("local asset hydration failed")?;

        if copied.is_empty() {
            println!("All assets already up to date.");
        } else {
            println!("Refreshed {} asset(s).", copied.len());
        }
        return Ok(());
    }

    let downloaded = capsem_core::asset_manager::download_missing_assets(
        &manifest,
        binary_version,
        arch,
        assets_dir,
        |p| {
            if p.done {
                let mb = p.bytes_done as f64 / 1_048_576.0;
                println!("  {} ({:.1} MB)", p.logical_name, mb);
            }
        },
    )
    .await
    .context("asset download failed")?;

    if downloaded.is_empty() {
        println!("All assets already up to date.");
    } else {
        println!("Refreshed {} asset(s).", downloaded.len());
    }
    Ok(())
}

struct InstalledManifestSnapshot {
    manifest: Option<Vec<u8>>,
    metadata: Option<Vec<u8>>,
}

impl InstalledManifestSnapshot {
    fn capture(assets_dir: &Path) -> Result<Self> {
        Ok(Self {
            manifest: read_optional_file(&assets_dir.join("manifest.json"))?,
            metadata: read_optional_file(&assets_dir.join("manifest-metadata.json"))?,
        })
    }

    fn restore(&self, assets_dir: &Path) -> Result<()> {
        restore_optional_file(&assets_dir.join("manifest.json"), self.manifest.as_deref())?;
        restore_optional_file(
            &assets_dir.join("manifest-metadata.json"),
            self.metadata.as_deref(),
        )?;
        Ok(())
    }
}

async fn provision_corp_config(source: &str) -> Result<()> {
    let capsem_dir = crate::paths::capsem_home()?;
    capsem_core::net::policy_config::corp_provision::provision_from_source(&capsem_dir, source)
        .await
        .with_context(|| format!("provision corp config from {source}"))?;
    println!("Corp config updated from {source}.");
    Ok(())
}

fn manifest_from_release_channel_profile_graph(
    body: &str,
    arch: &str,
) -> Result<(
    capsem_core::asset_manager::ManifestV2,
    Vec<ReleaseChannelAssetDownload>,
    Vec<ReleaseChannelProfileConfigDownload>,
)> {
    let document: ReleaseChannelProfileManifest = serde_json::from_str(body)
        .context("failed to parse release channel profile manifest JSON")?;
    if document.profiles.is_empty() {
        anyhow::bail!("release channel profile manifest contains no profiles");
    }

    let mut primary: Option<(
        String,
        HashMap<String, capsem_core::asset_manager::AssetEntry>,
    )> = None;
    let mut downloads = Vec::new();
    let mut profile_config_downloads = Vec::new();

    for (profile_id, profile) in &document.profiles {
        if release_channel_status_is_revoked(&profile.status) {
            continue;
        }
        let Some(arch_images) = profile
            .architectures
            .iter()
            .find(|candidate| candidate.architecture == arch)
        else {
            continue;
        };
        let assets = profile_assets_from_release_channel_images(
            profile_id,
            &profile.revision,
            arch,
            &arch_images.artifacts,
        )?;
        let is_default = profile_id == "default";
        if primary.is_none() || is_default {
            primary = Some((profile.revision.clone(), assets.clone()));
        }
        for artifact in &arch_images.artifacts {
            if release_channel_status_is_revoked(&artifact.status) {
                continue;
            }
            if let Some(logical_name) = release_channel_image_logical_name(&artifact.kind) {
                validate_release_channel_digest(&artifact.digest)?;
                downloads.push(ReleaseChannelAssetDownload {
                    logical_name: logical_name.to_string(),
                    url: artifact.url.clone(),
                    size: artifact.size,
                    sha256: artifact.digest.sha256.clone(),
                    blake3: artifact.digest.blake3.clone(),
                });
            }
        }
        for config in &arch_images.config {
            if release_channel_status_is_revoked(&config.status) {
                continue;
            }
            validate_release_channel_digest(&config.digest)?;
            let profile_prefix = std::path::Path::new("profiles").join(profile_id);
            let relative_path = std::path::Path::new(&config.path)
                .strip_prefix(&profile_prefix)
                .with_context(|| {
                    format!(
                        "release channel profile config path {} must be under {}/",
                        config.path,
                        profile_prefix.display()
                    )
                })?
                .to_path_buf();
            if relative_path.as_os_str().is_empty()
                || relative_path
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                anyhow::bail!(
                    "release channel profile config path {} is not a safe relative path",
                    config.path
                );
            }
            profile_config_downloads.push(ReleaseChannelProfileConfigDownload {
                profile_id: profile_id.clone(),
                relative_path,
                url: config.url.clone(),
                size: config.size,
                sha256: config.digest.sha256.clone(),
                blake3: config.digest.blake3.clone(),
            });
        }
    }

    let Some((asset_version, arch_assets)) = primary else {
        anyhow::bail!("release channel profile manifest contains no complete {arch} image set");
    };
    let binary_version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = capsem_core::asset_manager::ManifestV2 {
        format: 2,
        refresh_policy: "24h".to_string(),
        asset_base: None,
        assets: capsem_core::asset_manager::AssetsSection {
            current: asset_version.clone(),
            releases: HashMap::from([(
                asset_version.clone(),
                capsem_core::asset_manager::AssetRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    min_binary: String::new(),
                    arches: HashMap::from([(arch.to_string(), arch_assets)]),
                },
            )]),
        },
        binaries: capsem_core::asset_manager::BinariesSection {
            current: binary_version.clone(),
            releases: HashMap::from([(
                binary_version.clone(),
                capsem_core::asset_manager::BinaryRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    min_assets: asset_version,
                    version: binary_version,
                    files: Vec::new(),
                },
            )]),
        },
    };
    let json = serde_json::to_string(&manifest).context("serialize converted asset manifest")?;
    capsem_core::asset_manager::ManifestV2::from_json(&json)
        .context("validate converted asset manifest")?;
    Ok((
        manifest,
        dedupe_release_channel_downloads(downloads),
        profile_config_downloads,
    ))
}

fn profile_assets_from_release_channel_images(
    profile_id: &str,
    revision: &str,
    arch: &str,
    artifacts: &[ReleaseChannelProfileImage],
) -> Result<HashMap<String, capsem_core::asset_manager::AssetEntry>> {
    let mut assets = HashMap::new();
    for artifact in artifacts {
        if release_channel_status_is_revoked(&artifact.status) {
            continue;
        }
        if artifact.name.trim().is_empty() {
            anyhow::bail!(
                "release channel profile {profile_id} revision {revision} architecture {arch} has an unnamed {} image",
                artifact.kind
            );
        }
        let Some(logical_name) = release_channel_image_logical_name(&artifact.kind) else {
            continue;
        };
        validate_release_channel_digest(&artifact.digest)?;
        assets.insert(
            logical_name.to_string(),
            capsem_core::asset_manager::AssetEntry {
                hash: artifact.digest.blake3.clone(),
                sha256: artifact.digest.sha256.clone(),
                size: artifact.size,
            },
        );
    }
    for required in ["vmlinuz", "initrd.img", "rootfs.erofs"] {
        if !assets.contains_key(required) {
            anyhow::bail!(
                "release channel profile {profile_id} revision {revision} architecture {arch} missing {required} image"
            );
        }
    }
    Ok(assets)
}

fn release_channel_image_logical_name(kind: &str) -> Option<&'static str> {
    match kind {
        "kernel" => Some("vmlinuz"),
        "initrd" => Some("initrd.img"),
        "rootfs" => Some("rootfs.erofs"),
        _ => None,
    }
}

fn release_channel_status_is_revoked(status: &str) -> bool {
    status.eq_ignore_ascii_case("revoked")
}

fn validate_release_channel_digest(digest: &ReleaseChannelProfileDigest) -> Result<()> {
    validate_blake3_hex("profile image blake3", &digest.blake3)?;
    if digest.sha256.len() != 64 || !digest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("profile image sha256 must be a 64-character hex digest");
    }
    Ok(())
}

fn dedupe_release_channel_downloads(
    downloads: Vec<ReleaseChannelAssetDownload>,
) -> Vec<ReleaseChannelAssetDownload> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for download in downloads {
        let key = (download.logical_name.clone(), download.blake3.clone());
        if seen.insert(key) {
            unique.push(download);
        }
    }
    unique.sort_by(|left, right| {
        left.logical_name
            .cmp(&right.logical_name)
            .then_with(|| left.url.cmp(&right.url))
    });
    unique
}

async fn install_release_channel_profile_manifest(
    assets_dir: &Path,
    source: &str,
    body: &str,
) -> Result<()> {
    let arch = capsem_core::asset_manager::host_manifest_arch();
    let (_, downloads, profile_config_downloads) =
        manifest_from_release_channel_profile_graph(body, arch)?;
    capsem_core::asset_manager::ManifestV2::from_json(body)
        .context("validate release graph through the runtime manifest parser")?;
    hydrate_release_channel_profile_assets(assets_dir, source, &downloads).await?;
    hydrate_release_channel_profile_configs(source, &profile_config_downloads).await?;

    std::fs::create_dir_all(assets_dir)
        .with_context(|| format!("cannot create {}", assets_dir.display()))?;
    atomic_write(&assets_dir.join("manifest.json"), body.as_bytes())?;
    write_installed_manifest_metadata(assets_dir, source, body.as_bytes())?;
    println!("Installed asset manifest from {source}.");
    Ok(())
}

async fn hydrate_release_channel_profile_configs(
    manifest_source: &str,
    downloads: &[ReleaseChannelProfileConfigDownload],
) -> Result<()> {
    if downloads.is_empty() {
        return Ok(());
    }

    let capsem_home = capsem_core::paths::capsem_home();
    std::fs::create_dir_all(&capsem_home)
        .with_context(|| format!("create {}", capsem_home.display()))?;
    let nonce = std::process::id();
    let stage = capsem_home.join(format!("profiles.installing.{nonce}"));
    let backup = capsem_home.join(format!("profiles.previous.{nonce}"));
    let profiles_dir = capsem_home.join("profiles");
    let _ = std::fs::remove_dir_all(&stage);
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::create_dir_all(&stage).with_context(|| format!("create {}", stage.display()))?;

    let install_result: Result<()> = async {
        let mut profile_ids = BTreeSet::new();
        for download in downloads {
            profile_ids.insert(download.profile_id.clone());
            let target = stage
                .join(&download.profile_id)
                .join(&download.relative_path);
            let parent = target
                .parent()
                .context("profile config target has no parent directory")?;
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
            let bytes = read_release_channel_profile_config(manifest_source, &download.url).await?;
            let actual_blake3 = blake3::hash(&bytes).to_hex().to_string();
            let actual_sha256 = sha256_hex(&bytes);
            if bytes.len() as u64 != download.size
                || actual_blake3 != download.blake3
                || !actual_sha256.eq_ignore_ascii_case(&download.sha256)
            {
                anyhow::bail!(
                    "profile config {} failed size or digest verification",
                    download.url
                );
            }
            atomic_write(&target, &bytes)?;
        }
        for profile_id in profile_ids {
            let profile_toml = stage.join(&profile_id).join("profile.toml");
            if !profile_toml.is_file() {
                anyhow::bail!(
                    "release channel profile {profile_id} has config payloads but no profile.toml"
                );
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = install_result {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(error);
    }

    if profiles_dir.exists() {
        std::fs::rename(&profiles_dir, &backup).with_context(|| {
            format!(
                "move existing profile catalog {} to {}",
                profiles_dir.display(),
                backup.display()
            )
        })?;
    }
    if let Err(error) = std::fs::rename(&stage, &profiles_dir) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &profiles_dir);
        }
        return Err(anyhow::Error::new(error).context(format!(
            "install hydrated profile catalog at {}",
            profiles_dir.display()
        )));
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

async fn read_release_channel_profile_config(
    manifest_source: &str,
    artifact_url: &str,
) -> Result<Vec<u8>> {
    let url = resolve_release_channel_artifact_url(manifest_source, artifact_url)?;
    let parsed = reqwest::Url::parse(&url)
        .with_context(|| format!("parse release channel profile config URL {url}"))?;
    match parsed.scheme() {
        "file" => {
            let path = parsed
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("profile config file URL must be absolute: {url}"))?;
            std::fs::read(&path).with_context(|| format!("read {}", path.display()))
        }
        "http" | "https" => release_http_get_bytes(parsed, None, &url)
            .await
            .with_context(|| format!("read profile config body from {url}")),
        scheme => anyhow::bail!(
            "unsupported profile config URL scheme {scheme}: use https://, http://, or file://"
        ),
    }
}

async fn hydrate_release_channel_profile_assets(
    assets_dir: &Path,
    source: &str,
    downloads: &[ReleaseChannelAssetDownload],
) -> Result<()> {
    if downloads.is_empty() {
        anyhow::bail!("release channel profile manifest contains no image artifacts");
    }
    let arch = capsem_core::asset_manager::host_manifest_arch();
    let arch_dir = assets_dir.join(arch);
    std::fs::create_dir_all(&arch_dir).with_context(|| format!("create {}", arch_dir.display()))?;

    for download in downloads {
        download_release_channel_profile_asset(&arch_dir, source, download).await?;
    }
    Ok(())
}

async fn download_release_channel_profile_asset(
    arch_dir: &Path,
    manifest_source: &str,
    download: &ReleaseChannelAssetDownload,
) -> Result<()> {
    validate_blake3_hex("profile image blake3", &download.blake3)?;
    let target = arch_dir.join(capsem_core::asset_manager::hash_filename(
        &download.logical_name,
        &download.blake3,
    ));
    if target.exists() {
        if verify_release_channel_asset_file(&target, download)
            .with_context(|| format!("verify existing profile image asset {}", target.display()))?
        {
            return Ok(());
        }
        let _ = std::fs::remove_file(&target);
    }

    let url = resolve_release_channel_artifact_url(manifest_source, &download.url)?;
    let parsed = reqwest::Url::parse(&url)
        .with_context(|| format!("parse release channel profile image URL {url}"))?;
    match parsed.scheme() {
        "file" => download_release_channel_profile_asset_from_file(&target, &parsed, download)
            .with_context(|| format!("copy profile image {}", download.url))?,
        "http" | "https" => {
            download_release_channel_profile_asset_from_http(&target, &url, download).await?
        }
        scheme => anyhow::bail!(
            "unsupported profile image URL scheme {scheme}: use https://, http://, or file://"
        ),
    }
    Ok(())
}

fn verify_release_channel_asset_file(
    path: &Path,
    download: &ReleaseChannelAssetDownload,
) -> Result<bool> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut blake3_hasher = blake3::Hasher::new();
    let mut sha256_hasher = Sha256::new();
    let mut bytes_done = 0u64;
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            break;
        }
        blake3_hasher.update(&buffer[..n]);
        sha256_hasher.update(&buffer[..n]);
        bytes_done += n as u64;
    }
    let actual_blake3 = blake3_hasher.finalize().to_hex().to_string();
    let actual_sha256 = format!("{:x}", sha256_hasher.finalize());
    Ok(bytes_done == download.size
        && actual_blake3 == download.blake3
        && actual_sha256.eq_ignore_ascii_case(&download.sha256))
}

fn download_release_channel_profile_asset_from_file(
    target: &Path,
    url: &reqwest::Url,
    download: &ReleaseChannelAssetDownload,
) -> Result<()> {
    use std::io::{Read, Write};

    let source_path = url.to_file_path().map_err(|_| {
        anyhow::anyhow!("profile image file URL must be absolute: {}", url.as_str())
    })?;
    let tmp = target.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut source = std::fs::File::open(&source_path)
        .with_context(|| format!("open {}", source_path.display()))?;
    let mut dest =
        std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    let mut blake3_hasher = blake3::Hasher::new();
    let mut sha256_hasher = Sha256::new();
    let mut bytes_done = 0u64;
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let n = source
            .read(&mut buffer)
            .with_context(|| format!("read {}", source_path.display()))?;
        if n == 0 {
            break;
        }
        dest.write_all(&buffer[..n])
            .with_context(|| format!("write {}", tmp.display()))?;
        blake3_hasher.update(&buffer[..n]);
        sha256_hasher.update(&buffer[..n]);
        bytes_done += n as u64;
    }
    dest.flush()
        .with_context(|| format!("flush {}", tmp.display()))?;
    drop(dest);
    finish_release_channel_asset_download(
        target,
        &tmp,
        download,
        bytes_done,
        blake3_hasher.finalize().to_hex().to_string(),
        format!("{:x}", sha256_hasher.finalize()),
    )
}

async fn download_release_channel_profile_asset_from_http(
    target: &Path,
    url: &str,
    download: &ReleaseChannelAssetDownload,
) -> Result<()> {
    let parsed =
        reqwest::Url::parse(url).with_context(|| format!("parse profile image URL {url}"))?;
    let bytes = release_http_get_bytes(parsed, None, url)
        .await
        .with_context(|| format!("read profile image body from {url}"))?;

    let tmp = target.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let bytes_done = bytes.len() as u64;
    let actual_blake3 = blake3::hash(&bytes).to_hex().to_string();
    let actual_sha256 = sha256_hex(&bytes);
    if let Err(error) = std::fs::write(&tmp, &bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::new(error).context(format!("write {}", tmp.display())));
    }

    finish_release_channel_asset_download(
        target,
        &tmp,
        download,
        bytes_done,
        actual_blake3,
        actual_sha256,
    )
}

fn finish_release_channel_asset_download(
    target: &Path,
    tmp: &Path,
    download: &ReleaseChannelAssetDownload,
    bytes_done: u64,
    actual_blake3: String,
    actual_sha256: String,
) -> Result<()> {
    if bytes_done != download.size {
        let _ = std::fs::remove_file(tmp);
        anyhow::bail!(
            "{}: size mismatch (expected {}, got {})",
            download.logical_name,
            download.size,
            bytes_done
        );
    }
    if actual_blake3 != download.blake3 {
        let _ = std::fs::remove_file(tmp);
        anyhow::bail!(
            "{}: hash mismatch (expected {}, got {})",
            download.logical_name,
            download.blake3,
            actual_blake3
        );
    }
    if !actual_sha256.eq_ignore_ascii_case(&download.sha256) {
        let _ = std::fs::remove_file(tmp);
        anyhow::bail!(
            "{}: sha256 mismatch (expected {}, got {})",
            download.logical_name,
            download.sha256,
            actual_sha256
        );
    }
    std::fs::rename(tmp, target)
        .with_context(|| format!("rename {} -> {}", tmp.display(), target.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o444));
    }
    Ok(())
}

async fn install_manifest_source(assets_dir: &std::path::Path, source: &str) -> Result<()> {
    let bytes = read_manifest_source(source).await?;
    install_manifest_bytes(assets_dir, source, &bytes).await
}

async fn install_manifest_bytes(
    assets_dir: &std::path::Path,
    source: &str,
    bytes: &[u8],
) -> Result<()> {
    let body = std::str::from_utf8(bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {source}"))?;
    let document: serde_json::Value =
        serde_json::from_str(body).with_context(|| format!("parse manifest JSON from {source}"))?;
    if document.get("format").is_none() && document.get("profiles").is_some() {
        install_release_channel_profile_manifest(assets_dir, source, body)
            .await
            .with_context(|| format!("install release channel profile graph from {source}"))?;
        return Ok(());
    }
    capsem_core::asset_manager::ManifestV2::from_json(body)
        .with_context(|| format!("parse format 2 manifest from {source}"))?;

    std::fs::create_dir_all(assets_dir)
        .with_context(|| format!("cannot create {}", assets_dir.display()))?;
    atomic_write(&assets_dir.join("manifest.json"), bytes)?;
    write_installed_manifest_metadata(assets_dir, source, bytes)?;
    println!("Installed asset manifest from {source}.");
    Ok(())
}

fn write_manifest_metadata(assets_dir: &Path, source: &str) -> Result<()> {
    let metadata_path = assets_dir.join("manifest-metadata.json");
    let mut metadata = read_manifest_metadata_value(&metadata_path)?.unwrap_or_else(|| {
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1"
        })
    });
    let object = metadata
        .as_object_mut()
        .context("manifest metadata must be a JSON object")?;
    object.insert(
        "schema".to_string(),
        serde_json::json!("capsem.manifest_metadata.v1"),
    );
    object.insert("origin".to_string(), serde_json::json!("update"));
    object.insert("manifest_url".to_string(), serde_json::json!(source));
    object.insert("refreshed_at".to_string(), serde_json::json!(now_secs()));
    object
        .entry("installed_at".to_string())
        .or_insert_with(|| serde_json::json!(now_secs()));
    let metadata_bytes = serde_json::to_vec_pretty(&metadata)?;
    atomic_write(&metadata_path, &metadata_bytes)?;
    Ok(())
}

fn write_installed_manifest_metadata(
    assets_dir: &Path,
    source: &str,
    manifest_bytes: &[u8],
) -> Result<()> {
    write_manifest_metadata(assets_dir, source)?;
    let check = update_check_from_release_payload(
        manifest_bytes,
        &platform::detect_install_layout(),
        source,
        Some(channel_payload_hash(manifest_bytes)),
    )
    .with_context(|| format!("derive installed manifest status from {source}"))?;
    write_cache_to_path(&assets_dir.join("manifest-metadata.json"), &check)
        .context("write installed manifest status")
}

async fn release_http_get_bytes(
    url: reqwest::Url,
    accept: Option<&'static str>,
    display_url: &str,
) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent("capsem")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .context("build release HTTP client")?;

    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=RELEASE_HTTP_ATTEMPTS {
        let mut request = client.get(url.clone());
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    match response.bytes().await {
                        Ok(bytes) => return Ok(bytes.to_vec()),
                        Err(error) => {
                            let error =
                                anyhow::Error::new(error).context(format!("read {display_url}"));
                            if attempt == RELEASE_HTTP_ATTEMPTS {
                                return Err(error);
                            }
                            warn!(
                                attempt,
                                max_attempts = RELEASE_HTTP_ATTEMPTS,
                                url = %display_url,
                                error = %error,
                                "release HTTP body read failed; retrying"
                            );
                            last_error = Some(error);
                        }
                    }
                } else if release_http_status_is_retryable(status) {
                    let error = anyhow::anyhow!("GET {} returned {}", display_url, status);
                    if attempt == RELEASE_HTTP_ATTEMPTS {
                        return Err(error);
                    }
                    warn!(
                        attempt,
                        max_attempts = RELEASE_HTTP_ATTEMPTS,
                        url = %display_url,
                        status = %status,
                        "release HTTP status is retryable"
                    );
                    last_error = Some(error);
                } else {
                    anyhow::bail!("GET {} returned {}", display_url, status);
                }
            }
            Err(error) => {
                let error = anyhow::Error::new(error).context(format!("GET {display_url}"));
                if attempt == RELEASE_HTTP_ATTEMPTS {
                    return Err(error);
                }
                warn!(
                    attempt,
                    max_attempts = RELEASE_HTTP_ATTEMPTS,
                    url = %display_url,
                    error = %error,
                    "release HTTP request failed; retrying"
                );
                last_error = Some(error);
            }
        }

        tokio::time::sleep(release_http_retry_backoff(attempt)).await;
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("GET {display_url} failed")))
}

fn release_http_status_is_retryable(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn release_http_retry_backoff(attempt: usize) -> Duration {
    let multiplier = 1u64 << attempt.saturating_sub(1).min(4);
    Duration::from_millis(RELEASE_HTTP_INITIAL_BACKOFF_MS * multiplier)
}

async fn read_manifest_source(source: &str) -> Result<Vec<u8>> {
    let url = reqwest::Url::parse(source).with_context(|| {
        format!("--manifest must be a URL: use https://..., http://..., or file:///absolute/path, got {source}")
    })?;
    match url.scheme() {
        "file" => {
            if !has_scheme_authority_prefix(source, "file") {
                anyhow::bail!("--manifest file URL must start with file://: {source}");
            }
            let path = url
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("--manifest file URL must be absolute: {source}"))?;
            std::fs::read(&path).with_context(|| format!("read manifest {}", path.display()))
        }
        "http" | "https" => {
            if !has_scheme_authority_prefix(source, url.scheme()) {
                anyhow::bail!(
                    "--manifest must use https://, http://, or file:// URLs, got {source}"
                );
            }
            release_http_get_bytes(url.clone(), Some("application/json"), source)
                .await
                .with_context(|| format!("read manifest body from {source}"))
        }
        scheme => anyhow::bail!(
            "unsupported --manifest URL scheme {scheme}: use https://, http://, or file://"
        ),
    }
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn restore_optional_file(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    if let Some(bytes) = bytes {
        atomic_write(path, bytes)
    } else if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
    } else {
        Ok(())
    }
}

fn local_manifest_asset_source(assets_dir: &std::path::Path) -> Result<Option<PathBuf>> {
    let metadata_path = assets_dir.join("manifest-metadata.json");
    if !metadata_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&metadata_path)
        .with_context(|| format!("read {}", metadata_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parse {}", metadata_path.display()))?;
    let Some(source) = value.get("manifest_url").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(None);
    }
    let parsed = reqwest::Url::parse(source).with_context(|| {
        format!(
            "asset manifest metadata source must be a URL: use https://..., http://..., or file:///absolute/path, got {source}"
        )
    })?;
    if parsed.scheme() != "file" {
        anyhow::bail!(
            "unsupported asset manifest metadata URL scheme {}: use https://, http://, or file://",
            parsed.scheme()
        );
    }
    if !has_scheme_authority_prefix(source, "file") {
        anyhow::bail!("asset manifest metadata file URL must start with file://: {source}");
    }
    let path = parsed.to_file_path().map_err(|_| {
        anyhow::anyhow!("asset manifest metadata file URL must be absolute: {source}")
    })?;
    if !path.is_file() {
        return Ok(None);
    }
    Ok(path.parent().map(|parent| parent.to_path_buf()))
}

fn remote_manifest_asset_source(assets_dir: &std::path::Path) -> Result<Option<String>> {
    let metadata_path = assets_dir.join("manifest-metadata.json");
    if !metadata_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&metadata_path)
        .with_context(|| format!("read {}", metadata_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parse {}", metadata_path.display()))?;
    let Some(source) = value.get("manifest_url").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    if !(source.starts_with("http://") || source.starts_with("https://")) {
        return Ok(None);
    }
    let parsed = reqwest::Url::parse(source).with_context(|| {
        format!(
            "asset manifest metadata source must be a URL: use https://..., http://..., or file:///absolute/path, got {source}"
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!(
            "unsupported asset manifest metadata URL scheme {}: use https://, http://, or file://",
            parsed.scheme()
        );
    }
    if !has_scheme_authority_prefix(source, parsed.scheme()) {
        anyhow::bail!(
            "asset manifest metadata must use https://, http://, or file:// URLs, got {source}"
        );
    }
    Ok(Some(source.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_semver() {
        assert!(is_newer("0.17.0", "0.16.1"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.16.1", "0.16.1"));
        assert!(!is_newer("0.16.0", "0.16.1"));
    }

    #[test]
    fn is_newer_rejects_garbage() {
        assert!(!is_newer("error", "0.16.1"));
        assert!(!is_newer("", "0.16.1"));
        assert!(!is_newer("not-a-version", "0.16.1"));
    }

    #[test]
    fn is_newer_rejects_malformed_current() {
        assert!(!is_newer("0.17.0", "garbage"));
    }

    #[test]
    fn is_newer_prerelease() {
        assert!(!is_newer("0.17.0-beta.1", "0.17.0"));
        assert!(is_newer("0.18.0-beta.1", "0.17.0"));
    }

    #[test]
    fn update_check_roundtrip() {
        let check = UpdateCheck {
            checked_at: 1718444400,
            latest_version: Some("0.17.0".into()),
            update_available: true,
            binary_installer: Some(BinaryInstaller {
                name: "Capsem-0.17.0.pkg".into(),
                url: "https://github.com/google/capsem/releases/download/v0.17.0/Capsem-0.17.0.pkg"
                    .into(),
                sha256: "abc123".into(),
                size: 123,
                install_layout: "macos_pkg".into(),
            }),
            latest_assets: Some("2030.0101.1".into()),
            current_assets: Some("2030.0101.0".into()),
            assets_update_available: true,
            assets_state: Some("published".into()),
            assets_blocked_reason: None,
            latest_profiles: Some("profiles-2030.0101.1".into()),
            current_profiles: Some("profiles-2030.0101.0".into()),
            profiles_update_available: false,
            profiles_state: Some("published".into()),
            profiles_blocked_reason: Some("requires binary 1.4.0 or newer".into()),
            profile_catalog_source: Some(
                "/profiles/releases/profiles-2030.0101.1/catalog.json".into(),
            ),
            profile_catalog_hash: Some("b".repeat(64)),
            latest_images: None,
            images_update_available: false,
            images_state: Some("not_published".into()),
            images_blocked_reason: None,
            source: Some("https://release.capsem.org/assets/stable/manifest.json".into()),
            channel_hash: Some("a".repeat(64)),
            validation_status: Some("valid".into()),
            validation_error: None,
        };
        let json = serde_json::to_string(&check).unwrap();
        let rt: UpdateCheck = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.latest_version, Some("0.17.0".into()));
        assert!(rt.update_available);
        assert_eq!(
            rt.binary_installer
                .as_ref()
                .map(|installer| installer.name.as_str()),
            Some("Capsem-0.17.0.pkg")
        );
        assert_eq!(rt.latest_assets, Some("2030.0101.1".into()));
        assert_eq!(rt.current_assets, Some("2030.0101.0".into()));
        assert!(rt.assets_update_available);
        assert_eq!(rt.assets_state, Some("published".into()));
        assert_eq!(rt.assets_blocked_reason, None);
        assert_eq!(rt.latest_profiles, Some("profiles-2030.0101.1".into()));
        assert_eq!(rt.current_profiles, Some("profiles-2030.0101.0".into()));
        assert!(!rt.profiles_update_available);
        assert_eq!(rt.profiles_state, Some("published".into()));
        assert_eq!(
            rt.profiles_blocked_reason,
            Some("requires binary 1.4.0 or newer".into())
        );
        assert_eq!(
            rt.profile_catalog_source,
            Some("/profiles/releases/profiles-2030.0101.1/catalog.json".into())
        );
        assert_eq!(rt.profile_catalog_hash, Some("b".repeat(64)));
        assert_eq!(rt.latest_images, None);
        assert!(!rt.images_update_available);
        assert_eq!(rt.images_state, Some("not_published".into()));
        assert_eq!(rt.images_blocked_reason, None);
        assert_eq!(
            rt.source,
            Some("https://release.capsem.org/assets/stable/manifest.json".into())
        );
        assert_eq!(rt.channel_hash, Some("a".repeat(64)));
        assert_eq!(rt.validation_status, Some("valid".into()));
        assert_eq!(rt.validation_error, None);
    }

    #[test]
    fn update_check_old_cache_shape_defaults_new_release_channel_fields() {
        let rt: UpdateCheck = serde_json::from_str(
            r#"{"checked_at":1718444400,"latest_version":"0.17.0","update_available":true}"#,
        )
        .unwrap();

        assert_eq!(rt.latest_version, Some("0.17.0".into()));
        assert!(rt.update_available);
        assert_eq!(rt.binary_installer, None);
        assert_eq!(rt.latest_assets, None);
        assert_eq!(rt.current_assets, None);
        assert!(!rt.assets_update_available);
        assert_eq!(rt.assets_state, None);
        assert_eq!(rt.assets_blocked_reason, None);
        assert_eq!(rt.latest_profiles, None);
        assert_eq!(rt.current_profiles, None);
        assert!(!rt.profiles_update_available);
        assert_eq!(rt.profiles_state, None);
        assert_eq!(rt.profiles_blocked_reason, None);
        assert_eq!(rt.profile_catalog_source, None);
        assert_eq!(rt.profile_catalog_hash, None);
        assert_eq!(rt.latest_images, None);
        assert!(!rt.images_update_available);
        assert_eq!(rt.images_state, None);
        assert_eq!(rt.images_blocked_reason, None);
        assert_eq!(rt.source, None);
        assert_eq!(rt.channel_hash, None);
        assert_eq!(rt.validation_status, None);
        assert_eq!(rt.validation_error, None);
    }

    #[test]
    fn cached_update_notice_reports_asset_only_updates() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
        let mut check = cached_notice_check();
        check.latest_assets = Some("2030.0101.1".into());
        check.current_assets = Some("2026.0627.1".into());
        check.assets_update_available = true;
        seed_manifest_metadata(&check);
        write_cache(&check).unwrap();

        assert_eq!(
            read_cached_update_notice().as_deref(),
            Some(
                "VM asset update available: 2030.0101.1. Run `capsem update --assets` to refresh."
            )
        );
    }

    #[test]
    fn cached_update_notice_reports_profile_catalog_updates() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
        let mut check = cached_notice_check();
        check.latest_profiles = Some("profiles-2030.0101.1".into());
        check.current_profiles = Some("profiles-2030.0101.0".into());
        check.profiles_update_available = true;
        seed_manifest_metadata(&check);
        write_cache(&check).unwrap();

        assert_eq!(
            read_cached_update_notice().as_deref(),
            Some(
                "Profile catalog update available: profiles-2030.0101.1. Run `capsem update` to refresh."
            )
        );
    }

    #[test]
    fn cached_update_notice_reports_blocked_profile_catalog_updates() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
        let mut check = cached_notice_check();
        check.latest_profiles = Some("profiles-2030.0101.1".into());
        check.current_profiles = Some("profiles-2030.0101.0".into());
        check.profiles_blocked_reason = Some("requires binary 1.4.1 or newer".into());
        seed_manifest_metadata(&check);
        write_cache(&check).unwrap();

        assert_eq!(
            read_cached_update_notice().as_deref(),
            Some(
                "Profile catalog update blocked: requires binary 1.4.1 or newer. Run `capsem update --check` for details."
            )
        );
    }
    #[test]
    fn update_check_merges_into_single_manifest_metadata_file() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
        let assets = home.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let path = assets.join("manifest-metadata.json");
        assert_eq!(manifest_metadata_path().as_deref(), Some(path.as_path()));
        std::fs::write(
            &path,
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
                "installed_at": 100,
                "package_version": "1.5.0"
            })
            .to_string(),
        )
        .unwrap();

        let check = cached_notice_check();
        write_cache(&check).unwrap();

        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(metadata["schema"], "capsem.manifest_metadata.v1");
        assert_eq!(metadata["origin"], "package");
        assert_eq!(metadata["installed_at"], 100);
        assert_eq!(metadata["package_version"], "1.5.0");
        assert_eq!(
            metadata["checked_url"],
            check.source.unwrap(),
            "metadata={metadata}"
        );
        assert_eq!(metadata["checked_at"], check.checked_at);
    }

    #[test]
    fn single_manifest_metadata_records_only_the_latest_channel_check() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");

        let mut stable = cached_notice_check();
        stable.source = Some("https://release.capsem.org/assets/stable/manifest.json".into());
        write_cache(&stable).unwrap();
        let mut nightly = cached_notice_check();
        nightly.source = Some("https://release.capsem.org/assets/nightly/manifest.json".into());
        write_cache(&nightly).unwrap();

        assert!(read_cache_for_source(stable.source.as_deref().unwrap()).is_err());
        assert_eq!(
            read_cache_for_source(nightly.source.as_deref().unwrap())
                .unwrap()
                .source,
            nightly.source
        );
    }

    #[test]
    fn stable_to_nightly_manifest_switch_resolves_nightly_updates() {
        let stable_source = "https://release.capsem.org/assets/stable/manifest.json";
        let nightly_source = "https://release.capsem.org/assets/nightly/manifest.json";
        let stable = test_manifest("1.4.0", "2026.0627.8", "1.4.0", "2026.0627.8");
        let nightly = test_manifest(
            "1.5.0-nightly.20260702",
            "2026.0702.1-nightly",
            "1.4.0",
            "2026.0627.8",
        );

        let stable_check = update_check_from_release_manifest(
            &stable,
            100,
            "1.4.0",
            Some("2026.0627.8"),
            None,
            &InstallLayout::MacosPkg,
            stable_source,
            Some("stable-channel-hash".into()),
        )
        .expect("stable check");
        let nightly_check = update_check_from_release_manifest(
            &nightly,
            200,
            "1.4.0",
            Some("2026.0627.8"),
            None,
            &InstallLayout::MacosPkg,
            nightly_source,
            Some("nightly-channel-hash".into()),
        )
        .expect("nightly check");

        assert_eq!(stable_check.source.as_deref(), Some(stable_source));
        assert_eq!(stable_check.latest_version.as_deref(), Some("1.4.0"));
        assert!(!stable_check.update_available);
        assert!(!stable_check.assets_update_available);
        assert_eq!(
            stable_check.channel_hash.as_deref(),
            Some("stable-channel-hash")
        );

        assert_eq!(nightly_check.source.as_deref(), Some(nightly_source));
        assert_eq!(
            nightly_check.latest_version.as_deref(),
            Some("1.5.0-nightly.20260702")
        );
        assert!(nightly_check.update_available);
        assert_eq!(
            nightly_check.latest_assets.as_deref(),
            Some("2026.0702.1-nightly")
        );
        assert!(nightly_check.assets_update_available);
        assert_eq!(
            nightly_check.channel_hash.as_deref(),
            Some("nightly-channel-hash")
        );
        assert_eq!(
            nightly_check
                .binary_installer
                .as_ref()
                .map(|installer| installer.name.as_str()),
            Some("Capsem-1.5.0-nightly.20260702.pkg")
        );
    }

    fn test_manifest(
        binary_version: &str,
        asset_version: &str,
        min_binary: &str,
        min_assets: &str,
    ) -> capsem_core::asset_manager::ManifestV2 {
        capsem_core::asset_manager::ManifestV2::from_json(&format!(
            r#"{{
                "format": 2,
                "refresh_policy": "24h",
                "asset_base": "https://github.com/google/capsem/releases/download/v{binary_version}/",
                "assets": {{
                    "current": "{asset_version}",
                    "releases": {{
                        "{asset_version}": {{
                            "date": "2026-07-02",
                            "deprecated": false,
                            "min_binary": "{min_binary}",
                            "arches": {{}}
                        }}
                    }}
                }},
                "binaries": {{
                    "current": "{binary_version}",
                    "releases": {{
                        "{binary_version}": {{
                            "date": "2026-07-02",
                            "deprecated": false,
                            "min_assets": "{min_assets}",
                            "version": "{binary_version}",
                            "files": [
                                {{
                                    "name": "Capsem-{binary_version}.pkg",
                                    "size": 42,
                                    "sha256": "{}",
                                    "blake3": "{}"
                                }}
                            ]
                        }}
                    }}
                }}
            }}"#,
            "a".repeat(64),
            "b".repeat(64)
        ))
        .expect("test manifest")
    }

    fn cached_notice_check() -> UpdateCheck {
        UpdateCheck {
            checked_at: now_secs(),
            latest_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            update_available: false,
            binary_installer: None,
            latest_assets: Some("2026.0627.1".into()),
            current_assets: Some("2026.0627.1".into()),
            assets_update_available: false,
            assets_state: Some("published".into()),
            assets_blocked_reason: None,
            latest_profiles: Some("profiles-2030.0101.1".into()),
            current_profiles: Some("profiles-2030.0101.0".into()),
            profiles_update_available: false,
            profiles_state: Some("published".into()),
            profiles_blocked_reason: Some("requires binary 1.4.1 or newer".into()),
            profile_catalog_source: Some(
                "/profiles/releases/profiles-2030.0101.1/catalog.json".into(),
            ),
            profile_catalog_hash: Some("b".repeat(64)),
            latest_images: None,
            images_update_available: false,
            images_state: Some("not_published".into()),
            images_blocked_reason: None,
            source: Some("https://release.capsem.org/assets/stable/manifest.json".into()),
            channel_hash: Some("a".repeat(64)),
            validation_status: Some("valid".into()),
            validation_error: None,
        }
    }

    fn seed_manifest_metadata(check: &UpdateCheck) {
        let path = manifest_metadata_path().expect("manifest metadata path");
        std::fs::create_dir_all(path.parent().expect("metadata parent")).unwrap();
        std::fs::write(
            path,
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "manifest_url": check.source.as_deref().expect("check source"),
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn update_channel_provenance_preserves_previous_cache_on_failure() {
        let previous = UpdateCheck {
            checked_at: 1000,
            latest_version: Some("99.99.99".into()),
            update_available: true,
            binary_installer: None,
            latest_assets: Some("2030.0101.1".into()),
            current_assets: Some("2030.0101.0".into()),
            assets_update_available: true,
            assets_state: Some("published".into()),
            assets_blocked_reason: None,
            latest_profiles: None,
            current_profiles: None,
            profiles_update_available: false,
            profiles_state: None,
            profiles_blocked_reason: None,
            profile_catalog_source: None,
            profile_catalog_hash: None,
            latest_images: None,
            images_update_available: false,
            images_state: None,
            images_blocked_reason: None,
            source: Some("https://release.capsem.org/assets/stable/manifest.json".into()),
            channel_hash: Some("f".repeat(64)),
            validation_status: Some("valid".into()),
            validation_error: None,
        };

        let check = failed_update_check_from_previous(
            Some(previous),
            1200,
            "https://release.capsem.org/assets/stable/manifest.json",
            "fetch_error",
            "connection refused".to_string(),
        );

        assert_eq!(check.checked_at, 1200);
        assert_eq!(check.latest_version, Some("99.99.99".into()));
        assert_eq!(check.latest_assets, Some("2030.0101.1".into()));
        assert_eq!(check.current_assets, Some("2030.0101.0".into()));
        assert_eq!(check.channel_hash, Some("f".repeat(64)));
        assert_eq!(check.validation_status, Some("fetch_error".into()));
        assert_eq!(check.validation_error, Some("connection refused".into()));
    }

    #[test]
    fn cache_ttl_constant() {
        assert_eq!(CACHE_TTL_SECS, 86400);
    }

    #[test]
    fn update_does_not_fetch_health_for_manifest_url() {
        assert_eq!(
            release_manifest_url_from_manifest_url(
                "https://release.capsem.org/assets/stable/manifest.json"
            ),
            Some("https://release.capsem.org/assets/stable/manifest.json".to_string())
        );
        assert_eq!(
            release_manifest_url_from_manifest_url(
                "https://corp.example/capsem/assets/internal/manifest.json"
            ),
            Some("https://corp.example/capsem/assets/internal/manifest.json".to_string())
        );
        assert_eq!(
            release_manifest_url_from_manifest_url("file:///tmp/assets/stable/manifest.json"),
            None
        );
    }

    #[test]
    fn installed_update_source_requires_manifest_metadata() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
        let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");

        let error = release_manifest_url_for_layout(&InstallLayout::UserDir)
            .expect_err("installed Capsem must not silently select stable");

        assert!(
            format!("{error:#}").contains("manifest-metadata.json"),
            "{error:#}"
        );
    }

    #[test]
    fn installed_update_source_rejects_malformed_manifest_metadata() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
        let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
        let assets = home.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("manifest-metadata.json"), b"not json\n").unwrap();

        let error = release_manifest_url_for_layout(&InstallLayout::MacosPkg)
            .expect_err("malformed installed metadata must fail closed");

        assert!(format!("{error:#}").contains("parse"), "{error:#}");
    }

    #[test]
    fn installed_update_source_requires_manifest_url_field() {
        let _lock = crate::lock_test_env();
        let assets = tempfile::tempdir().unwrap();
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
        let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
        std::fs::write(
            assets.path().join("manifest-metadata.json"),
            br#"{"schema":"capsem.manifest_metadata.v1"}"#,
        )
        .unwrap();

        let error = release_manifest_url_for_layout(&InstallLayout::LinuxDeb)
            .expect_err("installed metadata without manifest_url must fail closed");

        assert!(format!("{error:#}").contains("manifest_url"), "{error:#}");
    }

    #[test]
    fn installed_update_source_rejects_wrong_metadata_schema() {
        let _lock = crate::lock_test_env();
        let assets = tempfile::tempdir().unwrap();
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
        let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
        std::fs::write(
            assets.path().join("manifest-metadata.json"),
            br#"{"schema":"capsem.wrong.v1","manifest_url":"https://release.capsem.org/assets/nightly/manifest.json"}"#,
        )
        .unwrap();

        let error = release_manifest_url_for_layout(&InstallLayout::MacosPkg)
            .expect_err("wrong metadata schema must fail closed");

        assert!(
            format!("{error:#}").contains("capsem.manifest_metadata.v1"),
            "{error:#}"
        );
    }

    #[test]
    fn installed_update_source_does_not_replace_file_manifest_with_stable() {
        let _lock = crate::lock_test_env();
        let assets = tempfile::tempdir().unwrap();
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
        let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
        std::fs::write(
            assets.path().join("manifest-metadata.json"),
            br#"{"schema":"capsem.manifest_metadata.v1","manifest_url":"file:///tmp/release/assets/nightly/manifest.json"}"#,
        )
        .unwrap();

        let error = release_manifest_url_for_layout(&InstallLayout::UserDir)
            .expect_err("local manifest provenance must not silently become stable");

        let message = format!("{error:#}");
        assert!(message.contains("http(s)"), "{message}");
        assert!(!message.contains(DEFAULT_RELEASE_MANIFEST_URL), "{message}");
    }

    #[test]
    fn installed_update_source_uses_exact_metadata_url_and_assets_override() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let assets = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
        let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
        let metadata_path = assets.path().join("manifest-metadata.json");
        let nightly = "https://release.capsem.org/assets/nightly/manifest.json";
        std::fs::write(
            &metadata_path,
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "manifest_url": nightly,
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            manifest_metadata_path().as_deref(),
            Some(metadata_path.as_path())
        );
        assert_eq!(
            release_manifest_url_for_layout(&InstallLayout::MacosPkg).unwrap(),
            nightly
        );
        assert!(!home.path().join("assets/manifest-metadata.json").exists());
    }

    #[test]
    fn installed_update_source_rejects_environment_channel_bypass() {
        let _lock = crate::lock_test_env();
        let assets = tempfile::tempdir().unwrap();
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
        let _manifest_override = EnvGuard::set(
            RELEASE_MANIFEST_URL_ENV,
            "https://release.capsem.org/assets/stable/manifest.json",
        );
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
        std::fs::write(
            assets.path().join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "manifest_url": "https://corp.example/capsem/assets/internal/manifest.json",
            })
            .to_string(),
        )
        .unwrap();

        let error = release_manifest_url_for_layout(&InstallLayout::UserDir)
            .expect_err("installed environment must not bypass corporate provenance");

        let message = format!("{error:#}");
        assert!(message.contains(RELEASE_MANIFEST_URL_ENV), "{message}");
        assert!(message.contains("installed"), "{message}");
    }

    #[test]
    fn development_update_source_accepts_explicit_environment_url() {
        let _lock = crate::lock_test_env();
        let assets = tempfile::tempdir().unwrap();
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
        let nightly = "https://release.capsem.org/assets/nightly/manifest.json";
        let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, nightly);
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");

        assert_eq!(
            release_manifest_url_for_layout(&InstallLayout::Development).unwrap(),
            nightly
        );
    }

    #[test]
    fn development_update_source_may_default_to_stable_without_metadata() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
        let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
        let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");

        assert_eq!(
            release_manifest_url_for_layout(&InstallLayout::Development).unwrap(),
            DEFAULT_RELEASE_MANIFEST_URL
        );
    }

    fn channel_catalog_fixture() -> ReleaseChannelsCatalog {
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "channels": {
                "nightly": {
                    "manifests": [
                        {
                            "version": "1.5.0-nightly.20260702",
                            "status": "current",
                            "url": "/assets/nightly/manifest.json",
                            "digest": {
                                "sha256": "a".repeat(64),
                                "blake3": "b".repeat(64),
                                "hmac": "nightly-current-hmac"
                            },
                            "min_capsem_version": "1.5.0"
                        },
                        {
                            "version": "1.4.9-nightly.20260701",
                            "status": "supported",
                            "url": "/assets/nightly/1.4/manifest.json",
                            "digest": {
                                "sha256": "c".repeat(64),
                                "blake3": "d".repeat(64),
                                "hmac": "nightly-supported-hmac"
                            },
                            "min_capsem_version": "1.4.0",
                            "max_capsem_version": "1.4.99"
                        },
                        {
                            "version": "1.3.0-nightly.revoked",
                            "status": "revoked",
                            "url": "/assets/nightly/revoked/manifest.json",
                            "digest": {
                                "sha256": "e".repeat(64),
                                "blake3": "f".repeat(64),
                                "hmac": "nightly-revoked-hmac"
                            }
                        }
                    ]
                }
            }
        }))
        .expect("channel catalog fixture")
    }

    #[test]
    fn channel_manifest_resolution_never_selects_revoked_manifest() {
        let catalog = channel_catalog_fixture();

        let selected =
            select_channel_manifest_url(&catalog, "nightly", "1.4.12").expect("selection");

        assert_ne!(selected, "/assets/nightly/revoked/manifest.json");
        assert_eq!(selected, "/assets/nightly/1.4/manifest.json");
    }

    #[test]
    fn channel_manifest_resolution_old_capsem_selects_compatible_supported_manifest() {
        let catalog = channel_catalog_fixture();

        let selected =
            select_channel_manifest_url(&catalog, "nightly", "1.4.12").expect("selection");

        assert_eq!(selected, "/assets/nightly/1.4/manifest.json");
    }

    #[test]
    fn channel_manifest_resolution_requires_digest_shape() {
        let catalog: ReleaseChannelsCatalog = serde_json::from_value(serde_json::json!({
            "version": 1,
            "channels": {
                "stable": {
                    "manifests": [
                        {
                            "version": "1.4.0",
                            "status": "current",
                            "url": "/assets/stable/manifest.json",
                            "digest": {
                                "sha256": "abc123",
                                "blake3": "b".repeat(64)
                            }
                        }
                    ]
                }
            }
        }))
        .expect("bad catalog parses before validation");

        let error = select_channel_manifest_url(&catalog, "stable", "1.4.0")
            .expect_err("bad digest shape rejected");

        assert!(format!("{error:#}").contains("sha256"), "{error:#}");
    }

    #[test]
    fn selected_channel_manifest_verification_rejects_payload_substitution() {
        let bytes = br#"{"channel":"stable"}"#;
        let selection = ResolvedReleaseChannelManifest {
            channel: "stable".to_string(),
            url: "https://release.capsem.org/assets/stable/manifest.json".to_string(),
            sha256: sha256_hex(bytes),
            blake3: blake3::hash(bytes).to_hex().to_string(),
        };

        verify_selected_channel_manifest(&selection, bytes).expect("matching payload");
        let error = verify_selected_channel_manifest(&selection, br#"{"channel":"nightly"}"#)
            .expect_err("substituted payload must fail closed");
        assert!(format!("{error:#}").contains("SHA-256 mismatch"));
    }

    #[test]
    fn release_manifest_url_env_rejects_bare_paths() {
        let err =
            validate_release_manifest_url("/tmp/release/assets/stable/manifest.json").unwrap_err();
        assert!(
            format!("{err:#}").contains("CAPSEM_RELEASE_MANIFEST_URL must be a URL"),
            "{err:#}"
        );
    }

    #[test]
    fn update_source_url_flags_are_url_only() {
        for flag in ["--manifest", "--corp"] {
            for source in [
                "https://release.capsem.org/assets/stable/manifest.json",
                "http://127.0.0.1:8080/assets/stable/manifest.json",
                "file:///tmp/capsem/assets/stable/manifest.json",
            ] {
                assert_eq!(
                    validate_source_url_arg(flag, source),
                    Ok(source.to_string()),
                    "{flag} should accept {source}"
                );
            }

            for source in [
                "/tmp/capsem/assets/stable/manifest.json",
                "assets/stable/manifest.json",
                "file:assets/stable/manifest.json",
                "file://relative/manifest.json",
                "ssh://updates.example/assets/stable/manifest.json",
                "https:release.capsem.org/assets/stable/manifest.json",
            ] {
                let err =
                    validate_source_url_arg(flag, source).expect_err("source should be rejected");
                assert!(
                    err.contains(flag),
                    "error for {source} should mention {flag}: {err}"
                );
            }
        }
    }

    #[test]
    fn release_graph_update_check_selects_linux_deb_package() {
        let package_name = format!("Capsem_2.0.0_{}.deb", deb_arch());
        let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
            "version": "1.0.0",
            "channel": "nightly",
            "packages": [
                {
                    "name": "Capsem_2.0.0_wrong.deb",
                    "url": "/releases/download/v2.0.0/Capsem_2.0.0_wrong.deb",
                    "version": "2.0.0",
                    "kind": "debian_package",
                    "platform": "linux",
                    "architecture": "wrong",
                    "status": "current",
                    "bytes": 111,
                    "digest": {"sha256": "1".repeat(64), "blake3": "a".repeat(64)}
                },
                {
                    "name": package_name,
                    "url": format!("/releases/download/v2.0.0/{package_name}"),
                    "version": "2.0.0",
                    "kind": "debian_package",
                    "platform": "linux",
                    "architecture": deb_graph_arch(),
                    "status": "current",
                    "bytes": 222,
                    "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
                },
                {
                    "name": "Capsem-2.0.0.pkg",
                    "url": "https://github.com/google/capsem/releases/download/v2.0.0/Capsem-2.0.0.pkg",
                    "version": "2.0.0",
                    "kind": "macos_pkg",
                    "platform": "macos",
                    "architecture": "arm64",
                    "status": "current",
                    "bytes": 333,
                    "digest": {"sha256": "3".repeat(64), "blake3": "c".repeat(64)}
                }
            ],
            "profiles": {
                "code": {
                    "revision": "profiles-2026.0709.7",
                    "status": "current",
                    "architectures": [
                        {
                            "architecture": deb_graph_arch(),
                            "image_revision": "2026.0709.7",
                            "images": []
                        }
                    ]
                }
            }
        }))
        .unwrap();

        let check = update_check_from_release_graph_manifest(
            &graph,
            1718444400,
            "1.5.0",
            Some("2026.0709.6"),
            Some("profiles-2026.0709.6"),
            &InstallLayout::LinuxDeb,
            "http://127.0.0.1:33773/assets/nightly/manifest.json",
            Some("f".repeat(64)),
        )
        .unwrap();

        assert_eq!(check.latest_version, Some("2.0.0".to_string()));
        assert!(check.update_available);
        let installer = check.binary_installer.as_ref().unwrap();
        assert_eq!(installer.name, package_name);
        assert_eq!(
            installer.url,
            format!("http://127.0.0.1:33773/releases/download/v2.0.0/{package_name}")
        );
        assert_eq!(installer.sha256, "2".repeat(64));
        assert_eq!(installer.size, 222);
        assert_eq!(installer.install_layout, "linux_deb");
        assert_eq!(check.latest_assets, Some("2026.0709.7".to_string()));
        assert!(check.assets_update_available);
        assert_eq!(
            check.latest_profiles,
            Some("profiles-2026.0709.7".to_string())
        );
        assert_eq!(check.channel_hash, Some("f".repeat(64)));
        assert_eq!(check.validation_status, Some("valid".to_string()));
    }

    #[test]
    fn release_graph_update_check_selects_macos_pkg_package() {
        let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
            "version": "1.0.0",
            "channel": "stable",
            "packages": [
                {
                    "name": "Capsem-2.0.0.pkg",
                    "url": "https://github.com/google/capsem/releases/download/v2.0.0/Capsem-2.0.0.pkg",
                    "version": "2.0.0",
                    "kind": "macos_pkg",
                    "platform": "macos",
                    "architecture": "arm64",
                    "status": "current",
                    "bytes": 333,
                    "digest": {"sha256": "3".repeat(64), "blake3": "c".repeat(64)}
                },
                {
                    "name": format!("Capsem_2.0.0_{}.deb", deb_arch()),
                    "url": format!(
                        "https://github.com/google/capsem/releases/download/v2.0.0/Capsem_2.0.0_{}.deb",
                        deb_arch()
                    ),
                    "version": "2.0.0",
                    "kind": "debian_package",
                    "platform": "linux",
                    "architecture": deb_graph_arch(),
                    "status": "current",
                    "bytes": 222,
                    "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
                }
            ],
            "profiles": {}
        }))
        .unwrap();

        let check = update_check_from_release_graph_manifest(
            &graph,
            1718444400,
            "1.5.0",
            None,
            None,
            &InstallLayout::MacosPkg,
            "https://release.capsem.org/assets/stable/manifest.json",
            None,
        )
        .unwrap();

        let installer = check.binary_installer.as_ref().unwrap();
        assert_eq!(installer.name, "Capsem-2.0.0.pkg");
        assert_eq!(installer.install_layout, "macos_pkg");
        assert_eq!(installer.sha256, "3".repeat(64));
    }

    #[test]
    fn release_graph_update_check_does_not_select_installer_when_current() {
        let package_name = format!("Capsem_1.5.0_{}.deb", deb_arch());
        let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
            "packages": [
                {
                    "name": package_name,
                    "url": format!("https://github.com/google/capsem/releases/download/v1.5.0/{package_name}"),
                    "version": "1.5.0",
                    "kind": "debian_package",
                    "platform": "linux",
                    "architecture": deb_graph_arch(),
                    "status": "current",
                    "bytes": 222,
                    "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
                }
            ],
            "profiles": {}
        }))
        .unwrap();

        let check = update_check_from_release_graph_manifest(
            &graph,
            1718444400,
            "1.5.0",
            None,
            None,
            &InstallLayout::LinuxDeb,
            "https://release.capsem.org/assets/stable/manifest.json",
            None,
        )
        .unwrap();

        assert_eq!(check.latest_version, Some("1.5.0".to_string()));
        assert!(!check.update_available);
        assert_eq!(check.binary_installer, None);
    }

    #[test]
    fn shared_release_payload_parser_accepts_public_release_graphs() {
        let body = serde_json::to_vec(&serde_json::json!({
            "version": "1.0.142",
            "channel": "stable",
            "status": "current",
            "packages": [{
                "name": "Capsem-99.99.99.pkg",
                "url": "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg",
                "version": "99.99.99",
                "kind": "macos_package",
                "platform": "macos",
                "architecture": "universal",
                "status": "current",
                "bytes": 123,
                "digest": {"sha256": "3".repeat(64), "blake3": "b".repeat(64)}
            }],
            "profiles": {}
        }))
        .unwrap();

        let check = update_check_from_release_payload(
            &body,
            &InstallLayout::MacosPkg,
            "https://release.capsem.org/assets/stable/manifest.json",
            Some("f".repeat(64)),
        )
        .expect("public graph payload");

        assert_eq!(check.latest_version.as_deref(), Some("99.99.99"));
        assert_eq!(
            check.source.as_deref(),
            Some("https://release.capsem.org/assets/stable/manifest.json")
        );
        assert_eq!(check.channel_hash, Some("f".repeat(64)));
    }

    #[test]
    fn shared_release_payload_parser_accepts_profiles_only_release_graphs() {
        let body = serde_json::to_vec(&serde_json::json!({
            "version": "1.0.142",
            "channel": "stable",
            "status": "current",
            "packages": [],
            "profiles": {
                "default": {
                    "revision": "2030.0101.2",
                    "status": "current",
                    "architectures": [{
                        "architecture": deb_graph_arch(),
                        "image_revision": "2030.0101.7",
                        "images": [
                            {
                                "kind": "kernel",
                                "name": "vmlinuz",
                                "url": "https://release.capsem.org/assets/releases/2030.0101.7/vmlinuz",
                                "bytes": 1,
                                "status": "current",
                                "digest": {
                                    "sha256": "1".repeat(64),
                                    "blake3": "a".repeat(64)
                                }
                            },
                            {
                                "kind": "initrd",
                                "name": "initrd.img",
                                "url": "https://release.capsem.org/assets/releases/2030.0101.7/initrd.img",
                                "bytes": 1,
                                "status": "current",
                                "digest": {
                                    "sha256": "2".repeat(64),
                                    "blake3": "b".repeat(64)
                                }
                            },
                            {
                                "kind": "rootfs",
                                "name": "rootfs.erofs",
                                "url": "https://release.capsem.org/assets/releases/2030.0101.7/rootfs.erofs",
                                "bytes": 1,
                                "status": "current",
                                "digest": {
                                    "sha256": "3".repeat(64),
                                    "blake3": "c".repeat(64)
                                }
                            }
                        ]
                    }]
                }
            }
        }))
        .unwrap();

        let check = update_check_from_release_payload(
            &body,
            &InstallLayout::LinuxDeb,
            "https://release.capsem.org/assets/stable/manifest.json",
            Some("f".repeat(64)),
        )
        .expect("profiles-only public graph payload");

        assert_eq!(check.latest_version, None);
        assert_eq!(check.latest_assets.as_deref(), Some("2030.0101.7"));
        assert_eq!(check.latest_profiles.as_deref(), Some("2030.0101.2"));
        assert_eq!(
            binary_installer_from_release_payload(
                &body,
                &InstallLayout::LinuxDeb,
                "https://release.capsem.org/assets/stable/manifest.json",
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn release_graph_update_check_does_not_downgrade_lower_nightly_package() {
        let package_name = format!("Capsem_1.5.99_{}.deb", deb_arch());
        let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
            "version": "1.0.0",
            "channel": "nightly",
            "packages": [
                {
                    "name": package_name,
                    "url": format!("https://github.com/google/capsem/releases/download/v1.5.99/{package_name}"),
                    "version": "1.5.99",
                    "kind": "debian_package",
                    "platform": "linux",
                    "architecture": deb_graph_arch(),
                    "status": "current",
                    "bytes": 222,
                    "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
                }
            ],
            "profiles": {}
        }))
        .unwrap();

        let check = update_check_from_release_graph_manifest(
            &graph,
            1718444400,
            "1.5.100",
            None,
            None,
            &InstallLayout::LinuxDeb,
            "https://release.capsem.org/assets/nightly/manifest.json",
            None,
        )
        .unwrap();

        assert_eq!(check.latest_version, Some("1.5.99".to_string()));
        assert!(!check.update_available);
        assert_eq!(check.binary_installer, None);
    }

    #[test]
    fn release_graph_update_check_does_not_update_non_comparable_nightly_package() {
        let package_name = format!("Capsem_nightly-20260710_{}.deb", deb_arch());
        let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
            "version": "1.0.0",
            "channel": "nightly",
            "packages": [
                {
                    "name": package_name,
                    "url": format!("https://github.com/google/capsem/releases/download/nightly-20260710/{package_name}"),
                    "version": "nightly-20260710",
                    "kind": "debian_package",
                    "platform": "linux",
                    "architecture": deb_graph_arch(),
                    "status": "current",
                    "bytes": 222,
                    "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
                }
            ],
            "profiles": {}
        }))
        .unwrap();

        let check = update_check_from_release_graph_manifest(
            &graph,
            1718444400,
            "1.5.100",
            None,
            None,
            &InstallLayout::LinuxDeb,
            "https://release.capsem.org/assets/nightly/manifest.json",
            None,
        )
        .unwrap();

        assert_eq!(check.latest_version, Some("nightly-20260710".to_string()));
        assert!(!check.update_available);
        assert_eq!(check.binary_installer, None);
    }

    #[test]
    fn release_health_update_check_uses_updates_block() {
        let pkg_sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let deb_sha = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let pkg_blake3 = "1111111111111111111111111111111111111111111111111111111111111111";
        let deb_blake3 = "2222222222222222222222222222222222222222222222222222222222222222";
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.assets_channel.legacy.v1",
            "updates": {
                "binary": {
                    "latest": "99.99.99",
                    "current": "99.99.98",
                    "files": [
                        {
                            "name": "Capsem-99.99.99.pkg",
                            "url": "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg",
                            "sha256": pkg_sha,
                            "blake3": pkg_blake3,
                            "size": 123
                        },
                        {
                            "name": format!("Capsem_99.99.99_{}.deb", deb_arch()),
                            "url": format!("https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_{}.deb", deb_arch()),
                            "sha256": deb_sha,
                            "blake3": deb_blake3,
                            "size": 456
                        }
                    ]
                },
                "assets": {
                    "latest": "2030.0101.1",
                    "current": "2030.0101.0",
                    "state": "published",
                    "compatibility": {
                        "min_binary": "1.0.0"
                    }
                },
                "profiles": {
                    "latest": "profiles-2030.0101.1",
                    "state": "published",
                    "source": "/profiles/releases/profiles-2030.0101.1/catalog.json",
                    "hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "requires_newer": {
                        "binary": false,
                        "assets": false
                    }
                },
                "images": {"latest": null, "state": "not_published"}
            }
        }))
        .unwrap();

        let check = update_check_from_release_health(
            &health,
            1718444400,
            "1.3.1782582155",
            Some("2026.0627.1"),
            Some("profiles-2030.0101.0"),
            &InstallLayout::MacosPkg,
            "https://release.capsem.org/assets/stable/manifest.json",
            Some("f".repeat(64)),
        )
        .unwrap();

        assert_eq!(check.latest_version, Some("99.99.99".to_string()));
        assert!(check.update_available);
        let installer = check.binary_installer.as_ref().unwrap();
        assert_eq!(installer.name, "Capsem-99.99.99.pkg");
        assert_eq!(installer.sha256, pkg_sha);
        assert_eq!(installer.size, 123);
        assert_eq!(installer.install_layout, "macos_pkg");
        assert_eq!(check.latest_assets, Some("2030.0101.1".to_string()));
        assert!(check.assets_update_available);
        assert_eq!(check.assets_state, Some("published".to_string()));
        assert_eq!(check.assets_blocked_reason, None);
        assert_eq!(
            check.current_profiles,
            Some("profiles-2030.0101.0".to_string())
        );
        assert_eq!(
            check.latest_profiles,
            Some("profiles-2030.0101.1".to_string())
        );
        assert!(check.profiles_update_available);
        assert_eq!(check.profiles_state, Some("published".to_string()));
        assert_eq!(check.profiles_blocked_reason, None);
        assert_eq!(
            check.profile_catalog_source,
            Some("/profiles/releases/profiles-2030.0101.1/catalog.json".to_string())
        );
        assert_eq!(check.profile_catalog_hash, Some("b".repeat(64)));
        assert_eq!(check.latest_images, None);
        assert!(!check.images_update_available);
        assert_eq!(check.images_state, Some("not_published".to_string()));
        assert_eq!(check.images_blocked_reason, None);
        assert_eq!(
            check.source,
            Some("https://release.capsem.org/assets/stable/manifest.json".to_string())
        );
        assert_eq!(check.channel_hash, Some("f".repeat(64)));
        assert_eq!(check.validation_status, Some("valid".to_string()));
        assert_eq!(check.validation_error, None);
    }

    #[test]
    fn release_health_update_check_accepts_legacy_current_targets() {
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.assets_channel.legacy.v1",
            "updates": {
                "binary": {"current": "99.99.99"},
                "assets": {"current": "2030.0101.1"}
            }
        }))
        .unwrap();

        let check = update_check_from_release_health(
            &health,
            1718444400,
            "1.3.1782582155",
            Some("2026.0627.1"),
            None,
            &InstallLayout::MacosPkg,
            "https://release.capsem.org/assets/stable/manifest.json",
            None,
        )
        .unwrap();

        assert_eq!(check.latest_version, Some("99.99.99".to_string()));
        assert_eq!(check.latest_assets, Some("2030.0101.1".to_string()));
    }

    #[test]
    fn release_health_asset_update_reports_blocked_compatibility() {
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.assets_channel.legacy.v1",
            "updates": {
                "binary": {"current": "1.3.1782582155"},
                "assets": {
                    "latest": "2030.0101.1",
                    "current": "2030.0101.1",
                    "state": "published",
                    "compatibility": {
                        "min_binary": "99.99.99"
                    }
                }
            }
        }))
        .unwrap();

        let check = update_check_from_release_health(
            &health,
            1718444400,
            "1.3.1782582155",
            Some("2026.0627.1"),
            None,
            &InstallLayout::MacosPkg,
            "https://release.capsem.org/assets/stable/manifest.json",
            None,
        )
        .unwrap();

        assert_eq!(check.latest_assets, Some("2030.0101.1".to_string()));
        assert_eq!(check.current_assets, Some("2026.0627.1".to_string()));
        assert!(!check.assets_update_available);
        assert_eq!(check.assets_state, Some("published".to_string()));
        assert_eq!(
            check.assets_blocked_reason.as_deref(),
            Some("requires binary 99.99.99 or newer")
        );
    }

    #[test]
    fn release_health_deprecated_asset_update_is_blocked() {
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.assets_channel.legacy.v1",
            "updates": {
                "binary": {"current": "1.3.1782582155"},
                "assets": {
                    "latest": "2030.0101.1",
                    "current": "2030.0101.1",
                    "state": "deprecated"
                }
            }
        }))
        .unwrap();

        let check = update_check_from_release_health(
            &health,
            1718444400,
            "1.3.1782582155",
            Some("2026.0627.1"),
            None,
            &InstallLayout::MacosPkg,
            "https://release.capsem.org/assets/stable/manifest.json",
            None,
        )
        .unwrap();

        assert!(!check.assets_update_available);
        assert_eq!(
            check.assets_blocked_reason.as_deref(),
            Some("latest VM asset release is deprecated")
        );
    }

    #[test]
    fn release_health_profile_update_reports_blocked_compatibility() {
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.assets_channel.legacy.v1",
            "updates": {
                "binary": {"current": "1.3.1782582155"},
                "assets": {"current": "2026.0627.1"},
                "profiles": {
                    "latest": "profiles-2030.0101.1",
                    "state": "published",
                    "requires_newer": {
                        "binary": true,
                        "assets": false
                    },
                    "compatibility": {
                        "min_binary": "1.4.0",
                        "min_assets": "2026.0627.1"
                    }
                }
            }
        }))
        .unwrap();

        let check = update_check_from_release_health(
            &health,
            1718444400,
            "1.3.1782582155",
            Some("2026.0627.1"),
            Some("profiles-2030.0101.0"),
            &InstallLayout::MacosPkg,
            "https://release.capsem.org/assets/stable/manifest.json",
            None,
        )
        .unwrap();

        assert_eq!(
            check.current_profiles,
            Some("profiles-2030.0101.0".to_string())
        );
        assert_eq!(
            check.latest_profiles,
            Some("profiles-2030.0101.1".to_string())
        );
        assert!(!check.profiles_update_available);
        assert_eq!(
            check.profiles_blocked_reason.as_deref(),
            Some("requires binary 1.4.0 or newer")
        );
    }

    #[test]
    fn binary_installer_for_layout_selects_matching_deb_arch() {
        let files = vec![
            ReleaseChannelBinaryFile {
                name: "Capsem_99.99.99_wrong.deb".to_string(),
                url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_wrong.deb".to_string(),
                sha256: "1".repeat(64),
                blake3: "a".repeat(64),
                size: 10,
            },
            ReleaseChannelBinaryFile {
                name: format!("Capsem_99.99.99_{}.deb", deb_arch()),
                url: format!(
                    "https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_{}.deb",
                    deb_arch()
                ),
                sha256: "2".repeat(64),
                blake3: "b".repeat(64),
                size: 20,
            },
            ReleaseChannelBinaryFile {
                name: "Capsem-99.99.99.pkg".to_string(),
                url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg".to_string(),
                sha256: "3".repeat(64),
                blake3: "c".repeat(64),
                size: 30,
            },
        ];

        let installer = binary_installer_for_layout(&files, &InstallLayout::LinuxDeb).unwrap();

        assert_eq!(
            installer.name,
            format!("Capsem_99.99.99_{}.deb", deb_arch())
        );
        assert_eq!(installer.sha256, "2".repeat(64));
        assert_eq!(installer.size, 20);
        assert_eq!(installer.install_layout, "linux_deb");
    }

    #[test]
    fn binary_installer_for_layout_rejects_non_http_urls() {
        let files = vec![ReleaseChannelBinaryFile {
            name: "Capsem-99.99.99.pkg".to_string(),
            url: "file:///tmp/Capsem-99.99.99.pkg".to_string(),
            sha256: "local".to_string(),
            blake3: "local".to_string(),
            size: 10,
        }];

        assert_eq!(
            binary_installer_for_layout(&files, &InstallLayout::MacosPkg),
            None
        );
    }

    #[test]
    fn verify_binary_installer_bytes_accepts_matching_sha256_and_size() {
        let bytes = b"verified installer payload";
        let installer = BinaryInstaller {
            name: "Capsem-99.99.99.pkg".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
                .to_string(),
            sha256: test_sha256(bytes),
            size: bytes.len() as u64,
            install_layout: "macos_pkg".to_string(),
        };

        verify_binary_installer_bytes(bytes, &installer).unwrap();
    }

    #[test]
    fn verify_binary_installer_bytes_rejects_size_mismatch() {
        let bytes = b"verified installer payload";
        let installer = BinaryInstaller {
            name: "Capsem-99.99.99.pkg".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
                .to_string(),
            sha256: test_sha256(bytes),
            size: bytes.len() as u64 + 1,
            install_layout: "macos_pkg".to_string(),
        };

        let err = verify_binary_installer_bytes(bytes, &installer).unwrap_err();

        assert!(
            format!("{err:#}").contains("binary installer size mismatch"),
            "{err:#}"
        );
    }

    #[test]
    fn verify_binary_installer_bytes_rejects_sha256_mismatch() {
        let bytes = b"verified installer payload";
        let installer = BinaryInstaller {
            name: "Capsem-99.99.99.pkg".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
                .to_string(),
            sha256: "0".repeat(64),
            size: bytes.len() as u64,
            install_layout: "macos_pkg".to_string(),
        };

        let err = verify_binary_installer_bytes(bytes, &installer).unwrap_err();

        assert!(
            format!("{err:#}").contains("binary installer sha256 mismatch"),
            "{err:#}"
        );
    }

    #[test]
    fn binary_installer_metadata_rejects_path_names() {
        let installer = BinaryInstaller {
            name: "../Capsem-99.99.99.pkg".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
                .to_string(),
            sha256: "0".repeat(64),
            size: 10,
            install_layout: "macos_pkg".to_string(),
        };

        let err = validate_binary_installer_metadata(&installer).unwrap_err();

        assert!(
            format!("{err:#}").contains("binary installer name must be a plain filename"),
            "{err:#}"
        );
    }

    #[test]
    fn binary_installer_apply_plan_uses_macos_pkg_installer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Capsem 99.99.99.pkg");
        std::fs::write(&path, b"pkg").unwrap();
        let installer = BinaryInstaller {
            name: "Capsem-99.99.99.pkg".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
                .to_string(),
            sha256: "0".repeat(64),
            size: 3,
            install_layout: "macos_pkg".to_string(),
        };

        let plan = binary_installer_apply_plan(&installer, &path).unwrap();

        assert_eq!(
            plan.commands,
            vec![BinaryInstallerApplyCommand {
                program: "sudo".to_string(),
                args: vec![
                    "/usr/sbin/installer".to_string(),
                    "-pkg".to_string(),
                    path.display().to_string(),
                    "-target".to_string(),
                    "/".to_string(),
                ],
            }]
        );
        assert_eq!(
            plan.command_lines(),
            vec![format!(
                "sudo /usr/sbin/installer -pkg '{}' -target /",
                path.display()
            )]
        );
    }

    #[test]
    fn binary_installer_apply_plan_uses_apt_for_linux_deb() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Capsem_99.99.99_arm64.deb");
        std::fs::write(&path, b"deb").unwrap();
        let installer = BinaryInstaller {
            name: "Capsem_99.99.99_arm64.deb".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_arm64.deb"
                .to_string(),
            sha256: "0".repeat(64),
            size: 3,
            install_layout: "linux_deb".to_string(),
        };

        let plan = binary_installer_apply_plan(&installer, &path).unwrap();

        assert_eq!(
            plan.commands,
            vec![BinaryInstallerApplyCommand {
                program: "sudo".to_string(),
                args: vec![
                    "apt-get".to_string(),
                    "install".to_string(),
                    "--yes".to_string(),
                    "--allow-downgrades".to_string(),
                    path.display().to_string(),
                ],
            }]
        );
        assert_eq!(
            plan.command_lines(),
            vec![format!(
                "sudo apt-get install --yes --allow-downgrades {}",
                path.display()
            )]
        );
    }

    #[test]
    fn binary_installer_apply_plan_rejects_unknown_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Capsem-99.99.99.pkg");
        std::fs::write(&path, b"pkg").unwrap();
        let installer = BinaryInstaller {
            name: "Capsem-99.99.99.pkg".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
                .to_string(),
            sha256: "0".repeat(64),
            size: 3,
            install_layout: "portable_zip".to_string(),
        };

        let err = binary_installer_apply_plan(&installer, &path).unwrap_err();

        assert!(
            format!("{err:#}").contains("unsupported binary installer layout portable_zip"),
            "{err:#}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::await_holding_lock)]
    async fn download_binary_installer_fetches_verifies_and_caches() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let payload = b"downloaded installer payload".to_vec();
        let response_payload = payload.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                response_payload.len()
            );
            std::io::Write::write_all(&mut stream, header.as_bytes()).unwrap();
            std::io::Write::write_all(&mut stream, &response_payload).unwrap();
        });
        let installer = BinaryInstaller {
            name: "Capsem-99.99.99.pkg".to_string(),
            url: format!("http://{addr}/Capsem-99.99.99.pkg"),
            sha256: test_sha256(&payload),
            size: payload.len() as u64,
            install_layout: "macos_pkg".to_string(),
        };

        let path = download_binary_installer(&installer).await.unwrap();
        server.join().unwrap();

        assert_eq!(
            path,
            home.path().join("updates/installers/Capsem-99.99.99.pkg")
        );
        assert_eq!(std::fs::read(path).unwrap(), payload);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn update_check_rejects_mutating_options_programmatically() {
        for result in [
            run_update(true, true, false, None, None, None).await,
            run_update(false, true, true, None, None, None).await,
            run_update(
                false,
                true,
                false,
                None,
                Some("https://release.capsem.org/assets/stable/manifest.json"),
                None,
            )
            .await,
            run_update(
                false,
                true,
                false,
                None,
                None,
                Some("https://corp.example/capsem/corp.json"),
            )
            .await,
        ] {
            let err = result.expect_err("check-only update must reject mutating options");
            assert!(
                format!("{err:#}").contains("--check cannot be combined"),
                "{err:#}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn update_assets_rejects_corp_policy_source_programmatically() {
        let result = run_update(
            false,
            false,
            true,
            None,
            None,
            Some("https://corp.example/capsem/corp.toml"),
        )
        .await;
        let err = result.expect_err("--assets must not accept a corp policy source");
        let message = format!("{err:#}");
        assert!(
            message.contains("--assets cannot be combined with --corp"),
            "{message}"
        );
        assert!(
            message.contains("--manifest for corporate asset channels"),
            "{message}"
        );
    }

    fn test_sha256(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn release_health_update_check_rejects_wrong_schema() {
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.bad_legacy.v1",
            "updates": {
                "binary": {"current": "99.99.99"},
                "assets": {"current": "2030.0101.1"}
            }
        }))
        .unwrap();

        let err = update_check_from_release_health(
            &health,
            1718444400,
            "1.3.1782582155",
            Some("2026.0627.1"),
            None,
            &InstallLayout::MacosPkg,
            "https://release.capsem.org/assets/stable/manifest.json",
            None,
        )
        .unwrap_err();

        assert!(
            format!("{err:#}").contains("release channel legacy schema mismatch"),
            "{err:#}"
        );
    }

    #[test]
    fn write_manifest_metadata_preserves_package_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
                "package_version": "1.5.1783554373",
                "packaged_at": "2026-07-10T07:20:51Z"
            })
            .to_string(),
        )
        .unwrap();

        write_manifest_metadata(
            &assets_dir,
            "https://release.capsem.org/assets/nightly/manifest.json",
        )
        .unwrap();

        let origin: serde_json::Value = serde_json::from_slice(
            &std::fs::read(assets_dir.join("manifest-metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(origin["schema"], "capsem.manifest_metadata.v1");
        assert_eq!(origin["origin"], "update");
        assert_eq!(
            origin["manifest_url"],
            "https://release.capsem.org/assets/nightly/manifest.json"
        );
        assert_eq!(origin["package_version"], "1.5.1783554373");
        assert_eq!(origin["packaged_at"], "2026-07-10T07:20:51Z");
    }

    #[test]
    fn installed_manifest_metadata_replaces_the_previous_channel_check() {
        let _lock = crate::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
        let assets_dir = home.path().join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        let _assets = EnvGuard::set("CAPSEM_ASSETS_DIR", assets_dir.to_str().unwrap());
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "update",
                "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
                "checked_url": "https://release.capsem.org/assets/stable/manifest.json",
                "checked_at": 100,
                "latest_assets": "stale-assets",
                "assets_update_available": true,
                "package_version": env!("CARGO_PKG_VERSION")
            })
            .to_string(),
        )
        .unwrap();
        let manifest = test_manifest(
            env!("CARGO_PKG_VERSION"),
            "2026.0714.18",
            env!("CARGO_PKG_VERSION"),
            "2026.0714.18",
        );
        let bytes = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(assets_dir.join("manifest.json"), &bytes).unwrap();
        let corp_source = "https://corp.example/capsem/manifest.json";

        write_installed_manifest_metadata(&assets_dir, corp_source, &bytes).unwrap();

        let metadata: serde_json::Value = serde_json::from_slice(
            &std::fs::read(assets_dir.join("manifest-metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["manifest_url"], corp_source);
        assert_eq!(metadata["checked_url"], corp_source);
        assert_eq!(metadata["latest_assets"], "2026.0714.18");
        assert_eq!(metadata["current_assets"], "2026.0714.18");
        assert_eq!(metadata["assets_update_available"], false);
        assert_eq!(metadata["validation_status"], "valid");
        assert!(metadata["checked_at"].as_u64().unwrap() > 100);
    }

    #[test]
    fn public_channel_switch_is_allowed_in_both_directions_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "update",
                "manifest_url": "https://release.capsem.org/assets/nightly/manifest.json",
                "channel": "nightly",
                "channel_kind": "public",
                "channel_locked": false
            })
            .to_string(),
        )
        .unwrap();

        let transition = channel_transition_for_request(&assets_dir, Some("stable"), None).unwrap();
        assert_eq!(transition, ChannelTransition::Public("stable".to_string()));
        persist_channel_transition(&assets_dir, &transition).unwrap();

        let origin = installed_manifest_metadata(&assets_dir).unwrap().unwrap();
        assert_eq!(origin["channel"], "stable");
        assert_eq!(origin["channel_kind"], "public");
        assert_eq!(origin["channel_locked"], false);
    }

    #[test]
    fn explicit_corporate_manifest_locks_channel_one_way() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "update",
                "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
                "channel": "stable",
                "channel_kind": "public",
                "channel_locked": false
            })
            .to_string(),
        )
        .unwrap();

        let corp_source = "https://corp.example/capsem/manifest.json";
        let transition =
            channel_transition_for_request(&assets_dir, None, Some(corp_source)).unwrap();
        assert_eq!(transition, ChannelTransition::Corporate);
        write_manifest_metadata(&assets_dir, corp_source).unwrap();
        persist_channel_transition(&assets_dir, &transition).unwrap();

        let origin = installed_manifest_metadata(&assets_dir).unwrap().unwrap();
        assert_eq!(origin["channel"], "corp");
        assert_eq!(origin["channel_kind"], "corporate");
        assert_eq!(origin["channel_locked"], true);

        let error = channel_transition_for_request(&assets_dir, Some("stable"), None).unwrap_err();
        assert!(
            format!("{error:#}").contains("corporate channel is locked"),
            "{error:#}"
        );
        assert_eq!(
            channel_transition_for_request(&assets_dir, None, Some(corp_source)).unwrap(),
            ChannelTransition::Preserve
        );
    }

    #[test]
    fn local_manifest_asset_source_uses_manifest_metadata_parent() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        let source_dir = dir.path().join("source-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&source_dir).unwrap();
        let manifest = source_dir.join("manifest.json");
        std::fs::write(&manifest, "{}").unwrap();
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": format!("file://{}", manifest.display())
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            local_manifest_asset_source(&assets_dir).unwrap(),
            Some(source_dir)
        );
    }

    #[test]
    fn local_manifest_asset_source_ignores_remote_origin() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": "https://example.invalid/manifest.json"
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(local_manifest_asset_source(&assets_dir).unwrap(), None);
    }

    #[test]
    fn remote_manifest_asset_source_uses_remote_origin() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": "https://release.capsem.org/assets/stable/manifest.json"
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            remote_manifest_asset_source(&assets_dir).unwrap(),
            Some("https://release.capsem.org/assets/stable/manifest.json".to_string())
        );
    }

    #[test]
    fn remote_manifest_asset_source_ignores_file_origin() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        let source_dir = dir.path().join("source-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&source_dir).unwrap();
        let manifest = source_dir.join("manifest.json");
        std::fs::write(&manifest, "{}").unwrap();
        let source = format!("file://{}", manifest.display());
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": source
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(remote_manifest_asset_source(&assets_dir).unwrap(), None);
    }

    #[test]
    fn local_manifest_asset_source_rejects_bare_paths() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": "/tmp/corp/assets/stable/manifest.json"
            })
            .to_string(),
        )
        .unwrap();

        let err = local_manifest_asset_source(&assets_dir).unwrap_err();
        assert!(
            format!("{err:#}").contains("asset manifest metadata source must be a URL"),
            "{err:#}"
        );
    }

    #[test]
    fn local_manifest_asset_source_rejects_file_url_shorthand_paths() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(
            assets_dir.join("manifest-metadata.json"),
            serde_json::json!({
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": "file:assets/stable/manifest.json"
            })
            .to_string(),
        )
        .unwrap();

        let err = local_manifest_asset_source(&assets_dir).unwrap_err();
        assert!(
            format!("{err:#}").contains("asset manifest metadata file URL must start with file://"),
            "{err:#}"
        );
    }
}
