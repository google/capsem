use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, bail, Context, Result};
pub use capsem_assets::asset_manager::{Architecture, PackageArchitecture};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

use crate::source_commit::{deserialize_optional, SourceCommit};

const REQUIRED_RUNTIME_IMAGE_ARTIFACT_KINDS: [RuntimeImageArtifactKind; 3] = [
    RuntimeImageArtifactKind::Kernel,
    RuntimeImageArtifactKind::Initrd,
    RuntimeImageArtifactKind::Rootfs,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Current,
    Supported,
    Deprecated,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestSet {
    pub sha256: String,
    pub blake3: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRecord {
    pub version: String,
    pub status: Status,
    pub url: String,
    pub digest: DigestSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_capsem_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_capsem_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    MacosPkg,
    DebianPackage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeImageArtifactKind {
    Kernel,
    Initrd,
    Rootfs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseLedgerKind {
    Manifest,
    Package,
    Binary,
    Runtime,
    RuntimeImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ReleaseLedgerArchitecture {
    Package(PackageArchitecture),
    Machine(Architecture),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub url: String,
    pub bytes: u64,
    pub digest: DigestSet,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInventoryRow {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_commit: Option<SourceCommit>,
    pub kind: PackageKind,
    pub platform: String,
    pub architecture: PackageArchitecture,
    pub url: String,
    pub bytes: u64,
    pub digest: DigestSet,
    pub status: Status,
    #[serde(default)]
    pub binaries: Vec<BinaryInventoryRow>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryInventoryRow {
    pub name: String,
    pub version: String,
    pub description: String,
    pub installed_path: String,
    pub platform: String,
    pub architecture: PackageArchitecture,
    pub bytes: u64,
    pub digest: DigestSet,
    pub status: Status,
    pub sbom_component_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedExecutableFile {
    pub name: String,
    pub description: String,
    pub installed_path: String,
    pub bytes: Vec<u8>,
}

/// One channel's public release graph: the package cohort and the runtime.
///
/// `runtime` is absent until the channel's first runtime release; a graph
/// carries no other VM release unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub version: String,
    pub channel: String,
    #[serde(default = "default_status_current")]
    pub status: Status,
    #[serde(default)]
    pub packages: Vec<PackageInventoryRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeDocument>,
}

/// The VM runtime a channel publishes: one image set per architecture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDocument {
    pub revision: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_commit: Option<SourceCommit>,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_capsem_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_capsem_version: Option<String>,
    pub architectures: Vec<RuntimeArchitecture>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoftwareInventoryRow {
    pub name: String,
    pub version: String,
    pub source: String,
    pub architecture: Architecture,
    pub evidence: String,
    pub digest: DigestSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeArchitecture {
    pub architecture: Architecture,
    pub package_inventory_revision: String,
    pub image_revision: String,
    #[serde(default)]
    pub software: Vec<SoftwareInventoryRow>,
    pub images: Vec<RuntimeImageArtifactRef>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeImageArtifactRef {
    pub kind: RuntimeImageArtifactKind,
    pub name: String,
    pub url: String,
    pub bytes: u64,
    pub digest: DigestSet,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelRecord {
    pub label: String,
    pub manifests: Vec<ManifestRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelsCatalog {
    pub version: u64,
    pub generated_at: String,
    pub channels: BTreeMap<String, ChannelRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseLedger {
    pub entries: Vec<ReleaseLedgerEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseLedgerEntry {
    pub channel: String,
    pub kind: ReleaseLedgerKind,
    pub name: String,
    pub version: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<ReleaseLedgerArchitecture>,
}

impl DigestSet {
    fn validate(&self, context: &str) -> Result<()> {
        validate_hex_digest(&self.sha256, 64).with_context(|| format!("{context} sha256 digest is invalid"))?;
        validate_hex_digest(&self.blake3, 64).with_context(|| format!("{context} blake3 digest is invalid"))?;
        Ok(())
    }

    pub fn verify_bytes(&self, bytes: &[u8], context: &str) -> Result<()> {
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        if sha256 != self.sha256 {
            bail!("{context} sha256 mismatch");
        }
        let blake3 = blake3::hash(bytes).to_hex().to_string();
        if blake3 != self.blake3 {
            bail!("{context} blake3 mismatch");
        }
        Ok(())
    }
}

impl ManifestRecord {
    fn validate(&self, channel: &str) -> Result<()> {
        if self.version.trim().is_empty() {
            bail!("channel {channel} manifest version must not be empty");
        }
        if self.version.contains('/') || self.version.contains('\\') || self.version.contains("..") {
            bail!(
                "channel {channel} manifest version contains a path separator: {}",
                self.version
            );
        }
        if self.url.trim().is_empty() {
            bail!("channel {channel} manifest {} url must not be empty", self.version);
        }
        if !(self.url.starts_with('/') || self.url.starts_with("https://") || self.url.starts_with("http://")) {
            bail!(
                "channel {channel} manifest {} url must be release-site relative or http(s): {}",
                self.version,
                self.url
            );
        }
        self.digest
            .validate(&format!("channel {channel} manifest {}", self.version))?;
        Ok(())
    }
}

impl EvidenceRef {
    fn validate(&self, context: &str) -> Result<()> {
        if self.kind.trim().is_empty() {
            bail!("{context} evidence kind must not be empty");
        }
        validate_url_like(&self.url).with_context(|| format!("{context} evidence url is invalid"))?;
        self.digest.validate(&format!("{context} evidence {}", self.kind))?;
        Ok(())
    }
}

impl PackageInventoryRow {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("package inventory row name must not be empty");
        }
        if self.version.trim().is_empty() {
            bail!("package {} version must not be empty", self.name);
        }
        if self.platform.trim().is_empty() {
            bail!("package {} platform must not be empty", self.name);
        }
        let expected_architecture = PackageArchitecture::from_package_name(&self.name)?;
        if expected_architecture != self.architecture {
            bail!(
                "package {} filename architecture does not match graph architecture",
                self.name
            );
        }
        match self.kind {
            PackageKind::MacosPkg if self.platform != "macos" => {
                bail!("macOS package {} platform must be macos", self.name);
            }
            PackageKind::MacosPkg if !self.name.ends_with(".pkg") => {
                bail!("macOS package {} name must end in .pkg", self.name);
            }
            PackageKind::DebianPackage if self.platform != "linux" => {
                bail!("Debian package {} platform must be linux", self.name);
            }
            PackageKind::DebianPackage if !self.name.ends_with(".deb") => {
                bail!("Debian package {} name must end in .deb", self.name);
            }
            _ => {}
        }
        validate_url_like(&self.url)
            .with_context(|| format!("package {} {} download url is invalid", self.name, self.version))?;
        if self.bytes == 0 {
            bail!("package {} bytes must be non-zero", self.name);
        }
        self.digest
            .validate(&format!("package {} {}", self.name, self.version))?;
        if self.binaries.is_empty() {
            bail!("package {} must list packaged binaries", self.name);
        }
        for binary in &self.binaries {
            binary.validate()?;
            if binary.version != self.version {
                bail!(
                    "package {} binary {} version mismatch: expected {}, got {}",
                    self.name,
                    binary.name,
                    self.version,
                    binary.version
                );
            }
            if binary.platform != self.platform {
                bail!(
                    "package {} binary {} platform mismatch: expected {}, got {}",
                    self.name,
                    binary.name,
                    self.platform,
                    binary.platform
                );
            }
            if binary.architecture != self.architecture {
                bail!("package {} binary {} architecture mismatch", self.name, binary.name);
            }
        }
        for evidence in &self.evidence {
            evidence.validate(&format!("package {} {}", self.name, self.version))?;
        }
        if !self.evidence.iter().any(|item| item.kind == "sbom") {
            bail!("package {} must include package SBOM evidence", self.name);
        }
        Ok(())
    }
}

impl BinaryInventoryRow {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("binary inventory row name must not be empty");
        }
        if self.version.trim().is_empty() {
            bail!("binary {} version must not be empty", self.name);
        }
        if self.description.trim().is_empty() {
            bail!("binary {} description must not be empty", self.name);
        }
        if self.installed_path.trim().is_empty() {
            bail!("binary {} installed_path must not be empty", self.name);
        }
        if self.platform.trim().is_empty() {
            bail!("binary {} platform must not be empty", self.name);
        }
        if self.bytes == 0 {
            bail!("binary {} bytes must be non-zero", self.name);
        }
        if self.sbom_component_ref.trim().is_empty() {
            bail!("binary {} sbom_component_ref must not be empty", self.name);
        }
        self.digest
            .validate(&format!("binary {} {}", self.name, self.version))?;
        Ok(())
    }
}

pub fn executable_inventory_from_package_files(
    package: &PackageInventoryRow,
    files: &[PackagedExecutableFile],
    sbom_component_refs: &BTreeMap<String, String>,
) -> Result<Vec<BinaryInventoryRow>> {
    let mut rows = Vec::new();
    let mut installed_paths = std::collections::BTreeSet::new();
    for file in files {
        if file.name.trim().is_empty() {
            bail!("packaged executable name must not be empty");
        }
        if file.installed_path.trim().is_empty() {
            bail!("packaged executable {} installed_path must not be empty", file.name);
        }
        if !installed_paths.insert(file.installed_path.as_str()) {
            bail!("duplicate packaged executable installed_path {}", file.installed_path);
        }
        if file.bytes.is_empty() {
            bail!("packaged executable {} must not be empty", file.installed_path);
        }
        let sbom_component_ref = sbom_component_refs
            .get(&file.installed_path)
            .ok_or_else(|| {
                anyhow!(
                    "packaged executable {} missing SBOM component reference",
                    file.installed_path
                )
            })?
            .clone();
        let row = BinaryInventoryRow {
            name: file.name.clone(),
            version: package.version.clone(),
            description: file.description.clone(),
            installed_path: file.installed_path.clone(),
            platform: package.platform.clone(),
            architecture: package.architecture,
            bytes: file.bytes.len() as u64,
            digest: DigestSet {
                sha256: format!("{:x}", Sha256::digest(&file.bytes)),
                blake3: blake3::hash(&file.bytes).to_hex().to_string(),
            },
            status: package.status,
            sbom_component_ref,
        };
        row.validate()?;
        rows.push(row);
    }
    rows.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(rows)
}

pub fn verify_package_contents_match_binary_inventory(
    package: &PackageInventoryRow,
    files: &[PackagedExecutableFile],
    binaries: &[BinaryInventoryRow],
) -> Result<()> {
    let mut rows_by_path = BTreeMap::new();
    for row in binaries {
        if rows_by_path.insert(row.installed_path.as_str(), row).is_some() {
            bail!(
                "binary inventory has duplicate installed_path {} for package {}",
                row.installed_path,
                package.name
            );
        }
    }

    let mut seen_paths = std::collections::BTreeSet::new();
    for file in files {
        let row = rows_by_path.get(file.installed_path.as_str()).ok_or_else(|| {
            anyhow!(
                "package {} executable {} missing from binary inventory",
                package.name,
                file.installed_path
            )
        })?;
        if row.name != file.name {
            bail!(
                "binary inventory name mismatch for {}: expected {}, got {}",
                file.installed_path,
                file.name,
                row.name
            );
        }
        if row.version != package.version {
            bail!(
                "binary inventory version mismatch for {}: expected {}, got {}",
                file.installed_path,
                package.version,
                row.version
            );
        }
        if row.platform != package.platform {
            bail!(
                "binary inventory platform mismatch for {}: expected {}, got {}",
                file.installed_path,
                package.platform,
                row.platform
            );
        }
        if row.architecture != package.architecture {
            bail!("binary inventory architecture mismatch for {}", file.installed_path);
        }
        if row.bytes != file.bytes.len() as u64 {
            bail!("binary inventory byte count mismatch for {}", file.installed_path);
        }
        let sha256 = format!("{:x}", Sha256::digest(&file.bytes));
        if row.digest.sha256 != sha256 {
            bail!("binary inventory sha256 mismatch for {}", file.installed_path);
        }
        let blake3 = blake3::hash(&file.bytes).to_hex().to_string();
        if row.digest.blake3 != blake3 {
            bail!("binary inventory blake3 mismatch for {}", file.installed_path);
        }
        if row.sbom_component_ref.trim().is_empty() {
            bail!(
                "binary inventory missing SBOM component reference for {}",
                file.installed_path
            );
        }
        seen_paths.insert(file.installed_path.as_str());
    }

    for installed_path in rows_by_path.keys() {
        if !seen_paths.contains(installed_path) {
            bail!(
                "binary inventory lists {} for package {} but package contents do not contain it",
                installed_path,
                package.name
            );
        }
    }

    Ok(())
}

impl ReleaseManifest {
    pub fn validate_inventory_shape(&self) -> Result<()> {
        if self.version.trim().is_empty() {
            bail!("release manifest version must not be empty");
        }
        if self.packages.is_empty() {
            bail!("release manifest {} must list packages", self.version);
        }
        for package in &self.packages {
            package.validate()?;
        }
        Ok(())
    }
}

impl ReleaseLedger {
    pub fn derive(catalog: &ChannelsCatalog, manifests: &BTreeMap<String, BTreeMap<String, ReleaseManifest>>) -> Self {
        let mut entries = Vec::new();
        for (channel, record) in &catalog.channels {
            for manifest_record in &record.manifests {
                entries.push(ReleaseLedgerEntry {
                    channel: channel.clone(),
                    kind: ReleaseLedgerKind::Manifest,
                    name: manifest_record.url.clone(),
                    version: manifest_record.version.clone(),
                    status: manifest_record.status,
                    architecture: None,
                });
            }
            let Some(channel_manifests) = manifests.get(channel) else {
                continue;
            };
            for manifest in channel_manifests.values() {
                entries.extend(manifest.ledger_entries(channel));
            }
        }
        Self { entries }
    }
}

impl ReleaseManifest {
    fn ledger_entries(&self, channel: &str) -> Vec<ReleaseLedgerEntry> {
        let mut entries = Vec::new();
        for package in &self.packages {
            entries.push(ReleaseLedgerEntry {
                channel: channel.to_string(),
                kind: ReleaseLedgerKind::Package,
                name: package.name.clone(),
                version: package.version.clone(),
                status: package.status,
                architecture: Some(ReleaseLedgerArchitecture::Package(package.architecture)),
            });
        }
        for package in &self.packages {
            for binary in &package.binaries {
                entries.push(ReleaseLedgerEntry {
                    channel: channel.to_string(),
                    kind: ReleaseLedgerKind::Binary,
                    name: binary.name.clone(),
                    version: binary.version.clone(),
                    status: binary.status,
                    architecture: Some(ReleaseLedgerArchitecture::Package(binary.architecture)),
                });
            }
        }
        if let Some(runtime) = &self.runtime {
            entries.push(ReleaseLedgerEntry {
                channel: channel.to_string(),
                kind: ReleaseLedgerKind::Runtime,
                name: "runtime".to_string(),
                version: runtime.revision.clone(),
                status: runtime.status,
                architecture: None,
            });
            for architecture in &runtime.architectures {
                for image in &architecture.images {
                    entries.push(ReleaseLedgerEntry {
                        channel: channel.to_string(),
                        kind: ReleaseLedgerKind::RuntimeImage,
                        name: image.name.clone(),
                        version: runtime.revision.clone(),
                        status: image.status,
                        architecture: Some(ReleaseLedgerArchitecture::Machine(architecture.architecture)),
                    });
                }
            }
        }
        entries
    }
}

/// Refuse a runtime revision that cannot name an immutable publication path.
///
/// The revision is a path component of every published image URL and part of
/// the publication identity, so it is restricted to one URL-safe component.
pub fn validate_runtime_revision(revision: &str) -> Result<()> {
    if revision.is_empty()
        || revision.contains("..")
        || !revision
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("runtime revision must be one URL-path-safe component: {revision:?}");
    }
    Ok(())
}

impl RuntimeDocument {
    pub fn validate(&self) -> Result<()> {
        validate_runtime_revision(&self.revision)?;
        let minimum = parse_capsem_bound(self.min_capsem_version.as_deref(), "minimum")?;
        let maximum = parse_capsem_bound(self.max_capsem_version.as_deref(), "maximum")?;
        if let (Some(minimum), Some(maximum)) = (&minimum, &maximum) {
            if minimum > maximum {
                bail!("runtime minimum Capsem version {minimum} exceeds maximum {maximum}");
            }
        }
        if self.architectures.is_empty() {
            bail!("runtime {} must list architecture records", self.revision);
        }
        let mut seen = BTreeSet::new();
        for architecture in &self.architectures {
            if !seen.insert(architecture.architecture) {
                bail!(
                    "runtime {} repeats architecture {}",
                    self.revision,
                    architecture.architecture.as_str()
                );
            }
            architecture.validate(&self.revision)?;
        }
        Ok(())
    }
}

fn parse_capsem_bound(value: Option<&str>, label: &str) -> Result<Option<Version>> {
    value
        .map(|value| {
            Version::parse(value).with_context(|| format!("runtime {label} Capsem version is invalid: {value}"))
        })
        .transpose()
}

impl SoftwareInventoryRow {
    fn validate(&self, architecture: Architecture) -> Result<()> {
        let arch = architecture.as_str();
        if self.name.trim().is_empty() {
            bail!("runtime {arch} software name must not be empty");
        }
        let version = self.version.trim();
        if version.is_empty() {
            bail!("runtime {arch} software {} version must not be empty", self.name);
        }
        if matches!(
            version.to_ascii_lowercase().as_str(),
            "unversioned" | "unknown" | "latest"
        ) {
            bail!("runtime {arch} software {} version is {}", self.name, self.version);
        }
        if self.source.trim().is_empty() {
            bail!("runtime {arch} software {} source must not be empty", self.name);
        }
        if self.architecture != architecture {
            bail!("runtime {arch} software {} architecture mismatch", self.name);
        }
        validate_url_like(&self.evidence)
            .with_context(|| format!("runtime {arch} software {} evidence is invalid", self.name))?;
        self.digest
            .validate(&format!("runtime {arch} software {}", self.name))?;
        Ok(())
    }
}

impl RuntimeArchitecture {
    fn validate(&self, revision: &str) -> Result<()> {
        let arch = self.architecture.as_str();
        for (label, value) in [
            ("package_inventory_revision", &self.package_inventory_revision),
            ("image_revision", &self.image_revision),
        ] {
            if value != revision {
                bail!("runtime {revision} architecture {arch} {label} {value} is not the runtime revision");
            }
        }
        if self.software.is_empty() {
            bail!("runtime {revision} architecture {arch} must list software");
        }
        let software_inventory_digests = self
            .evidence
            .iter()
            .filter(|evidence| evidence.kind == "software_inventory")
            .map(|evidence| &evidence.digest)
            .collect::<Vec<_>>();
        for software in &self.software {
            software.validate(self.architecture)?;
            if software_inventory_digests.contains(&&software.digest) {
                bail!(
                    "runtime {arch} software {} digest reuses software_inventory evidence digest",
                    software.name
                );
            }
        }
        for required_kind in REQUIRED_RUNTIME_IMAGE_ARTIFACT_KINDS {
            if !self.images.iter().any(|image| image.kind == required_kind) {
                bail!(
                    "runtime {revision} architecture {arch} images missing {}",
                    required_kind.as_str()
                );
            }
        }
        for image in &self.images {
            image.validate(arch)?;
        }
        for evidence in &self.evidence {
            let kind = evidence.kind.as_str();
            if matches!(kind, "abom" | "obom") && !evidence_url_matches_architecture(&evidence.url, self.architecture) {
                bail!("runtime architecture {arch} evidence {kind} url must include /{arch}/");
            }
            evidence.validate(&format!("runtime {arch} image"))?;
        }
        Ok(())
    }
}

fn evidence_url_matches_architecture(url: &str, architecture: Architecture) -> bool {
    let arch = architecture.as_str();
    url.contains(&format!("/{arch}/")) || url.contains(&format!("/{arch}-"))
}

impl RuntimeImageArtifactKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Initrd => "initrd",
            Self::Rootfs => "rootfs",
        }
    }
}

impl RuntimeImageArtifactRef {
    fn validate(&self, arch: &str) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("runtime {arch} image artifact name must not be empty");
        }
        validate_url_like(&self.url)
            .with_context(|| format!("runtime {arch} image artifact {} url is invalid", self.name))?;
        if self.bytes == 0 {
            bail!("runtime {arch} image artifact {} bytes must be non-zero", self.name);
        }
        self.digest
            .validate(&format!("runtime {arch} image artifact {}", self.name))?;
        Ok(())
    }
}

impl ChannelsCatalog {
    pub fn validate(&self) -> Result<()> {
        if self.version == 0 {
            bail!("channels catalog version must be non-zero");
        }
        if self.generated_at.trim().is_empty() {
            bail!("channels catalog generated_at must not be empty");
        }
        if self.channels.is_empty() {
            bail!("channels catalog must list at least one channel");
        }
        for (channel, record) in &self.channels {
            validate_channel_id(channel)?;
            if record.label.trim().is_empty() {
                bail!("channel {channel} label must not be empty");
            }
            if record.manifests.is_empty() {
                bail!("channel {channel} must list at least one manifest");
            }
            let mut seen_versions = std::collections::BTreeSet::new();
            for manifest in &record.manifests {
                manifest.validate(channel)?;
                if !seen_versions.insert(manifest.version.as_str()) {
                    bail!(
                        "channel {channel} lists duplicate manifest version {}",
                        manifest.version
                    );
                }
            }
        }
        Ok(())
    }

    pub fn select_manifest(&self, channel: &str) -> Result<&ManifestRecord> {
        let channel_record = self
            .channels
            .get(channel)
            .ok_or_else(|| anyhow!("channel {channel} is not listed"))?;
        channel_record
            .manifests
            .iter()
            .filter(|manifest| manifest.status != Status::Revoked)
            .min_by_key(|manifest| manifest.status.selection_rank())
            .ok_or_else(|| anyhow!("channel {channel} has no selectable manifest"))
    }
}

impl Status {
    fn selection_rank(self) -> u8 {
        match self {
            Status::Current => 0,
            Status::Supported => 1,
            Status::Deprecated => 2,
            Status::Revoked => 255,
        }
    }
}

fn validate_channel_id(channel: &str) -> Result<()> {
    if channel.trim().is_empty() {
        bail!("channel id must not be empty");
    }
    if !channel
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
    {
        return Err(anyhow!(
            "channel id must contain only lowercase ASCII letters, digits, '-' or '_': {channel}"
        ));
    }
    Ok(())
}

fn validate_hex_digest(value: &str, expected_len: usize) -> Result<()> {
    if value.len() != expected_len || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("expected {expected_len} hex chars, got {value}");
    }
    Ok(())
}

fn validate_url_like(value: &str) -> Result<()> {
    if !(value.starts_with('/')
        || value.starts_with("https://")
        || value.starts_with("http://")
        || value.starts_with("file://"))
    {
        bail!("expected release-site relative, file, or http(s) URL, got {value}");
    }
    Ok(())
}

fn default_status_current() -> Status {
    Status::Current
}

#[cfg(test)]
mod tests;
