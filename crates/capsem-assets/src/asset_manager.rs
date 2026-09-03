//! Asset manager for downloading and verifying VM assets.
//!
//! VM assets (rootfs) are too large to bundle in the DMG. The asset manager
//! downloads them on first launch and verifies integrity via blake3 hashes.
//!
//! ## Versioning
//!
//! Binary version (`1.0.{timestamp}`) and asset version (`YYYY.MMDD.patch`)
//! are independent. The manifest tracks both with compatibility ranges
//! (`min_binary`, `min_assets`).
//!
//! ## Storage
//!
//! Flat `~/.capsem/assets/` with hash-based filenames
//! (`vmlinuz-{hash16}`, `rootfs-{hash16}.erofs`). Same hash = same file =
//! natural dedup across asset versions.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

mod hydrate;
pub use hydrate::{copy_missing_local_assets, download_missing_assets, DownloadProgress};

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Validate a version string (no path traversal).
fn validate_version(version: &str) -> Result<()> {
    if version.is_empty() {
        bail!("version string is empty");
    }
    if version.contains("..") || version.contains('/') || version.contains('\\') {
        bail!("version contains path traversal: {version}");
    }
    Ok(())
}

/// Validate a filename (no path separators or traversal).
fn validate_filename(filename: &str) -> Result<()> {
    if filename.is_empty() {
        bail!("filename is empty");
    }
    if filename.contains(['/', '\\', '\0']) || filename.contains("..") {
        bail!("filename contains a path separator, traversal, or NUL: {filename:?}");
    }
    Ok(())
}

/// Validate a blake3 hash string (exactly 64 hex characters).
fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid blake3 hash (expected 64 hex chars): {hash}");
    }
    Ok(())
}

fn validate_sha256(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid sha256 hash (expected 64 hex chars): {hash}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Manifest types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    Arm64,
    X86_64,
}

impl Architecture {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Arm64 => "arm64",
            Self::X86_64 => "x86_64",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageArchitecture {
    Amd64,
    Arm64,
}

impl PackageArchitecture {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }

    pub fn from_package_name(name: &str) -> Result<Self> {
        if name.ends_with(".pkg") {
            return Ok(Self::Arm64);
        }
        if name.ends_with("_amd64.deb") {
            return Ok(Self::Amd64);
        }
        if name.ends_with("_arm64.deb") {
            return Ok(Self::Arm64);
        }
        bail!("package name must end in .pkg, _amd64.deb, or _arm64.deb: {name}")
    }
}

/// A single asset entry (keyed by logical name in the map).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetEntry {
    pub hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    pub size: u64,
}

/// An asset release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRelease {
    /// Build date (YYYY-MM-DD). Pure metadata. Optional because the CI
    /// release-pipeline writer historically omitted it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_date: Option<String>,
    /// Oldest binary version compatible with these assets. Optional; when set,
    /// runtime asset selection refuses this release for older binaries.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub min_binary: String,
    /// Per-arch asset maps: arch -> { logical_name -> AssetEntry }.
    pub arches: HashMap<String, HashMap<String, AssetEntry>>,
}

/// A binary release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryRelease {
    /// Build date (YYYY-MM-DD). Pure metadata. Optional because the CI
    /// release-pipeline writer omits it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_date: Option<String>,
    /// Oldest asset version this binary can boot. Optional -- when empty,
    /// `pick_asset_version` falls back to `assets.current`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub min_assets: String,
    /// Echo of the version key (release.yaml writes this; harmless).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// pkg/deb metadata published by the release pipeline. Not consulted
    /// at runtime; preserved on round-trip so external tooling can read it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<BinaryFile>,
}

/// One downloadable binary asset (e.g. .pkg, .deb) listed under a
/// `BinaryRelease`. Metadata only -- the runtime resolver never reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blake3: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<BinaryExecutable>,
}

/// One executable file contained inside a host package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryExecutable {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub installed_path: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blake3: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sbom_component_ref: String,
}

/// The assets section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetsSection {
    pub current: String,
    pub releases: HashMap<String, AssetRelease>,
}

/// The binaries section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinariesSection {
    pub current: String,
    pub releases: HashMap<String, BinaryRelease>,
}

/// Manifest with orthogonal binary and asset version tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestV2 {
    pub format: u32,
    pub refresh_policy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_base: Option<String>,
    pub assets: AssetsSection,
    pub binaries: BinariesSection,
}

/// Resolved file paths for booting a VM.
#[derive(Debug, Clone)]
pub struct ResolvedAssets {
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub rootfs: PathBuf,
    pub asset_version: String,
}

/// BLAKE3 hashes for the three canonical boot assets of one arch.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedAssetHashes {
    pub kernel: String,
    pub initrd: String,
    pub rootfs: String,
}

