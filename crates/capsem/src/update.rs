//! Self-update: check the release channel for binary and VM asset versions.
//!
//! `release.capsem.org/health.json` is the source of truth for freshness. The
//! binary path selects a platform installer from that health index; the
//! privileged installer apply step is intentionally separate from VM asset
//! hydration.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::platform::{self, InstallLayout};
use capsem_core::net::policy_config::{ProfileCatalog, ProfileConfigFile};

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
    /// Machine-readable release channel index used for this check.
    #[serde(default)]
    pub source: Option<String>,
    /// SHA-256 of the last valid release-channel health payload.
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
const DEFAULT_RELEASE_HEALTH_URL: &str = "https://release.capsem.org/health.json";
const RELEASE_HEALTH_URL_ENV: &str = "CAPSEM_RELEASE_HEALTH_URL";

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelHealth {
    schema: String,
    updates: ReleaseChannelUpdates,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseChannelUpdates {
    binary: ReleaseChannelUpdateTarget,
    assets: ReleaseChannelUpdateTarget,
    #[serde(default)]
    profiles: Option<ReleaseChannelUpdateTarget>,
    #[serde(default)]
    images: Option<ReleaseChannelUpdateTarget>,
}

#[derive(Debug, Clone, Deserialize)]
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

impl ReleaseChannelUpdateTarget {
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

fn cache_path() -> Option<PathBuf> {
    crate::paths::capsem_home()
        .ok()
        .map(|d| d.join("update-check.json"))
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
    let path = cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let check: UpdateCheck = serde_json::from_str(&content).ok()?;

    if !check.update_available {
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

    None
}

/// Write update check cache atomically (write tmp + rename).
fn write_cache(check: &UpdateCheck) -> Result<()> {
    let path = cache_path().context("HOME not set")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(check)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn read_cache_file(path: &Path) -> Result<UpdateCheck> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))
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
    let path = match cache_path() {
        Some(p) => p,
        None => return,
    };

    // Check if cache exists and is fresh.
    let previous_check = read_cache_file(&path).ok();
    if let Some(check) = previous_check.as_ref() {
        let age = now_secs().saturating_sub(check.checked_at);
        if age < CACHE_TTL_SECS {
            return; // Still fresh
        }
    }

    info!("update cache stale, checking for updates");

    let health_url = match release_health_url() {
        Ok(url) => url,
        Err(e) => {
            warn!(error = %e, "update check: invalid release channel URL");
            return;
        }
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get(&health_url)
        .header("Accept", "application/json")
        .header("User-Agent", "capsem")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            warn!(status = %r.status(), url = %health_url, "update check: release channel error");
            return;
        }
        Err(e) => {
            warn!(error = %e, "update check failed");
            let check = failed_update_check_from_previous(
                previous_check,
                now_secs(),
                &health_url,
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
            warn!(error = %e, url = %health_url, "update check: failed to read release channel body");
            let check = failed_update_check_from_previous(
                previous_check,
                now_secs(),
                &health_url,
                "fetch_error",
                e.to_string(),
            );
            let _ = write_cache(&check);
            return;
        }
    };
    let channel_hash = channel_payload_hash(&bytes);
    let body: ReleaseChannelHealth = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, url = %health_url, "update check: invalid release channel JSON");
            let check = failed_update_check_from_previous(
                previous_check,
                now_secs(),
                &health_url,
                "invalid_json",
                e.to_string(),
            );
            let _ = write_cache(&check);
            return;
        }
    };

    let check = match update_check_from_release_health(
        &body,
        now_secs(),
        env!("CARGO_PKG_VERSION"),
        local_current_asset_version().as_deref(),
        local_current_profile_catalog_revision().as_deref(),
        &platform::detect_install_layout(),
        &health_url,
        Some(channel_hash),
    ) {
        Ok(check) => check,
        Err(e) => {
            warn!(error = %e, url = %health_url, "update check: invalid release channel contract");
            let check = failed_update_check_from_previous(
                previous_check,
                now_secs(),
                &health_url,
                "invalid_contract",
                e.to_string(),
            );
            let _ = write_cache(&check);
            return;
        }
    };
    let _ = write_cache(&check);
}