/// Comparable state for every profile carried by one public release graph.
///
/// The installed public manifest remains the authority on disk. This in-memory
/// view gives the updater and service deterministic identities without
/// flattening distinct profile revisions or image sets into one default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseGraphProfileState {
    pub catalog_revision: String,
    pub images_revision: String,
    pub profiles: BTreeMap<String, ReleaseGraphProfileIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseGraphProfileIdentity {
    pub revision: String,
    pub status: String,
    pub config_revision: String,
    pub evidence_revision: String,
    pub images_revision: String,
    pub architectures: Vec<String>,
}

/// Map `std::env::consts::ARCH` names to the keys used under
/// `manifest.assets.releases.<ver>.arches`. Unknown arches pass through.
pub fn map_rustc_arch_to_manifest(rustc_arch: &str) -> &str {
    match rustc_arch {
        "aarch64" => "arm64",
        other => other,
    }
}

/// Host arch as a manifest key (e.g. "arm64", "x86_64").
pub fn host_manifest_arch() -> &'static str {
    map_rustc_arch_to_manifest(std::env::consts::ARCH)
}

const ROOTFS_ASSET_NAMES: [&str; 1] = ["rootfs.erofs"];

fn canonical_rootfs_asset_name(assets: &HashMap<String, AssetEntry>) -> Option<&'static str> {
    ROOTFS_ASSET_NAMES
        .iter()
        .copied()
        .find(|name| assets.contains_key(*name))
}

/// Load `manifest.json` from the assets dir (installed layout) or its parent
/// (dev tree layout where `assets` is already `assets/<arch>/`). Returns
/// `None` on missing file, read error, parse error, or schema mismatch --
/// profile-selected asset hashes remain the runtime authority.
pub fn load_manifest_for_assets(assets: &Path) -> Option<ManifestV2> {
    let mut candidates: Vec<PathBuf> = vec![assets.join("manifest.json")];
    if let Some(parent) = assets.parent() {
        candidates.push(parent.join("manifest.json"));
    }
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match ManifestV2::from_json(&content) {
                Ok(m) => return Some(m),
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "manifest parse failed");
                    return None;
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "manifest read failed");
                return None;
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Hash-based filename derivation
// ---------------------------------------------------------------------------

/// Derive a hash-based filename from a logical asset name and its blake3 hash.
///
/// Splits on the first `.` to get stem and extension:
/// - `"vmlinuz"` + `"2c0bd752..."` -> `"vmlinuz-2c0bd752db929642"`
/// - `"initrd.img"` + `"e5e910e9..."` -> `"initrd-e5e910e9ab38b873.img"`
/// - `"rootfs.erofs"` + `"89eb92b8..."` -> `"rootfs-89eb92b83534d9d0.erofs"`
pub fn hash_filename(logical_name: &str, hash: &str) -> String {
    let prefix = &hash[..16.min(hash.len())];
    if let Some(dot_pos) = logical_name.find('.') {
        let stem = &logical_name[..dot_pos];
        let ext = &logical_name[dot_pos..];
        format!("{stem}-{prefix}{ext}")
    } else {
        format!("{logical_name}-{prefix}")
    }
}

fn is_hash_tagged_asset_filename(filename: &str) -> bool {
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    let Some((_, suffix)) = stem.rsplit_once('-') else {
        return false;
    };
    suffix.len() == 16 && suffix.chars().all(|ch| ch.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------
// ManifestV2 implementation
// ---------------------------------------------------------------------------

impl ManifestV2 {
    /// Parse a manifest from JSON.
    pub fn from_json(content: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(content).context("failed to parse manifest JSON")?;
        let manifest: ManifestV2 = if value.get("format").is_some() {
            serde_json::from_value(value).context("failed to parse manifest v2 JSON")?
        } else {
            manifest_v2_from_release_graph(&value)?
        };
        if manifest.format != 2 {
            bail!("expected manifest format 2, got {}", manifest.format);
        }
        if manifest.refresh_policy.trim().is_empty() {
            bail!("manifest refresh_policy must not be empty");
        }
        validate_version(&manifest.assets.current)?;
        validate_version(&manifest.binaries.current)?;
        for (version, release) in &manifest.assets.releases {
            validate_version(version)?;
            for (arch, assets) in &release.arches {
                validate_filename(arch).with_context(|| format!("invalid asset architecture key {arch:?}"))?;
                ensure!(!assets.is_empty(), "asset release {version} has empty arch entry");
                for (name, entry) in assets {
                    validate_filename(name)?;
                    validate_hash(&entry.hash)?;
                    if !entry.sha256.is_empty() {
                        validate_sha256(&entry.sha256)?;
                    }
                }
            }
        }
        for version in manifest.binaries.releases.keys() {
            validate_version(version)?;
        }
        Ok(manifest)
    }

    /// Resolve asset file paths for a given binary version and architecture.
    ///
    /// Finds the best compatible asset release and returns hash-based file paths.
    pub fn resolve(&self, binary_version: &str, arch: &str, base_dir: &Path) -> Result<ResolvedAssets> {
        let asset_version = pick_asset_version(self, binary_version)?;

        let release = self
            .assets
            .releases
            .get(&asset_version)
            .with_context(|| format!("asset version {} not found in manifest", asset_version))?;
        let arch_assets = release
            .arches
            .get(arch)
            .with_context(|| format!("arch {} not found in asset release {}", arch, asset_version))?;

        let resolve_one = |name: &str| -> Result<PathBuf> {
            let entry = arch_assets
                .get(name)
                .with_context(|| format!("{} not found in asset release {} / {}", name, asset_version, arch))?;
            let hname = hash_filename(name, &entry.hash);
            // Check flat layout first (base_dir/{hash}), then arch subdir (base_dir/{arch}/{hash})
            let flat = base_dir.join(&hname);
            if flat.exists() {
                return Ok(flat);
            }
            let arch_path = base_dir.join(arch).join(&hname);
            if arch_path.exists() {
                return Ok(arch_path);
            }
            // Return the flat path (caller will report the error)
            Ok(flat)
        };
        let rootfs_name = canonical_rootfs_asset_name(arch_assets)
            .with_context(|| format!("rootfs not found in asset release {} / {}", asset_version, arch))?;

        Ok(ResolvedAssets {
            kernel: resolve_one("vmlinuz")?,
            initrd: resolve_one("initrd.img")?,
            rootfs: resolve_one(rootfs_name)?,
            asset_version,
        })
    }

    /// Expected hashes for the canonical boot triple (kernel/initrd/rootfs)
    /// from the current asset release on the given arch. Returns `None` if
    /// the current release or arch entry is missing, or if any of the three
    /// canonical filenames is absent from that arch's asset map.
    pub fn expected_hashes_current(&self, arch: &str) -> Option<ExpectedAssetHashes> {
        let release = self.assets.releases.get(&self.assets.current)?;
        let assets = release.arches.get(arch)?;
        Some(ExpectedAssetHashes {
            kernel: assets.get("vmlinuz")?.hash.clone(),
            initrd: assets.get("initrd.img")?.hash.clone(),
            rootfs: assets.get(canonical_rootfs_asset_name(assets)?)?.hash.clone(),
        })
    }

    /// Merge another manifest into this one, preserving existing entries.
    pub fn merge(&mut self, other: &ManifestV2) {
        for (version, entry) in &other.assets.releases {
            self.assets
                .releases
                .entry(version.clone())
                .or_insert_with(|| entry.clone());
        }
        if compare_versions(&other.assets.current, &self.assets.current).is_gt() {
            self.assets.current = other.assets.current.clone();
        }
        for (version, entry) in &other.binaries.releases {
            self.binaries
                .releases
                .entry(version.clone())
                .or_insert_with(|| entry.clone());
        }
        if compare_versions(&other.binaries.current, &self.binaries.current).is_gt() {
            self.binaries.current = other.binaries.current.clone();
        }
    }
}

/// Convert the public release graph into the runtime-only v2 view in memory.
///
/// The installed `manifest.json` remains the exact public document. This
/// adapter exists only because the boot resolver needs the compact v2 asset
/// index; it must never be serialized back over the installed manifest.
fn manifest_v2_from_release_graph(value: &serde_json::Value) -> Result<ManifestV2> {
    // Validate and retain the complete graph state before deriving the compact
    // compatibility view used by legacy boot-resolution callers.
    release_graph_profile_state(value)?;
    let profiles = value
        .get("profiles")
        .and_then(serde_json::Value::as_object)
        .context("manifest is neither format 2 nor a release graph with profiles")?;
    if profiles.is_empty() {
        bail!("release graph contains no profiles");
    }

    // Every profile owns its own images, so a channel-wide pointer can name at
    // most one of them. Emit a release for each and let `current` name only the
    // default, so no profile's assets are discarded on the way through.
    let usable: Vec<(&String, &serde_json::Value)> = profiles
        .iter()
        .filter(|(_, profile)| profile.get("status").and_then(serde_json::Value::as_str) != Some("revoked"))
        .collect();
    if usable.is_empty() {
        bail!("release graph contains no usable profile");
    }
    let default_first = {
        let mut ordered = usable.clone();
        ordered.sort_by_key(|(id, _)| (id.as_str() != "default", (*id).clone()));
        ordered
    };

    let mut releases: HashMap<String, AssetRelease> = HashMap::new();
    let mut current_version: Option<String> = None;
    for (profile_id, profile) in default_first {
        let (asset_version, min_binary, arches) = profile_asset_release(profile)?;
        // Two profiles may share an image revision while pinning different
        // images, so the revision alone cannot key them.
        let key = if releases.contains_key(&asset_version) {
            format!("{asset_version}+{profile_id}")
        } else {
            asset_version
        };
        if current_version.is_none() {
            current_version = Some(key.clone());
        }
        releases.insert(
            key,
            AssetRelease {
                date: String::new(),
                deprecated: false,
                deprecated_date: None,
                min_binary,
                arches,
            },
        );
    }
    let asset_version = current_version.context("release graph contains no usable profile")?;

    let packages = value
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let binary_version = packages
        .iter()
        .filter(|package| package.get("status").and_then(serde_json::Value::as_str) != Some("revoked"))
        .find_map(|package| package.get("version").and_then(serde_json::Value::as_str))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string();

    Ok(ManifestV2 {
        format: 2,
        refresh_policy: value
            .get("refresh_policy")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("24h")
            .to_string(),
        asset_base: None,
        assets: AssetsSection {
            current: asset_version.clone(),
            releases,
        },
        binaries: BinariesSection {
            current: binary_version.clone(),
            releases: HashMap::from([(
                binary_version.clone(),
                BinaryRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    min_assets: asset_version,
                    version: binary_version,
                    files: Vec::new(),
                },
            )]),
        },
    })
}

/// One profile's assets: architecture -> logical asset name -> entry.
type ProfileArchAssets = HashMap<String, HashMap<String, AssetEntry>>;

/// One profile's image set: its asset version, minimum binary, and per-arch
/// assets. Each profile carries its own, which is why no channel-wide pointer
/// can stand in for them.
fn profile_asset_release(profile: &serde_json::Value) -> Result<(String, String, ProfileArchAssets)> {
    let min_binary = profile
        .get("min_capsem_version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let architectures = profile
        .get("architectures")
        .and_then(serde_json::Value::as_array)
        .context("release graph profile is missing architectures")?;
    let mut arches = HashMap::new();
    let mut asset_version = None;
    for architecture in architectures {
        let arch = architecture
            .get("architecture")
            .and_then(serde_json::Value::as_str)
            .context("release graph profile architecture is missing architecture")?;
        let image_revision = architecture
            .get("image_revision")
            .and_then(serde_json::Value::as_str)
            .context("release graph profile architecture is missing image_revision")?;
        match asset_version.as_deref() {
            Some(expected) if expected != image_revision => {
                bail!("release graph profile image revisions disagree: {expected} != {image_revision} for {arch}")
            }
            None => asset_version = Some(image_revision.to_string()),
            _ => {}
        }
        let images = architecture
            .get("images")
            .and_then(serde_json::Value::as_array)
            .context("release graph profile architecture is missing images")?;
        let mut assets = HashMap::new();
        for image in images
            .iter()
            .filter(|image| image.get("status").and_then(serde_json::Value::as_str) != Some("revoked"))
        {
            let Some(kind) = image.get("kind").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if !matches!(kind, "kernel" | "initrd" | "rootfs") {
                continue;
            }
            let name = image
                .get("name")
                .and_then(serde_json::Value::as_str)
                .context("release graph image is missing name")?;
            let digest = image.get("digest").context("release graph image is missing digest")?;
            assets.insert(
                name.to_string(),
                AssetEntry {
                    hash: digest
                        .get("blake3")
                        .and_then(serde_json::Value::as_str)
                        .context("release graph image is missing BLAKE3")?
                        .to_string(),
                    sha256: digest
                        .get("sha256")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    size: image
                        .get("bytes")
                        .and_then(serde_json::Value::as_u64)
                        .context("release graph image is missing byte size")?,
                },
            );
        }
        for required in ["vmlinuz", "initrd.img", "rootfs.erofs"] {
            if !assets.contains_key(required) {
                bail!("release graph profile architecture {arch} is missing {required}");
            }
        }
        arches.insert(arch.to_string(), assets);
    }
    if arches.is_empty() {
        bail!("release graph profile contains no usable architectures");
    }
    let asset_version = asset_version.context("release graph profile contains no image revision")?;

    Ok((asset_version, min_binary, arches))
}

/// Parse and fingerprint every profile-owned identity in a public graph.
///
/// Object keys and semantically unordered artifact lists are canonicalized so
/// formatting or JSON key order cannot manufacture an update. Membership,
/// revisions, config, evidence, and image identity all participate in the
/// catalog revision; only profile/image identity participates in the image
/// revision.
pub fn release_graph_profile_state(value: &serde_json::Value) -> Result<ReleaseGraphProfileState> {
    let profiles = value
        .get("profiles")
        .and_then(serde_json::Value::as_object)
        .context("release graph is missing profiles")?;
    if profiles.is_empty() {
        bail!("release graph contains no profiles");
    }

    let mut identities = BTreeMap::new();
    let mut catalog_scope = BTreeMap::new();
    let mut images_scope = BTreeMap::new();
    let mut usable_profiles = 0usize;

    for (profile_id, profile) in profiles {
        validate_version(profile_id).with_context(|| format!("release graph profile id {profile_id} is invalid"))?;
        let revision = profile
            .get("revision")
            .and_then(serde_json::Value::as_str)
            .context("release graph profile is missing revision")?
            .to_string();
        validate_version(&revision)
            .with_context(|| format!("release graph profile {profile_id} has invalid revision {revision}"))?;
        let status = profile
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("current")
            .to_string();
        let revoked = status.eq_ignore_ascii_case("revoked");
        if !revoked {
            usable_profiles += 1;
        }

        let architectures = profile
            .get("architectures")
            .and_then(serde_json::Value::as_array)
            .context("release graph profile is missing architectures")?;
        if architectures.is_empty() && !revoked {
            bail!("release graph profile {profile_id} contains no architectures");
        }

        let mut architecture_names = BTreeSet::new();
        let mut config_scope = BTreeMap::new();
        let mut evidence_scope = BTreeMap::new();
        let mut profile_images_scope = BTreeMap::new();
        for architecture in architectures {
            let arch = architecture
                .get("architecture")
                .and_then(serde_json::Value::as_str)
                .context("release graph profile architecture is missing architecture")?;
            if !architecture_names.insert(arch.to_string()) {
                bail!("release graph profile {profile_id} repeats architecture {arch}");
            }
            let image_revision = architecture
                .get("image_revision")
                .and_then(serde_json::Value::as_str)
                .context("release graph profile architecture is missing image_revision")?;
            validate_version(image_revision).with_context(|| {
                format!("release graph profile {profile_id} architecture {arch} has invalid image revision")
            })?;

            let configs = canonical_active_artifacts(
                architecture.get("config"),
                &format!("release graph profile {profile_id} architecture {arch} config"),
                false,
            )?;
            let evidence = canonical_active_artifacts(
                architecture.get("evidence"),
                &format!("release graph profile {profile_id} architecture {arch} evidence"),
                false,
            )?;
            let images = canonical_active_artifacts(
                architecture.get("images"),
                &format!("release graph profile {profile_id} architecture {arch} images"),
                true,
            )?;
            if !revoked {
                let image_kinds: BTreeSet<&str> = images
                    .iter()
                    .filter_map(|image| image.get("kind").and_then(serde_json::Value::as_str))
                    .collect();
                for required in ["kernel", "initrd", "rootfs"] {
                    if !image_kinds.contains(required) {
                        bail!("release graph profile {profile_id} architecture {arch} is missing {required} image");
                    }
                }
            }

            config_scope.insert(arch.to_string(), configs);
            evidence_scope.insert(arch.to_string(), evidence);
            profile_images_scope.insert(
                arch.to_string(),
                serde_json::json!({
                    "image_revision": image_revision,
                    "images": images,
                }),
            );
        }

        let config_revision = state_revision(
            "config",
            &serde_json::to_value(&config_scope).context("serialize profile config state")?,
        );
        let evidence_revision = state_revision(
            "evidence",
            &serde_json::to_value(&evidence_scope).context("serialize profile evidence state")?,
        );
        let profile_images_revision = state_revision(
            "images",
            &serde_json::to_value(&profile_images_scope).context("serialize profile image state")?,
        );
        let identity = ReleaseGraphProfileIdentity {
            revision: revision.clone(),
            status: status.clone(),
            config_revision: config_revision.clone(),
            evidence_revision: evidence_revision.clone(),
            images_revision: profile_images_revision.clone(),
            architectures: architecture_names.into_iter().collect(),
        };
        let mut metadata = profile.clone();
        if let Some(object) = metadata.as_object_mut() {
            object.remove("architectures");
        }
        catalog_scope.insert(
            profile_id.clone(),
            serde_json::json!({
                "metadata": metadata,
                "config_revision": config_revision,
                "evidence_revision": evidence_revision,
                "images_revision": profile_images_revision,
            }),
        );
        images_scope.insert(
            profile_id.clone(),
            serde_json::json!({
                "status": status,
                "architectures": profile_images_scope,
            }),
        );
        identities.insert(profile_id.clone(), identity);
    }
    if usable_profiles == 0 {
        bail!("release graph contains no usable profile");
    }

    Ok(ReleaseGraphProfileState {
        catalog_revision: state_revision(
            "catalog",
            &serde_json::to_value(catalog_scope).context("serialize release graph profile catalog state")?,
        ),
        images_revision: state_revision(
            "images",
            &serde_json::to_value(images_scope).context("serialize release graph image catalog state")?,
        ),
        profiles: identities,
    })
}

fn canonical_active_artifacts(
    value: Option<&serde_json::Value>,
    context: &str,
    validate_images: bool,
) -> Result<Vec<serde_json::Value>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let artifacts = value
        .as_array()
        .with_context(|| format!("{context} must be an array"))?;
    let mut canonical = Vec::new();
    for artifact in artifacts.iter().filter(|artifact| {
        artifact
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|status| !status.eq_ignore_ascii_case("revoked"))
    }) {
        let digest = artifact
            .get("digest")
            .with_context(|| format!("{context} artifact is missing digest"))?;
        let blake3 = digest
            .get("blake3")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("{context} artifact is missing BLAKE3"))?;
        validate_hash(blake3)?;
        let sha256 = digest
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("{context} artifact is missing SHA-256"))?;
        validate_sha256(sha256)?;
        artifact
            .get("bytes")
            .and_then(serde_json::Value::as_u64)
            .with_context(|| format!("{context} artifact is missing byte size"))?;
        if validate_images {
            artifact
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("{context} image is missing kind"))?;
            artifact
                .get("name")
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("{context} image is missing name"))?;
        }
        canonical.push(artifact.clone());
    }
    canonical.sort_by_key(canonical_json);
    Ok(canonical)
}