fn release_health_url() -> Result<String> {
    if let Ok(value) = std::env::var(RELEASE_HEALTH_URL_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return validate_release_health_url(trimmed);
        }
    }

    if let Some(url) = release_health_url_from_manifest_origin() {
        return Ok(url);
    }

    Ok(DEFAULT_RELEASE_HEALTH_URL.to_string())
}

fn validate_release_health_url(url: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(url).with_context(|| {
        format!("{RELEASE_HEALTH_URL_ENV} must be a URL: use https://... or http://..., got {url}")
    })?;
    if !matches!(parsed.scheme(), "https" | "http") {
        anyhow::bail!(
            "unsupported {RELEASE_HEALTH_URL_ENV} scheme {}: use https:// or http://",
            parsed.scheme()
        );
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn release_health_url_from_manifest_origin() -> Option<String> {
    let assets_dir = capsem_core::asset_manager::default_assets_dir()?;
    let origin_path = assets_dir.join("manifest-origin.json");
    let content = std::fs::read_to_string(origin_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let source = value.get("source").and_then(|v| v.as_str())?;
    release_health_url_from_manifest_url(source)
}

fn release_health_url_from_manifest_url(manifest_url: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(manifest_url).ok()?;
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
    let root_segments = segments[..assets_pos].to_vec();
    {
        let mut out = url.path_segments_mut().ok()?;
        out.clear();
        for segment in root_segments {
            out.push(&segment);
        }
        out.push("health.json");
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

fn profile_blocked_reason(target: &ReleaseChannelUpdateTarget) -> Option<String> {
    let requires = target.requires_newer.as_ref()?;
    let compatibility = target.compatibility.as_ref();
    let mut reasons = Vec::new();
    if requires.binary {
        let version = compatibility
            .and_then(|compatibility| compatibility.min_binary.as_deref())
            .unwrap_or("a newer version");
        reasons.push(format!("requires binary {version} or newer"));
    }
    if requires.assets {
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

fn update_check_from_release_health(
    health: &ReleaseChannelHealth,
    checked_at: u64,
    current_binary: &str,
    current_assets: Option<&str>,
    current_profiles: Option<&str>,
    install_layout: &InstallLayout,
    source: &str,
    channel_hash: Option<String>,
) -> Result<UpdateCheck> {
    if health.schema != "capsem.assets_channel.health.v1" {
        anyhow::bail!("release channel health schema mismatch");
    }
    let latest_version = health.updates.binary.latest_version();
    let latest_assets = health.updates.assets.latest_version();
    let latest_profiles = health
        .updates
        .profiles
        .as_ref()
        .and_then(ReleaseChannelUpdateTarget::latest_version);
    let profiles_state = health
        .updates
        .profiles
        .as_ref()
        .and_then(|target| target.state.clone());
    let profile_catalog_source = health
        .updates
        .profiles
        .as_ref()
        .and_then(|target| target.source.clone());
    let profile_catalog_hash = health
        .updates
        .profiles
        .as_ref()
        .and_then(|target| target.hash.clone());
    let latest_images = health
        .updates
        .images
        .as_ref()
        .and_then(ReleaseChannelUpdateTarget::latest_version);
    let images_state = health
        .updates
        .images
        .as_ref()
        .and_then(|target| target.state.clone());
    let update_available = latest_version
        .as_deref()
        .is_some_and(|latest| is_newer(latest, current_binary));
    let binary_installer = if update_available {
        binary_installer_for_layout(&health.updates.binary.files, install_layout)
    } else {
        None
    };
    let assets_update_available = match (latest_assets.as_deref(), current_assets) {
        (Some(latest), Some(current)) => latest != current,
        _ => false,
    };
    let profiles_differ = match (latest_profiles.as_deref(), current_profiles) {
        (Some(latest), Some(current)) => latest != current,
        _ => false,
    };
    let profiles_blocked_reason = health
        .updates
        .profiles
        .as_ref()
        .and_then(profile_blocked_reason)
        .or_else(|| {
            if profiles_differ && profile_catalog_source.is_none() {
                Some("release channel did not advertise a profile catalog source".to_string())
            } else if profiles_differ && profile_catalog_hash.is_none() {
                Some("release channel did not advertise a profile catalog hash".to_string())
            } else {
                None
            }
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
    let (layout_name, matches_layout): (&str, Box<dyn Fn(&str) -> bool>) = match install_layout {
        InstallLayout::MacosPkg => ("macos_pkg", Box::new(|name| name.ends_with(".pkg"))),
        InstallLayout::LinuxDeb => {
            let suffix = format!("_{}.deb", deb_arch());
            ("linux_deb", Box::new(move |name| name.ends_with(&suffix)))
        }
        InstallLayout::UserDir | InstallLayout::Development => return None,
    };

    files
        .iter()
        .filter(|file| matches_layout(&file.name))
        .filter(|file| {
            let installer = BinaryInstaller {
                name: file.name.clone(),
                url: file.url.clone(),
                sha256: file.sha256.clone(),
                size: file.size,
                install_layout: layout_name.to_string(),
            };
            validate_binary_installer_metadata(&installer).is_ok()
        })
        .min_by(|left, right| left.name.cmp(&right.name))
        .map(|file| BinaryInstaller {
            name: file.name.clone(),
            url: file.url.clone(),
            sha256: file.sha256.clone(),
            size: file.size,
            install_layout: layout_name.to_string(),
        })
}

fn deb_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    }
}

async fn fetch_release_update_check(layout: &InstallLayout) -> Result<UpdateCheck> {
    let health_url = release_health_url()?;
    let resp = reqwest::Client::new()
        .get(&health_url)
        .header("Accept", "application/json")
        .header("User-Agent", "capsem")
        .send()
        .await
        .with_context(|| format!("GET {health_url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("GET {} returned {}", health_url, resp.status());
    }
    let body = resp
        .bytes()
        .await
        .with_context(|| format!("read release channel health from {health_url}"))?;
    let channel_hash = channel_payload_hash(&body);
    let body: ReleaseChannelHealth = serde_json::from_slice(&body)
        .with_context(|| format!("parse release channel health from {health_url}"))?;
    update_check_from_release_health(
        &body,
        now_secs(),
        env!("CARGO_PKG_VERSION"),
        local_current_asset_version().as_deref(),
        local_current_profile_catalog_revision().as_deref(),
        layout,
        &health_url,
        Some(channel_hash),
    )
}

async fn download_binary_installer(installer: &BinaryInstaller) -> Result<PathBuf> {
    validate_binary_installer_metadata(installer)?;
    let target = binary_installer_cache_path(installer)?;
    if target.exists() {
        verify_binary_installer_file(&target, installer)?;
        return Ok(target);
    }

    let resp = reqwest::Client::new()
        .get(&installer.url)
        .header("User-Agent", "capsem")
        .send()
        .await
        .with_context(|| format!("GET {}", installer.url))?;
    if !resp.status().is_success() {
        anyhow::bail!("GET {} returned {}", installer.url, resp.status());
    }
    let bytes = resp
        .bytes()
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

/// Run the update flow.
///
/// With `assets = true`, refresh only the VM asset files referenced by the
/// locally-installed manifest. Binary updates download the matching verified
/// package and hand it to the platform package manager when `--yes` is set.
pub async fn run_update(
    yes: bool,
    check_only: bool,
    assets: bool,
    manifest_source: Option<&str>,
    corp_source: Option<&str>,
) -> Result<()> {
    let layout = platform::detect_install_layout();
    if check_only && (yes || assets || manifest_source.is_some() || corp_source.is_some()) {
        anyhow::bail!("--check cannot be combined with mutating update options");
    }

    let mut did_work = false;
    if let Some(source) = corp_source {
        provision_corp_config(source).await?;
        did_work = true;
    }

    if assets || manifest_source.is_some() {
        refresh_assets(manifest_source).await?;
        return Ok(());
    }

    if did_work {
        return Ok(());
    }

    if layout == InstallLayout::Development {
        println!("Development build detected. Update from source with `git pull && just install`.");
        return Ok(());
    }

    let check = match fetch_release_update_check(&layout).await {
        Ok(check) => check,
        Err(error) => {
            println!("Binary update check failed: {error:#}");
            println!("Run `capsem update --assets` to refresh VM assets, or retry later.");
            return Ok(());
        }
    };
    let _ = write_cache(&check);

    let current = env!("CARGO_PKG_VERSION");
    if check_only {
        print_update_check_summary(&check, current, &layout);
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
                    let path = download_binary_installer(installer).await?;
                    println!("Verified installer: {}", path.display());
                    let plan = binary_installer_apply_plan(installer, &path)?;
                    println!("Apply command:");
                    for command in plan.command_lines() {
                        println!("  {command}");
                    }
                    apply_binary_installer_plan(&plan).await?;
                    println!("Binary update applied. Restart Capsem to use {latest}.");
                    did_update = true;
                } else {
                    println!("Re-run with --yes to download and verify the installer package.");
                }
            } else {
                println!(
                    "No installer package in release health matches this install layout ({layout:?})."
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

    if check.update_available || check.assets_update_available {
        println!("Run `capsem update --assets` separately to refresh VM assets.");
    }
    print_image_update_status(&check);

    if !check.update_available
        && !check.profiles_update_available
        && !check.assets_update_available
        && !check.images_update_available
    {
        println!("Capsem is current ({current}).");
    } else if !did_update && !check.update_available && !check.assets_update_available {
        if check.profiles_blocked_reason.is_none() {
            println!("No local update action was needed.");
        } else {
            println!(
                "Capsem binary is current; profile catalog update requires a newer dependency."
            );
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
                    "No installer package in release health matches this install layout ({layout:?})."
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

    if !check.update_available
        && !check.profiles_update_available
        && !check.assets_update_available
        && !check.images_update_available
    {
        println!("Capsem is current ({current}).");
    }
}

fn print_asset_update_status(check: &UpdateCheck) {
    if check.assets_update_available {
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
    if check.images_update_available {
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
        .unwrap_or(DEFAULT_RELEASE_HEALTH_URL);
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
            let resp = reqwest::Client::new()
                .get(url.clone())
                .header("Accept", "application/json")
                .header("User-Agent", "capsem")
                .send()
                .await
                .with_context(|| format!("GET {source}"))?;
            if !resp.status().is_success() {
                anyhow::bail!("GET {} returned {}", source, resp.status());
            }
            Ok(resp
                .bytes()
                .await
                .with_context(|| format!("read profile catalog body from {source}"))?
                .to_vec())
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

/// Pull any missing / hash-mismatched VM assets from the release URL.
async fn refresh_assets(manifest_source: Option<&str>) -> Result<()> {
    let assets_dir = capsem_core::asset_manager::default_assets_dir()
        .context("cannot resolve CAPSEM_HOME -- set $HOME or $CAPSEM_HOME")?;
    if let Some(source) = manifest_source {
        install_manifest_source(&assets_dir, source).await?;
    } else if let Some(source) = remote_manifest_asset_source(&assets_dir)? {
        install_manifest_source(&assets_dir, &source).await?;
    }
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
    if let Some(local_source) = local_manifest_asset_source(&assets_dir)? {
        println!("Using local asset source {}...", local_source.display());
        let copied = capsem_core::asset_manager::copy_missing_local_assets(
            &manifest,
            binary_version,
            arch,
            &local_source,
            &assets_dir,
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
        &assets_dir,
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

async fn provision_corp_config(source: &str) -> Result<()> {
    let capsem_dir = crate::paths::capsem_home()?;
    capsem_core::net::policy_config::corp_provision::provision_from_source(&capsem_dir, source)
        .await
        .with_context(|| format!("provision corp config from {source}"))?;
    println!("Corp config updated from {source}.");
    Ok(())
}

async fn install_manifest_source(assets_dir: &std::path::Path, source: &str) -> Result<()> {
    let bytes = read_manifest_source(source).await?;
    let body = std::str::from_utf8(&bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {source}"))?;
    capsem_core::asset_manager::ManifestV2::from_json(body)
        .with_context(|| format!("parse manifest from {source}"))?;

    std::fs::create_dir_all(assets_dir)
        .with_context(|| format!("cannot create {}", assets_dir.display()))?;
    atomic_write(&assets_dir.join("manifest.json"), &bytes)?;

    let origin = serde_json::json!({
        "schema": "capsem.manifest_origin.v1",
        "origin": "update",
        "source": source
    });
    let origin_bytes = serde_json::to_vec_pretty(&origin)?;
    atomic_write(&assets_dir.join("manifest-origin.json"), &origin_bytes)?;
    println!("Installed asset manifest from {source}.");
    Ok(())
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
            let resp = reqwest::Client::new()
                .get(url.clone())
                .header("Accept", "application/json")
                .header("User-Agent", "capsem")
                .send()
                .await
                .with_context(|| format!("GET {source}"))?;
            if !resp.status().is_success() {
                anyhow::bail!("GET {} returned {}", source, resp.status());
            }
            Ok(resp
                .bytes()
                .await
                .with_context(|| format!("read manifest body from {source}"))?
                .to_vec())
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

fn local_manifest_asset_source(assets_dir: &std::path::Path) -> Result<Option<PathBuf>> {
    let origin_path = assets_dir.join("manifest-origin.json");
    if !origin_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&origin_path)
        .with_context(|| format!("read {}", origin_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parse {}", origin_path.display()))?;
    let Some(source) = value.get("source").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(None);
    }
    let parsed = reqwest::Url::parse(source).with_context(|| {
        format!(
            "asset manifest origin source must be a URL: use https://..., http://..., or file:///absolute/path, got {source}"
        )
    })?;
    if parsed.scheme() != "file" {
        anyhow::bail!(
            "unsupported asset manifest origin URL scheme {}: use https://, http://, or file://",
            parsed.scheme()
        );
    }
    if !has_scheme_authority_prefix(source, "file") {
        anyhow::bail!("asset manifest origin file URL must start with file://: {source}");
    }
    let path = parsed.to_file_path().map_err(|_| {
        anyhow::anyhow!("asset manifest origin file URL must be absolute: {source}")
    })?;
    if !path.is_file() {
        return Ok(None);
    }
    Ok(path.parent().map(|parent| parent.to_path_buf()))
}

fn remote_manifest_asset_source(assets_dir: &std::path::Path) -> Result<Option<String>> {
    let origin_path = assets_dir.join("manifest-origin.json");
    if !origin_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&origin_path)
        .with_context(|| format!("read {}", origin_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parse {}", origin_path.display()))?;
    let Some(source) = value.get("source").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    if !(source.starts_with("http://") || source.starts_with("https://")) {
        return Ok(None);
    }
    let parsed = reqwest::Url::parse(source).with_context(|| {
        format!(
            "asset manifest origin source must be a URL: use https://..., http://..., or file:///absolute/path, got {source}"
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!(
            "unsupported asset manifest origin URL scheme {}: use https://, http://, or file://",
            parsed.scheme()
        );
    }
    if !has_scheme_authority_prefix(source, parsed.scheme()) {
        anyhow::bail!(
            "asset manifest origin must use https://, http://, or file:// URLs, got {source}"
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
            source: Some("https://release.capsem.org/health.json".into()),
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
        assert_eq!(
            rt.source,
            Some("https://release.capsem.org/health.json".into())
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
        assert_eq!(rt.source, None);
        assert_eq!(rt.channel_hash, None);
        assert_eq!(rt.validation_status, None);
        assert_eq!(rt.validation_error, None);
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
            source: Some("https://release.capsem.org/health.json".into()),
            channel_hash: Some("f".repeat(64)),
            validation_status: Some("valid".into()),
            validation_error: None,
        };

        let check = failed_update_check_from_previous(
            Some(previous),
            1200,
            "https://release.capsem.org/health.json",
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
    fn release_health_url_derives_from_remote_manifest_url() {
        assert_eq!(
            release_health_url_from_manifest_url(
                "https://release.capsem.org/assets/stable/manifest.json"
            ),
            Some("https://release.capsem.org/health.json".to_string())
        );
        assert_eq!(
            release_health_url_from_manifest_url(
                "https://corp.example/capsem/assets/internal/manifest.json"
            ),
            Some("https://corp.example/capsem/health.json".to_string())
        );
        assert_eq!(
            release_health_url_from_manifest_url("file:///tmp/assets/stable/manifest.json"),
            None
        );
    }

    #[test]
    fn release_health_url_env_rejects_bare_paths() {
        let err = validate_release_health_url("/tmp/release/health.json").unwrap_err();
        assert!(
            format!("{err:#}").contains("CAPSEM_RELEASE_HEALTH_URL must be a URL"),
            "{err:#}"
        );
    }

    #[test]
    fn release_health_update_check_uses_updates_block() {
        let pkg_sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let deb_sha = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.assets_channel.health.v1",
            "updates": {
                "binary": {
                    "latest": "99.99.99",
                    "current": "99.99.98",
                    "files": [
                        {
                            "name": "Capsem-99.99.99.pkg",
                            "url": "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg",
                            "sha256": pkg_sha,
                            "size": 123
                        },
                        {
                            "name": format!("Capsem_99.99.99_{}.deb", deb_arch()),
                            "url": format!("https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_{}.deb", deb_arch()),
                            "sha256": deb_sha,
                            "size": 456
                        }
                    ]
                },
                "assets": {"latest": "2030.0101.1", "current": "2030.0101.0"},
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
            "https://release.capsem.org/health.json",
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
        assert_eq!(
            check.source,
            Some("https://release.capsem.org/health.json".to_string())
        );
        assert_eq!(check.channel_hash, Some("f".repeat(64)));
        assert_eq!(check.validation_status, Some("valid".to_string()));
        assert_eq!(check.validation_error, None);
    }

    #[test]
    fn release_health_update_check_accepts_legacy_current_targets() {
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.assets_channel.health.v1",
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
            "https://release.capsem.org/health.json",
            None,
        )
        .unwrap();

        assert_eq!(check.latest_version, Some("99.99.99".to_string()));
        assert_eq!(check.latest_assets, Some("2030.0101.1".to_string()));
    }

    #[test]
    fn release_health_profile_update_reports_blocked_compatibility() {
        let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
            "schema": "capsem.assets_channel.health.v1",
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
            "https://release.capsem.org/health.json",
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
                size: 10,
            },
            ReleaseChannelBinaryFile {
                name: format!("Capsem_99.99.99_{}.deb", deb_arch()),
                url: format!(
                    "https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_{}.deb",
                    deb_arch()
                ),
                sha256: "2".repeat(64),
                size: 20,
            },
            ReleaseChannelBinaryFile {
                name: "Capsem-99.99.99.pkg".to_string(),
                url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg".to_string(),
                sha256: "3".repeat(64),
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
                    path.display().to_string(),
                ],
            }]
        );
        assert_eq!(
            plan.command_lines(),
            vec![format!("sudo apt-get install --yes {}", path.display())]
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
            run_update(true, true, false, None, None).await,
            run_update(false, true, true, None, None).await,
            run_update(
                false,
                true,
                false,
                Some("https://release.capsem.org/assets/stable/manifest.json"),
                None,
            )
            .await,
            run_update(
                false,
                true,
                false,
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
            "schema": "capsem.bad_health.v1",
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
            "https://release.capsem.org/health.json",
            None,
        )
        .unwrap_err();

        assert!(
            format!("{err:#}").contains("release channel health schema mismatch"),
            "{err:#}"
        );
    }

    #[test]
    fn local_manifest_asset_source_uses_manifest_origin_parent() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        let source_dir = dir.path().join("source-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&source_dir).unwrap();
        let manifest = source_dir.join("manifest.json");
        std::fs::write(&manifest, "{}").unwrap();
        std::fs::write(
            assets_dir.join("manifest-origin.json"),
            serde_json::json!({
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": format!("file://{}", manifest.display())
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
            assets_dir.join("manifest-origin.json"),
            serde_json::json!({
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": "https://example.invalid/manifest.json"
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
            assets_dir.join("manifest-origin.json"),
            serde_json::json!({
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": "https://release.capsem.org/assets/stable/manifest.json"
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
            assets_dir.join("manifest-origin.json"),
            serde_json::json!({
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": source
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
            assets_dir.join("manifest-origin.json"),
            serde_json::json!({
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": "/tmp/corp/assets/stable/manifest.json"
            })
            .to_string(),
        )
        .unwrap();

        let err = local_manifest_asset_source(&assets_dir).unwrap_err();
        assert!(
            format!("{err:#}").contains("asset manifest origin source must be a URL"),
            "{err:#}"
        );
    }

    #[test]
    fn local_manifest_asset_source_rejects_file_url_shorthand_paths() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("installed-assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(
            assets_dir.join("manifest-origin.json"),
            serde_json::json!({
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": "file:assets/stable/manifest.json"
            })
            .to_string(),
        )
        .unwrap();

        let err = local_manifest_asset_source(&assets_dir).unwrap_err();
        assert!(
            format!("{err:#}").contains("asset manifest origin file URL must start with file://"),
            "{err:#}"
        );
    }
}