fn state_revision(prefix: &str, value: &serde_json::Value) -> String {
    let hash = blake3::hash(canonical_json(value).as_bytes()).to_hex().to_string();
    format!("{prefix}-{}", &hash[..16])
}

fn canonical_json(value: &serde_json::Value) -> String {
    fn write(value: &serde_json::Value, output: &mut String) {
        match value {
            serde_json::Value::Null => output.push_str("null"),
            serde_json::Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            serde_json::Value::Number(value) => output.push_str(&value.to_string()),
            serde_json::Value::String(value) => {
                output.push_str(&serde_json::to_string(value).expect("JSON strings serialize"));
            }
            serde_json::Value::Array(values) => {
                output.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    write(value, output);
                }
                output.push(']');
            }
            serde_json::Value::Object(values) => {
                output.push('{');
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_by_key(|(key, _)| key.as_str());
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push_str(&serde_json::to_string(key).expect("JSON keys serialize"));
                    output.push(':');
                    write(value, output);
                }
                output.push('}');
            }
        }
    }

    let mut output = String::new();
    write(value, &mut output);
    output
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Compute the blake3 hash of a file.
/// Copy `source` to `dest` and return the blake3 hex of the bytes written.
fn copy_hashed(source: &Path, dest: &Path) -> Result<String> {
    use std::io::{Read, Write};
    let mut from = std::fs::File::open(source).with_context(|| format!("cannot open {}", source.display()))?;
    let mut to = std::fs::File::create(dest).with_context(|| format!("cannot create {}", dest.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = from.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        to.write_all(&buf[..n])?;
    }
    to.sync_all()?;
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        use std::io::Read;
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Return the default assets directory.
///
/// Resolves via [`capsem_foundation::paths::capsem_home_opt`], so the `CAPSEM_HOME` /
/// `CAPSEM_ASSETS_DIR` env overrides are honored.
pub fn default_assets_dir() -> Option<PathBuf> {
    // Honor CAPSEM_ASSETS_DIR first, then <capsem_home>/assets.
    if let Ok(v) = std::env::var("CAPSEM_ASSETS_DIR") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    capsem_foundation::paths::capsem_home_opt().map(|h| h.join("assets"))
}

/// Build the GitHub Releases download base URL for the given **binary**
/// version.
///
/// This is retained for binary update/download metadata. VM assets use
/// [`asset_release_base_url`] so the asset track can move independently of tag
/// releases.
pub fn release_url(binary_version: &str) -> String {
    let base = std::env::var("CAPSEM_RELEASE_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://github.com/google/capsem/releases/download".into());
    format!("{}/v{binary_version}", base.trim_end_matches('/'))
}

/// Default immutable VM asset blob base.
///
/// The stable channel manifest lives at
/// `https://release.capsem.org/assets/stable/manifest.json`, while blobs live
/// under `assets/releases/<asset-version>/...` so older manifests continue to
/// hydrate even after `stable` advances.
pub fn asset_release_base_url() -> String {
    std::env::var("CAPSEM_ASSET_BASE_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://release.capsem.org/assets/releases".into())
        .trim_end_matches('/')
        .to_string()
}

/// Derive the immutable asset blob base from a manifest URL.
///
/// Canonical channel manifests use `<prefix>/assets/<channel>/manifest.json`
/// and resolve blobs from `<prefix>/assets/releases/<asset-version>/...`.
pub fn asset_release_base_url_from_manifest_url(manifest_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(manifest_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut segments: Vec<String> = url
        .path_segments()
        .map(|segments| segments.map(ToOwned::to_owned).collect())
        .unwrap_or_default();
    if segments.len() < 3 || segments.last().map(String::as_str) != Some("manifest.json") {
        return None;
    }
    let channel_index = segments.len() - 2;
    if channel_index == 0 || segments[channel_index - 1] != "assets" {
        return None;
    }
    segments.truncate(channel_index);
    segments.push("releases".to_string());
    let mut out = url;
    out.set_path(&segments.join("/"));
    Some(out.as_str().trim_end_matches('/').to_string())
}

/// Derive a remote asset blob base from `manifest-metadata.json`, when present.
pub fn asset_release_base_url_from_manifest_metadata(assets_dir: &Path) -> Result<Option<String>> {
    let metadata_path = assets_dir.join("manifest-metadata.json");
    if !metadata_path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&metadata_path).with_context(|| format!("read {}", metadata_path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&content).with_context(|| format!("parse {}", metadata_path.display()))?;
    let Some(source) = value.get("manifest_url").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    Ok(asset_release_base_url_from_manifest_url(source))
}

fn remote_asset_release_base_url(manifest: &ManifestV2, assets_dir: &Path) -> Result<String> {
    let asset_base_url = manifest
        .asset_base
        .clone()
        .or(asset_release_base_url_from_manifest_metadata(assets_dir)?)
        .unwrap_or_else(asset_release_base_url);
    let asset_base_url = asset_base_url.trim_end_matches('/').to_string();
    let validation_url = asset_base_url.replace("{asset_version}", "0");
    let parsed = reqwest::Url::parse(&validation_url).map_err(|_| {
        anyhow::anyhow!("asset base URL must be a URL: use https://... or http://..., got {asset_base_url}")
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!(
            "unsupported asset base URL scheme {}: use https:// or http://",
            parsed.scheme()
        );
    }
    Ok(asset_base_url)
}

/// Full per-asset download URL:
/// `{asset_release_base_url}/{asset_version}/{arch}-{logical_name}`.
///
/// Single source of truth for the URL `download_missing_assets` constructs.
/// Pinned by unit tests so the layout the binary fetches stays in lock-step
/// with the layout `release-assets.yaml` deploys.
pub fn asset_download_url(asset_version: &str, arch: &str, logical_name: &str) -> String {
    asset_download_url_with_base(&asset_release_base_url(), asset_version, arch, logical_name)
}

pub fn asset_download_url_with_base(
    asset_base_url: &str,
    asset_version: &str,
    arch: &str,
    logical_name: &str,
) -> String {
    let asset_base_url = asset_base_url.trim_end_matches('/');
    let version_base = if asset_base_url.contains("{asset_version}") {
        asset_base_url.replace("{asset_version}", asset_version)
    } else {
        format!("{asset_base_url}/{asset_version}")
    };
    format!("{}/{}-{}", version_base.trim_end_matches('/'), arch, logical_name)
}

fn asset_storage_dir(base_dir: &Path, arch: &str) -> PathBuf {
    if base_dir.file_name().and_then(|name| name.to_str()) == Some(arch) {
        base_dir.to_path_buf()
    } else {
        base_dir.join(arch)
    }
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

/// Remove hash-named asset files not referenced by any non-deprecated release.
///
/// Returns paths that were removed.
pub fn cleanup_unused_assets(base_dir: &Path, manifest: &ManifestV2) -> Result<Vec<PathBuf>> {
    cleanup_unused_assets_preserving(base_dir, manifest, std::iter::empty::<String>())
}

/// Remove hash-named asset files not referenced by any non-deprecated release
/// or explicitly listed in `preserve_filenames`. Both a direct architecture
/// directory and an asset root containing manifest-declared architecture
/// directories are supported.
///
/// `preserve_filenames` is intentionally filename-only. Callers that own
/// higher-level contracts, such as profiles or saved VMs, translate those
/// contracts into hash-prefixed asset basenames before cleanup.
pub fn cleanup_unused_assets_preserving<I, S>(
    base_dir: &Path,
    manifest: &ManifestV2,
    preserve_filenames: I,
) -> Result<Vec<PathBuf>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut architecture_dirs = BTreeSet::new();

    for release in manifest.assets.releases.values() {
        if release.deprecated {
            continue;
        }
        for (arch, assets) in &release.arches {
            validate_filename(arch).with_context(|| format!("invalid asset architecture directory {arch}"))?;
            architecture_dirs.insert(arch.clone());
            for (name, entry) in assets {
                referenced.insert(hash_filename(name, &entry.hash));
            }
        }
    }
    referenced.extend(
        preserve_filenames
            .into_iter()
            .map(|filename| filename.as_ref().to_string()),
    );

    let mut removed = Vec::new();
    if !base_dir.exists() {
        return Ok(removed);
    }

    cleanup_hash_tagged_assets_in_dir(base_dir, &referenced, &mut removed)?;
    for arch in architecture_dirs {
        cleanup_hash_tagged_assets_in_dir(&base_dir.join(arch), &referenced, &mut removed)?;
    }

    Ok(removed)
}

fn cleanup_hash_tagged_assets_in_dir(
    dir: &Path,
    referenced: &std::collections::HashSet<String>,
    removed: &mut Vec<PathBuf>,
) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == "manifest.json"
            || name_str == "manifest-metadata.json"
            || name_str.starts_with('.')
            || name_str.ends_with(".tmp")
        {
            continue;
        }

        // Skip directories (arch subdirs like arm64/, x86_64/)
        if entry.file_type()?.is_dir() {
            continue;
        }

        // Remove hash-named files not referenced by any release.
        if is_hash_tagged_asset_filename(&name_str) && !referenced.contains(name_str.as_ref()) {
            info!(path = %entry.path().display(), "removing unreferenced asset");
            std::fs::remove_file(entry.path())?;
            removed.push(entry.path());
        }
    }

    Ok(())
}

/// Every asset this arch needs on disk, across every compatible release.
///
/// One rule, one function. The local-copy and download paths each resolved
/// their own single release, and each therefore materialized one profile's
/// images while the manifest promised several. A channel's profiles own their
/// images, so the channel pointer names at most one of them -- the rest went
/// missing on a fresh install, and the profile that sorted first became an
/// unbootable default.
fn arch_assets_to_materialize<'m>(
    manifest: &'m ManifestV2,
    binary_version: &str,
    arch: &str,
) -> Result<Vec<(&'m str, &'m String, &'m AssetEntry)>> {
    let versions = compatible_asset_versions(manifest, binary_version)?;
    // Keyed by what the bytes are called on disk -- logical name *and* hash.
    // Two profiles legitimately ship a different `vmlinuz`, and keying by name
    // alone silently keeps one of them: the same missing-kernel install this
    // function exists to prevent.
    let mut wanted: BTreeMap<(&str, &str), (&str, &String, &AssetEntry)> = BTreeMap::new();
    for asset_version in &versions {
        // A release that does not build for this arch is not this arch's
        // problem; only every one of them missing is.
        let Some(assets) = manifest.assets.releases[*asset_version].arches.get(arch) else {
            continue;
        };
        for (name, entry) in assets {
            wanted.insert(
                (name.as_str(), entry.hash.as_str()),
                (asset_version.as_str(), name, entry),
            );
        }
    }
    if wanted.is_empty() {
        bail!(
            "arch {arch} not found in any asset release compatible with binary \
             {binary_version} (checked {})",
            versions
                .iter()
                .map(|version| version.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(wanted.into_values().collect())
}

/// Every asset release this binary can boot, sorted for a stable answer.
///
/// One rule in one function: booting resolves a single release and hydration
/// must materialize all of them, and the two disagreeing about which are
/// compatible is how an install ends up missing exactly the assets it is about
/// to ask for.
fn compatible_asset_versions<'m>(manifest: &'m ManifestV2, binary_version: &str) -> Result<Vec<&'m String>> {
    // Empty min_assets means "no compatibility constraint declared".
    let min_assets = manifest
        .binaries
        .releases
        .get(binary_version)
        .map(|release| release.min_assets.as_str())
        .unwrap_or("");

    let mut compatible: Vec<&String> = manifest
        .assets
        .releases
        .iter()
        .filter(|(asset_version, release)| {
            !release.deprecated
                && version_at_least(asset_version, min_assets)
                && (release.min_binary.is_empty() || version_at_least(binary_version, &release.min_binary))
        })
        .map(|(asset_version, _)| asset_version)
        .collect();
    compatible.sort();

    if compatible.is_empty() {
        bail!(
            "no compatible asset release for binary {binary_version} (min_assets: {})",
            if min_assets.is_empty() {
                "unspecified"
            } else {
                min_assets
            }
        );
    }
    Ok(compatible)
}

/// Pick the asset version that [`ManifestV2::resolve`] would pick for a
/// given binary version -- the newest of the compatible set.
fn pick_asset_version(manifest: &ManifestV2, binary_version: &str) -> Result<String> {
    let mut best: Option<&String> = None;
    for asset_version in compatible_asset_versions(manifest, binary_version)? {
        if best.is_none_or(|current| version_at_least(asset_version, current)) {
            best = Some(asset_version);
        }
    }
    let best = best.cloned();

    let min_assets = manifest
        .binaries
        .releases
        .get(binary_version)
        .map(|release| release.min_assets.as_str())
        .unwrap_or("");
    best.ok_or_else(|| {
        anyhow::anyhow!(
            "no compatible asset release for binary {binary_version} (min_assets: {})",
            if min_assets.is_empty() {
                "unspecified"
            } else {
                min_assets
            }
        )
    })
}

fn version_at_least(actual: &str, minimum: &str) -> bool {
    if minimum.is_empty() {
        return true;
    }
    compare_versions(actual, minimum).is_ge()
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    match (numeric_version_parts(left), numeric_version_parts(right)) {
        (Some(left), Some(right)) => compare_numeric_versions(&left, &right),
        _ => left.cmp(right),
    }
}

fn numeric_version_parts(version: &str) -> Option<Vec<u64>> {
    let mut parts = Vec::new();
    for part in version.split('.') {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse().ok()?);
    }
    Some(parts)
}

fn compare_numeric_versions(left: &[u64], right: &[u64]) -> std::cmp::Ordering {
    let width = left.len().max(right.len());
    for index in 0..width {
        let left = left.get(index).copied().unwrap_or_default();
        let right = right.get(index).copied().unwrap_or_default();
        match left.cmp(&right) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
