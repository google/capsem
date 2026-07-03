use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

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
pub enum Architecture {
    Arm64,
    X86_64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileImageArtifactKind {
    Kernel,
    Initrd,
    Rootfs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileConfigKind {
    Profile,
    Mcp,
    Enforcement,
    Detection,
    Apt,
    Python,
    Npm,
    Build,
    Tips,
    RootManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseLedgerKind {
    Manifest,
    Package,
    Binary,
    Profile,
    ProfileImage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub kind: String,
    pub url: String,
    pub digest: DigestSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInventoryRow {
    pub name: String,
    pub version: String,
    pub kind: PackageKind,
    pub platform: String,
    pub architecture: Architecture,
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
    pub architecture: Architecture,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub version: String,
    #[serde(default = "default_status_current")]
    pub status: Status,
    #[serde(default)]
    pub packages: Vec<PackageInventoryRow>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDocument {
    pub version: String,
    pub id: String,
    pub name: String,
    pub revision: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_capsem_version: Option<String>,
    pub architectures: Vec<ProfileArchitectureImages>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoftwareInventoryRow {
    pub name: String,
    pub version: String,
    pub source: String,
    pub architecture: String,
    pub evidence: String,
    pub digest: DigestSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfigRef {
    pub kind: ProfileConfigKind,
    pub path: String,
    pub url: String,
    pub bytes: u64,
    pub digest: DigestSet,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileArchitectureImages {
    pub architecture: Architecture,
    #[serde(default)]
    pub software: Vec<SoftwareInventoryRow>,
    #[serde(default)]
    pub config: Vec<ProfileConfigRef>,
    #[serde(rename = "images")]
    pub artifacts: Vec<ProfileImageArtifactRef>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileVersionHistory {
    pub channel: String,
    pub profile_id: String,
    pub versions: Vec<ProfileDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProfileImageArtifactKey {
    pub architecture: Architecture,
    pub kind: ProfileImageArtifactKind,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileImageDiff {
    pub added: Vec<ProfileImageArtifactKey>,
    pub retained: Vec<ProfileImageArtifactKey>,
    pub removed: Vec<ProfileImageArtifactKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileImageArtifactRef {
    pub kind: ProfileImageArtifactKind,
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
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<Architecture>,
}

impl DigestSet {
    fn validate(&self, context: &str) -> Result<()> {
        validate_hex_digest(&self.sha256, 64)
            .with_context(|| format!("{context} sha256 digest is invalid"))?;
        validate_hex_digest(&self.blake3, 64)
            .with_context(|| format!("{context} blake3 digest is invalid"))?;
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
        if self.version.contains('/') || self.version.contains('\\') || self.version.contains("..")
        {
            bail!(
                "channel {channel} manifest version contains a path separator: {}",
                self.version
            );
        }
        if self.url.trim().is_empty() {
            bail!(
                "channel {channel} manifest {} url must not be empty",
                self.version
            );
        }
        if !(self.url.starts_with('/')
            || self.url.starts_with("https://")
            || self.url.starts_with("http://"))
        {
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
        validate_url_like(&self.url)
            .with_context(|| format!("{context} evidence url is invalid"))?;
        self.digest
            .validate(&format!("{context} evidence {}", self.kind))?;
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
        validate_url_like(&self.url).with_context(|| {
            format!(
                "package {} {} download url is invalid",
                self.name, self.version
            )
        })?;
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
                bail!(
                    "package {} binary {} architecture mismatch",
                    self.name,
                    binary.name
                );
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
            bail!(
                "packaged executable {} installed_path must not be empty",
                file.name
            );
        }
        if !installed_paths.insert(file.installed_path.as_str()) {
            bail!(
                "duplicate packaged executable installed_path {}",
                file.installed_path
            );
        }
        if file.bytes.is_empty() {
            bail!(
                "packaged executable {} must not be empty",
                file.installed_path
            );
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
        if rows_by_path
            .insert(row.installed_path.as_str(), row)
            .is_some()
        {
            bail!(
                "binary inventory has duplicate installed_path {} for package {}",
                row.installed_path,
                package.name
            );
        }
    }

    let mut seen_paths = std::collections::BTreeSet::new();
    for file in files {
        let row = rows_by_path
            .get(file.installed_path.as_str())
            .ok_or_else(|| {
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
            bail!(
                "binary inventory architecture mismatch for {}",
                file.installed_path
            );
        }
        if row.bytes != file.bytes.len() as u64 {
            bail!(
                "binary inventory byte count mismatch for {}",
                file.installed_path
            );
        }
        let sha256 = format!("{:x}", Sha256::digest(&file.bytes));
        if row.digest.sha256 != sha256 {
            bail!(
                "binary inventory sha256 mismatch for {}",
                file.installed_path
            );
        }
        let blake3 = blake3::hash(&file.bytes).to_hex().to_string();
        if row.digest.blake3 != blake3 {
            bail!(
                "binary inventory blake3 mismatch for {}",
                file.installed_path
            );
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
    pub fn derive(
        catalog: &ChannelsCatalog,
        manifests: &BTreeMap<String, BTreeMap<String, ReleaseManifest>>,
    ) -> Self {
        let mut entries = Vec::new();
        for (channel, record) in &catalog.channels {
            for manifest_record in &record.manifests {
                entries.push(ReleaseLedgerEntry {
                    channel: channel.clone(),
                    kind: ReleaseLedgerKind::Manifest,
                    name: manifest_record.url.clone(),
                    version: manifest_record.version.clone(),
                    status: manifest_record.status,
                    profile: None,
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
                profile: None,
                architecture: Some(package.architecture),
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
                    profile: None,
                    architecture: Some(binary.architecture),
                });
            }
        }
        for (profile_id, profile) in &self.profiles {
            entries.push(ReleaseLedgerEntry {
                channel: channel.to_string(),
                kind: ReleaseLedgerKind::Profile,
                name: profile_id.clone(),
                version: profile.revision.clone(),
                status: profile.status,
                profile: Some(profile_id.clone()),
                architecture: None,
            });
            for architecture in &profile.architectures {
                for artifact in &architecture.artifacts {
                    entries.push(ReleaseLedgerEntry {
                        channel: channel.to_string(),
                        kind: ReleaseLedgerKind::ProfileImage,
                        name: artifact.name.clone(),
                        version: profile.revision.clone(),
                        status: artifact.status,
                        profile: Some(profile_id.clone()),
                        architecture: Some(architecture.architecture),
                    });
                }
            }
        }
        entries
    }
}

impl ProfileDocument {
    pub fn validate_profile_ownership(&self) -> Result<()> {
        if self.version.trim().is_empty() {
            bail!("profile {} version must not be empty", self.id);
        }
        if self.id.trim().is_empty() {
            bail!("profile id must not be empty");
        }
        if self.name.trim().is_empty() {
            bail!("profile {} name must not be empty", self.id);
        }
        if self.revision.trim().is_empty() {
            bail!("profile {} revision must not be empty", self.id);
        }
        if self.architectures.is_empty() {
            bail!("profile {} must list architecture records", self.id);
        }
        for architecture in &self.architectures {
            architecture.validate(&self.id)?;
        }
        Ok(())
    }
}

impl ProfileVersionHistory {
    pub fn new(channel: impl Into<String>, first: ProfileDocument) -> Result<Self> {
        first.validate_profile_ownership()?;
        let channel = channel.into();
        validate_channel_id(&channel)?;
        Ok(Self {
            channel,
            profile_id: first.id.clone(),
            versions: vec![first],
        })
    }

    pub fn append_version(&mut self, next: ProfileDocument) -> Result<()> {
        next.validate_profile_ownership()?;
        if next.id != self.profile_id {
            bail!(
                "profile history {} cannot append profile {}",
                self.profile_id,
                next.id
            );
        }
        if self
            .versions
            .iter()
            .any(|profile| profile.revision == next.revision)
        {
            bail!(
                "profile history {} already contains revision {}",
                self.profile_id,
                next.revision
            );
        }
        self.versions.push(next);
        Ok(())
    }
}

pub fn diff_profile_image_artifacts(
    previous: &ProfileDocument,
    next: &ProfileDocument,
) -> Result<ProfileImageDiff> {
    if previous.id != next.id {
        bail!(
            "cannot diff profile images for different profiles: {} vs {}",
            previous.id,
            next.id
        );
    }
    previous.validate_profile_ownership()?;
    next.validate_profile_ownership()?;
    let previous_keys = profile_image_artifact_keys(previous);
    let next_keys = profile_image_artifact_keys(next);
    Ok(ProfileImageDiff {
        added: next_keys.difference(&previous_keys).cloned().collect(),
        retained: next_keys.intersection(&previous_keys).cloned().collect(),
        removed: previous_keys.difference(&next_keys).cloned().collect(),
    })
}

fn profile_image_artifact_keys(
    profile: &ProfileDocument,
) -> std::collections::BTreeSet<ProfileImageArtifactKey> {
    let mut keys = std::collections::BTreeSet::new();
    for architecture in &profile.architectures {
        for artifact in &architecture.artifacts {
            keys.insert(ProfileImageArtifactKey {
                architecture: architecture.architecture,
                kind: artifact.kind,
                name: artifact.name.clone(),
            });
        }
    }
    keys
}

impl SoftwareInventoryRow {
    fn validate(&self, profile: &str) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("profile {profile} software name must not be empty");
        }
        if self.version.trim().is_empty() {
            bail!(
                "profile {profile} software {} version must not be empty",
                self.name
            );
        }
        let version = self.version.trim();
        if matches!(
            version.to_ascii_lowercase().as_str(),
            "unversioned" | "unknown" | "latest"
        ) {
            bail!(
                "profile {profile} software {} version is {}",
                self.name,
                self.version
            );
        }
        if self.source.trim().is_empty() {
            bail!(
                "profile {profile} software {} source must not be empty",
                self.name
            );
        }
        if self.architecture.trim().is_empty() {
            bail!(
                "profile {profile} software {} architecture must not be empty",
                self.name
            );
        }
        validate_url_like(&self.evidence).with_context(|| {
            format!(
                "profile {profile} software {} evidence is invalid",
                self.name
            )
        })?;
        self.digest
            .validate(&format!("profile {profile} software {}", self.name))?;
        Ok(())
    }
}

impl ProfileConfigRef {
    fn validate(&self, profile: &str) -> Result<()> {
        if self.path.trim().is_empty() {
            bail!(
                "profile {profile} config {} path must not be empty",
                self.kind.as_str()
            );
        }
        validate_url_like(&self.url).with_context(|| {
            format!(
                "profile {profile} config {} url is invalid",
                self.kind.as_str()
            )
        })?;
        if self.bytes == 0 {
            bail!(
                "profile {profile} config {} bytes must be non-zero",
                self.kind.as_str()
            );
        }
        self.digest
            .validate(&format!("profile {profile} config {}", self.kind.as_str()))?;
        Ok(())
    }
}

impl ProfileConfigKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Mcp => "mcp",
            Self::Enforcement => "enforcement",
            Self::Detection => "detection",
            Self::Apt => "apt",
            Self::Python => "python",
            Self::Npm => "npm",
            Self::Build => "build",
            Self::Tips => "tips",
            Self::RootManifest => "root_manifest",
        }
    }
}

impl ProfileArchitectureImages {
    fn validate(&self, profile: &str) -> Result<()> {
        if self.software.is_empty() {
            bail!(
                "profile {profile} architecture {:?} must list software",
                self.architecture
            );
        }
        if self.config.is_empty() {
            bail!(
                "profile {profile} architecture {:?} must list config",
                self.architecture
            );
        }
        let software_inventory_digests = self
            .evidence
            .iter()
            .filter(|evidence| evidence.kind == "software_inventory")
            .map(|evidence| &evidence.digest)
            .collect::<Vec<_>>();
        for software in &self.software {
            software.validate(profile)?;
            if software_inventory_digests
                .iter()
                .any(|digest| **digest == software.digest)
            {
                bail!(
                    "profile {profile} architecture {:?} software {} digest reuses software_inventory evidence digest",
                    self.architecture,
                    software.name
                );
            }
        }
        for config in &self.config {
            config.validate(profile)?;
        }
        if self.artifacts.is_empty() {
            bail!("profile {profile} image set must list artifacts");
        }
        for artifact in &self.artifacts {
            artifact.validate(profile)?;
        }
        for evidence in &self.evidence {
            evidence.validate(&format!("profile {profile} image evidence"))?;
        }
        Ok(())
    }
}

impl ProfileImageArtifactRef {
    fn validate(&self, profile: &str) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("profile {profile} image artifact name must not be empty");
        }
        validate_url_like(&self.url).with_context(|| {
            format!(
                "profile {profile} image artifact {} url is invalid",
                self.name
            )
        })?;
        if self.bytes == 0 {
            bail!(
                "profile {profile} image artifact {} bytes must be non-zero",
                self.name
            );
        }
        self.digest
            .validate(&format!("profile {profile} image artifact {}", self.name))?;
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
mod tests {
    use super::*;

    fn digest_json() -> serde_json::Value {
        serde_json::json!({
            "sha256": "a".repeat(64),
            "blake3": "b".repeat(64)
        })
    }

    fn digest_set() -> DigestSet {
        digest_set_with('a', 'b')
    }

    fn digest_set_with(sha256: char, blake3: char) -> DigestSet {
        DigestSet {
            sha256: sha256.to_string().repeat(64),
            blake3: blake3.to_string().repeat(64),
        }
    }

    fn software_row() -> SoftwareInventoryRow {
        SoftwareInventoryRow {
            name: "python".to_string(),
            version: "3.12.11".to_string(),
            source: "apt".to_string(),
            architecture: "all".to_string(),
            evidence: "/profiles/releases/2026.07.02.1/co-work/apt-packages.txt".to_string(),
            digest: digest_set(),
        }
    }

    #[test]
    fn release_graph_enums_reject_unknown_status_values() {
        let error = serde_json::from_value::<Status>(serde_json::json!("removed"))
            .expect_err("removed is absence from a newer graph, not a status");

        assert!(
            error.to_string().contains("unknown variant")
                || error.to_string().contains("expected one of"),
            "{error}"
        );
    }

    #[test]
    fn release_graph_enums_accept_only_canonical_status_values() {
        for (raw, expected) in [
            ("current", Status::Current),
            ("supported", Status::Supported),
            ("deprecated", Status::Deprecated),
            ("revoked", Status::Revoked),
        ] {
            let parsed: Status = serde_json::from_value(serde_json::json!(raw)).expect(raw);
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn release_graph_manifest_records_use_version_not_schema_version() {
        let valid = serde_json::json!({
            "version": "1.4.0",
            "status": "current",
            "url": "/manifests/stable/1.4.0/manifest.json",
            "digest": digest_json(),
            "min_capsem_version": "1.4.0"
        });
        serde_json::from_value::<ManifestRecord>(valid)
            .expect("version is the manifest record key");

        let invalid = serde_json::json!({
            "schema_version": 2,
            "status": "current",
            "url": "/manifests/stable/1.4.0/manifest.json",
            "digest": digest_json()
        });
        let error = serde_json::from_value::<ManifestRecord>(invalid)
            .expect_err("manifest records must not use schema_version");

        assert!(error.to_string().contains("schema_version"), "{error}");
    }

    #[test]
    fn release_graph_channels_catalog_lists_manifest_records() {
        let catalog = serde_json::json!({
            "version": 1,
            "generated_at": "2030-01-01T00:00:00Z",
            "channels": {
                "stable": {
                    "label": "Stable",
                    "manifests": [
                        {
                            "version": "1.4.0",
                            "status": "current",
                            "url": "/manifests/stable/1.4.0/manifest.json",
                            "digest": digest_json()
                        },
                        {
                            "version": "1.3.0",
                            "status": "supported",
                            "url": "/manifests/stable/1.3.0/manifest.json",
                            "digest": digest_json()
                        }
                    ]
                },
                "nightly": {
                    "label": "Nightly",
                    "manifests": [
                        {
                            "version": "1.5.0-nightly.20300101",
                            "status": "current",
                            "url": "/manifests/nightly/1.5.0-nightly.20300101/manifest.json",
                            "digest": digest_json()
                        }
                    ]
                }
            }
        });

        let parsed: ChannelsCatalog =
            serde_json::from_value(catalog).expect("channels catalog parses");
        assert_eq!(parsed.channels["stable"].manifests.len(), 2);
        assert_eq!(
            parsed.channels["nightly"].manifests[0].status,
            Status::Current
        );
        parsed.validate().expect("catalog validates");
    }

    #[test]
    fn release_graph_channels_catalog_rejects_duplicate_manifest_versions() {
        let catalog = serde_json::json!({
            "version": 1,
            "generated_at": "2030-01-01T00:00:00Z",
            "channels": {
                "stable": {
                    "label": "Stable",
                    "manifests": [
                        {
                            "version": "1.4.0",
                            "status": "current",
                            "url": "/manifests/stable/1.4.0/manifest.json",
                            "digest": digest_json()
                        },
                        {
                            "version": "1.4.0",
                            "status": "supported",
                            "url": "/manifests/stable/1.4.0-copy/manifest.json",
                            "digest": digest_json()
                        }
                    ]
                }
            }
        });
        let parsed: ChannelsCatalog =
            serde_json::from_value(catalog).expect("JSON shape parses before validation");
        let error = parsed
            .validate()
            .expect_err("duplicate manifest versions are ambiguous");
        assert!(
            error.to_string().contains("duplicate manifest version"),
            "{error}"
        );
    }

    #[test]
    fn release_graph_channels_catalog_rejects_bad_digest_shape() {
        let catalog = serde_json::json!({
            "version": 1,
            "generated_at": "2030-01-01T00:00:00Z",
            "channels": {
                "nightly": {
                    "label": "Nightly",
                    "manifests": [
                        {
                            "version": "1.5.0-nightly.20300101",
                            "status": "current",
                            "url": "/manifests/nightly/1.5.0-nightly.20300101/manifest.json",
                            "digest": {
                                "sha256": "a".repeat(40),
                                "blake3": "b".repeat(64)
                            }
                        }
                    ]
                }
            }
        });
        let parsed: ChannelsCatalog =
            serde_json::from_value(catalog).expect("JSON shape parses before validation");
        let error = parsed.validate().expect_err("bad sha256 rejected");
        assert!(error.to_string().contains("sha256"), "{error}");
    }

    #[test]
    fn release_graph_digest_verifier_rejects_tampered_profile_ref() {
        let bytes = br#"{"id":"co-work","version":"1.2.0"}"#;
        let digest = DigestSet {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            blake3: blake3::hash(bytes).to_hex().to_string(),
        };

        digest
            .verify_bytes(bytes, "profile co-work")
            .expect("original bytes verify");
        let error = digest
            .verify_bytes(br#"{"id":"co-work","version":"1.2.1"}"#, "profile co-work")
            .expect_err("tampered profile ref is rejected");
        assert!(error.to_string().contains("sha256 mismatch"), "{error}");
    }

    #[test]
    fn release_graph_revoked_manifest_is_listed_but_not_selectable() {
        let catalog: ChannelsCatalog = serde_json::from_value(serde_json::json!({
            "version": 1,
            "generated_at": "2030-01-01T00:00:00Z",
            "channels": {
                "stable": {
                    "label": "Stable",
                    "manifests": [
                        {
                            "version": "1.4.0-bad",
                            "status": "revoked",
                            "url": "/manifests/stable/1.4.0-bad/manifest.json",
                            "digest": digest_json()
                        },
                        {
                            "version": "1.3.0",
                            "status": "supported",
                            "url": "/manifests/stable/1.3.0/manifest.json",
                            "digest": digest_json()
                        }
                    ]
                }
            }
        }))
        .expect("catalog shape");

        catalog
            .validate()
            .expect("revoked manifests remain auditable");
        let selected = catalog
            .select_manifest("stable")
            .expect("supported fallback selected");
        assert_eq!(selected.version, "1.3.0");
        assert_eq!(
            catalog.channels["stable"].manifests[0].status,
            Status::Revoked
        );
    }

    #[test]
    fn release_graph_current_manifest_is_preferred_over_supported_and_deprecated() {
        let catalog: ChannelsCatalog = serde_json::from_value(serde_json::json!({
            "version": 1,
            "generated_at": "2030-01-01T00:00:00Z",
            "channels": {
                "nightly": {
                    "label": "Nightly",
                    "manifests": [
                        {
                            "version": "1.5.0-nightly.old",
                            "status": "deprecated",
                            "url": "/manifests/nightly/1.5.0-nightly.old/manifest.json",
                            "digest": digest_json()
                        },
                        {
                            "version": "1.5.0-nightly.supported",
                            "status": "supported",
                            "url": "/manifests/nightly/1.5.0-nightly.supported/manifest.json",
                            "digest": digest_json()
                        },
                        {
                            "version": "1.5.0-nightly.current",
                            "status": "current",
                            "url": "/manifests/nightly/1.5.0-nightly.current/manifest.json",
                            "digest": digest_json()
                        }
                    ]
                }
            }
        }))
        .expect("catalog shape");

        let selected = catalog
            .select_manifest("nightly")
            .expect("manifest selected");
        assert_eq!(selected.version, "1.5.0-nightly.current");
    }

    #[test]
    fn package_inventory_rows_are_separate_from_binary_rows() {
        let binary = BinaryInventoryRow {
            name: "capsem".to_string(),
            version: "1.4.0".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/local/bin/capsem".to_string(),
            platform: "macos".to_string(),
            architecture: Architecture::Arm64,
            bytes: 7,
            digest: digest_set(),
            status: Status::Current,
            sbom_component_ref: "SPDXRef-File-capsem".to_string(),
        };
        let manifest = ReleaseManifest {
            version: "1.4.0".to_string(),
            status: Status::Current,
            packages: vec![PackageInventoryRow {
                name: "Capsem-1.4.0.pkg".to_string(),
                version: "1.4.0".to_string(),
                kind: PackageKind::MacosPkg,
                platform: "macos".to_string(),
                architecture: Architecture::Arm64,
                url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
                bytes: 42,
                digest: digest_set(),
                status: Status::Current,
                binaries: vec![binary],
                evidence: vec![EvidenceRef {
                    kind: "sbom".to_string(),
                    url: "/packages/stable/1.4.0/capsem-1-4-0-pkg-sbom.spdx.json".to_string(),
                    digest: digest_set(),
                }],
            }],
            profiles: BTreeMap::new(),
        };

        manifest
            .validate_inventory_shape()
            .expect("package and binary inventory is valid");
        assert_ne!(
            manifest.packages[0].name,
            manifest.packages[0].binaries[0].name
        );
        assert_eq!(
            manifest.packages[0].binaries[0].installed_path,
            "/usr/local/bin/capsem"
        );
    }

    #[test]
    fn package_inventory_requires_package_sbom() {
        let manifest = ReleaseManifest {
            version: "1.4.0".to_string(),
            status: Status::Current,
            packages: vec![PackageInventoryRow {
                name: "Capsem-1.4.0.pkg".to_string(),
                version: "1.4.0".to_string(),
                kind: PackageKind::MacosPkg,
                platform: "macos".to_string(),
                architecture: Architecture::Arm64,
                url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
                bytes: 42,
                digest: digest_set(),
                status: Status::Current,
                binaries: vec![BinaryInventoryRow {
                    name: "capsem".to_string(),
                    version: "1.4.0".to_string(),
                    description: "Capsem executable fixture".to_string(),
                    installed_path: "/usr/local/bin/capsem".to_string(),
                    platform: "macos".to_string(),
                    architecture: Architecture::Arm64,
                    bytes: 7,
                    digest: digest_set(),
                    status: Status::Current,
                    sbom_component_ref: "SPDXRef-File-capsem".to_string(),
                }],
                evidence: Vec::new(),
            }],
            profiles: BTreeMap::new(),
        };

        let error = manifest
            .validate_inventory_shape()
            .expect_err("missing package SBOM evidence is rejected");
        assert!(
            format!("{error:#}").contains("must include package SBOM evidence"),
            "{error:#}"
        );
    }

    #[test]
    fn package_inventory_requires_sha256_and_blake3() {
        let manifest = ReleaseManifest {
            version: "1.4.0".to_string(),
            status: Status::Current,
            packages: vec![PackageInventoryRow {
                name: "capsem_1.4.0_arm64.deb".to_string(),
                version: "1.4.0".to_string(),
                kind: PackageKind::DebianPackage,
                platform: "linux".to_string(),
                architecture: Architecture::Arm64,
                url: "/packages/stable/1.4.0/capsem_1.4.0_arm64.deb".to_string(),
                bytes: 42,
                digest: DigestSet {
                    sha256: "a".repeat(64),
                    blake3: "not-a-blake3-digest".to_string(),
                },
                status: Status::Current,
                binaries: vec![BinaryInventoryRow {
                    name: "capsem".to_string(),
                    version: "1.4.0".to_string(),
                    description: "Capsem executable fixture".to_string(),
                    installed_path: "/usr/bin/capsem".to_string(),
                    platform: "linux".to_string(),
                    architecture: Architecture::Arm64,
                    bytes: 7,
                    digest: digest_set(),
                    status: Status::Current,
                    sbom_component_ref: "SPDXRef-File-capsem".to_string(),
                }],
                evidence: Vec::new(),
            }],
            profiles: BTreeMap::new(),
        };

        let error = manifest
            .validate_inventory_shape()
            .expect_err("bad package digest is rejected");
        assert!(format!("{error:#}").contains("blake3"), "{error:#}");
    }

    #[test]
    fn executable_inventory_records_every_packaged_binary_with_hashes_and_sbom_refs() {
        let package = PackageInventoryRow {
            name: "Capsem-1.4.0.pkg".to_string(),
            version: "1.4.0".to_string(),
            kind: PackageKind::MacosPkg,
            platform: "macos".to_string(),
            architecture: Architecture::Arm64,
            url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
            bytes: 42,
            digest: digest_set(),
            status: Status::Current,
            binaries: Vec::new(),
            evidence: Vec::new(),
        };
        let files = vec![
            PackagedExecutableFile {
                name: "capsem-service".to_string(),
                description: "Capsem executable fixture".to_string(),
                installed_path: "/usr/local/share/capsem/bin/capsem-service".to_string(),
                bytes: b"service-bin".to_vec(),
            },
            PackagedExecutableFile {
                name: "capsem".to_string(),
                description: "Capsem executable fixture".to_string(),
                installed_path: "/usr/local/bin/capsem".to_string(),
                bytes: b"capsem-bin".to_vec(),
            },
        ];
        let sbom_refs = BTreeMap::from([
            (
                "/usr/local/bin/capsem".to_string(),
                "SPDXRef-File-capsem".to_string(),
            ),
            (
                "/usr/local/share/capsem/bin/capsem-service".to_string(),
                "SPDXRef-File-capsem-service".to_string(),
            ),
        ]);

        let rows =
            executable_inventory_from_package_files(&package, &files, &sbom_refs).expect("rows");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "capsem");
        assert_eq!(rows[0].installed_path, "/usr/local/bin/capsem");
        assert_eq!(
            rows[0].digest.sha256,
            format!("{:x}", Sha256::digest(b"capsem-bin"))
        );
        assert_eq!(
            rows[0].digest.blake3,
            blake3::hash(b"capsem-bin").to_hex().to_string()
        );
        assert_eq!(rows[0].sbom_component_ref, "SPDXRef-File-capsem");
        assert_eq!(rows[1].sbom_component_ref, "SPDXRef-File-capsem-service");
    }

    #[test]
    fn executable_inventory_rejects_missing_sbom_component_ref() {
        let package = PackageInventoryRow {
            name: "capsem_1.4.0_arm64.deb".to_string(),
            version: "1.4.0".to_string(),
            kind: PackageKind::DebianPackage,
            platform: "linux".to_string(),
            architecture: Architecture::Arm64,
            url: "/packages/stable/1.4.0/capsem_1.4.0_arm64.deb".to_string(),
            bytes: 42,
            digest: digest_set(),
            status: Status::Current,
            binaries: Vec::new(),
            evidence: Vec::new(),
        };
        let files = vec![PackagedExecutableFile {
            name: "capsem".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/bin/capsem".to_string(),
            bytes: b"capsem-bin".to_vec(),
        }];

        let error = executable_inventory_from_package_files(&package, &files, &BTreeMap::new())
            .expect_err("missing SBOM component ref rejected");

        assert!(
            format!("{error:#}").contains("missing SBOM component reference"),
            "{error:#}"
        );
    }

    #[test]
    fn executable_inventory_matches_macos_and_deb_package_contents() {
        let macos_package = PackageInventoryRow {
            name: "Capsem-1.4.0.pkg".to_string(),
            version: "1.4.0".to_string(),
            kind: PackageKind::MacosPkg,
            platform: "macos".to_string(),
            architecture: Architecture::Arm64,
            url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
            bytes: 99,
            digest: digest_set(),
            status: Status::Current,
            binaries: Vec::new(),
            evidence: Vec::new(),
        };
        let macos_files = vec![
            PackagedExecutableFile {
                name: "capsem".to_string(),
                description: "Capsem executable fixture".to_string(),
                installed_path: "/usr/local/share/capsem/bin/capsem".to_string(),
                bytes: b"macos-capsem".to_vec(),
            },
            PackagedExecutableFile {
                name: "capsem-service".to_string(),
                description: "Capsem executable fixture".to_string(),
                installed_path: "/usr/local/share/capsem/bin/capsem-service".to_string(),
                bytes: b"macos-service".to_vec(),
            },
        ];
        let macos_sbom_refs = BTreeMap::from([
            (
                "/usr/local/share/capsem/bin/capsem".to_string(),
                "SPDXRef-File-macos-capsem".to_string(),
            ),
            (
                "/usr/local/share/capsem/bin/capsem-service".to_string(),
                "SPDXRef-File-macos-capsem-service".to_string(),
            ),
        ]);
        let macos_rows =
            executable_inventory_from_package_files(&macos_package, &macos_files, &macos_sbom_refs)
                .expect("macOS package rows");
        verify_package_contents_match_binary_inventory(&macos_package, &macos_files, &macos_rows)
            .expect("macOS package contents match manifest inventory");

        let deb_package = PackageInventoryRow {
            name: "Capsem_1.4.0_arm64.deb".to_string(),
            version: "1.4.0".to_string(),
            kind: PackageKind::DebianPackage,
            platform: "linux".to_string(),
            architecture: Architecture::Arm64,
            url: "/packages/stable/1.4.0/Capsem_1.4.0_arm64.deb".to_string(),
            bytes: 101,
            digest: digest_set(),
            status: Status::Current,
            binaries: Vec::new(),
            evidence: Vec::new(),
        };
        let deb_files = vec![
            PackagedExecutableFile {
                name: "capsem".to_string(),
                description: "Capsem executable fixture".to_string(),
                installed_path: "/usr/bin/capsem".to_string(),
                bytes: b"deb-capsem".to_vec(),
            },
            PackagedExecutableFile {
                name: "capsem-service".to_string(),
                description: "Capsem executable fixture".to_string(),
                installed_path: "/usr/bin/capsem-service".to_string(),
                bytes: b"deb-service".to_vec(),
            },
        ];
        let deb_sbom_refs = BTreeMap::from([
            (
                "/usr/bin/capsem".to_string(),
                "SPDXRef-File-deb-capsem".to_string(),
            ),
            (
                "/usr/bin/capsem-service".to_string(),
                "SPDXRef-File-deb-capsem-service".to_string(),
            ),
        ]);
        let deb_rows =
            executable_inventory_from_package_files(&deb_package, &deb_files, &deb_sbom_refs)
                .expect("deb package rows");
        verify_package_contents_match_binary_inventory(&deb_package, &deb_files, &deb_rows)
            .expect("deb package contents match manifest inventory");
    }

    #[test]
    fn executable_inventory_rejects_package_content_hash_drift() {
        let package = PackageInventoryRow {
            name: "Capsem_1.4.0_arm64.deb".to_string(),
            version: "1.4.0".to_string(),
            kind: PackageKind::DebianPackage,
            platform: "linux".to_string(),
            architecture: Architecture::Arm64,
            url: "/packages/stable/1.4.0/Capsem_1.4.0_arm64.deb".to_string(),
            bytes: 101,
            digest: digest_set(),
            status: Status::Current,
            binaries: Vec::new(),
            evidence: Vec::new(),
        };
        let files = vec![PackagedExecutableFile {
            name: "capsem".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/bin/capsem".to_string(),
            bytes: b"deb-capsem".to_vec(),
        }];
        let sbom_refs = BTreeMap::from([(
            "/usr/bin/capsem".to_string(),
            "SPDXRef-File-deb-capsem".to_string(),
        )]);
        let mut rows =
            executable_inventory_from_package_files(&package, &files, &sbom_refs).expect("rows");
        rows[0].digest.sha256 = "0".repeat(64);

        let error = verify_package_contents_match_binary_inventory(&package, &files, &rows)
            .expect_err("tampered package content hash must be rejected");

        assert!(
            format!("{error:#}").contains("sha256 mismatch"),
            "{error:#}"
        );
    }

    fn profile_with_image_artifacts(
        revision: &str,
        artifacts: Vec<ProfileImageArtifactRef>,
    ) -> ProfileDocument {
        ProfileDocument {
            version: revision.to_string(),
            id: "co-work".to_string(),
            name: "Co-work".to_string(),
            revision: revision.to_string(),
            status: Status::Current,
            min_capsem_version: Some("1.4.0".to_string()),
            architectures: vec![profile_architecture(revision, artifacts)],
        }
    }

    fn profile_architecture(
        revision: &str,
        artifacts: Vec<ProfileImageArtifactRef>,
    ) -> ProfileArchitectureImages {
        ProfileArchitectureImages {
            architecture: Architecture::Arm64,
            software: vec![software_row()],
            config: vec![ProfileConfigRef {
                kind: ProfileConfigKind::Mcp,
                path: "profiles/co-work/mcp.json".to_string(),
                url: format!("/profiles/releases/{revision}/co-work/arm64/mcp.json"),
                bytes: 12,
                digest: digest_set(),
                status: Status::Current,
            }],
            artifacts,
            evidence: vec![
                EvidenceRef {
                    kind: "abom".to_string(),
                    url: format!("/profiles/releases/{revision}/co-work/arm64/abom.cdx.json"),
                    digest: digest_set(),
                },
                EvidenceRef {
                    kind: "obom".to_string(),
                    url: format!("/profiles/releases/{revision}/co-work/arm64/obom.cdx.json"),
                    digest: digest_set(),
                },
                EvidenceRef {
                    kind: "software_inventory".to_string(),
                    url: format!(
                        "/profiles/releases/{revision}/co-work/arm64/software-inventory.json"
                    ),
                    digest: digest_set_with('c', 'd'),
                },
            ],
        }
    }

    fn profile_image_artifact(
        kind: ProfileImageArtifactKind,
        name: &str,
        revision: &str,
    ) -> ProfileImageArtifactRef {
        ProfileImageArtifactRef {
            kind,
            name: name.to_string(),
            url: format!("/profiles/releases/{revision}/co-work/arm64/{name}"),
            bytes: 42,
            digest: digest_set(),
            status: Status::Current,
        }
    }

    #[test]
    fn profile_image_versions_append_without_deprecating_previous() {
        let first = profile_with_image_artifacts(
            "2026.07.02.1",
            vec![profile_image_artifact(
                ProfileImageArtifactKind::Rootfs,
                "rootfs.erofs",
                "2026.07.02.1",
            )],
        );
        let second = profile_with_image_artifacts(
            "2026.07.02.2",
            vec![profile_image_artifact(
                ProfileImageArtifactKind::Rootfs,
                "rootfs.erofs",
                "2026.07.02.2",
            )],
        );
        let mut history =
            ProfileVersionHistory::new("nightly", first).expect("first profile version");

        history
            .append_version(second)
            .expect("new profile image version appends");

        assert_eq!(history.versions.len(), 2);
        assert_eq!(history.versions[0].revision, "2026.07.02.1");
        assert_eq!(
            history.versions[0].architectures[0].artifacts[0].status,
            Status::Current
        );
        assert_eq!(history.versions[1].revision, "2026.07.02.2");
    }

    #[test]
    fn profile_image_versions_removed_image_is_absent_not_status_removed() {
        let previous = profile_with_image_artifacts(
            "2026.07.02.1",
            vec![
                profile_image_artifact(
                    ProfileImageArtifactKind::Initrd,
                    "initrd.img",
                    "2026.07.02.1",
                ),
                profile_image_artifact(
                    ProfileImageArtifactKind::Rootfs,
                    "rootfs.erofs",
                    "2026.07.02.1",
                ),
            ],
        );
        let next = profile_with_image_artifacts(
            "2026.07.02.2",
            vec![profile_image_artifact(
                ProfileImageArtifactKind::Rootfs,
                "rootfs.erofs",
                "2026.07.02.2",
            )],
        );

        let diff = diff_profile_image_artifacts(&previous, &next).expect("profile diff");

        assert_eq!(
            diff.removed,
            vec![ProfileImageArtifactKey {
                architecture: Architecture::Arm64,
                kind: ProfileImageArtifactKind::Initrd,
                name: "initrd.img".to_string(),
            }]
        );
        assert_eq!(next.architectures[0].artifacts.len(), 1);
        assert!(next
            .architectures
            .iter()
            .flat_map(|images| images.artifacts.iter())
            .all(|artifact| artifact.status != Status::Deprecated));

        let invalid_removed_status = serde_json::json!({
            "kind": "initrd",
            "name": "initrd.img",
            "url": "/profiles/releases/2026.07.02.2/co-work/arm64/initrd.img",
            "bytes": 42,
            "digest": digest_json(),
            "status": "removed"
        });
        serde_json::from_value::<ProfileImageArtifactRef>(invalid_removed_status)
            .expect_err("removed is represented by absence, not by a status enum");
    }

    #[test]
    fn profile_config_kind_rejects_unknown_values() {
        let invalid_kind = serde_json::json!({
            "kind": "misc",
            "path": "profiles/co-work/misc.json",
            "url": "/profiles/releases/2026.07.02.1/co-work/arm64/misc.json",
            "bytes": 42,
            "digest": digest_json(),
            "status": "current"
        });

        serde_json::from_value::<ProfileConfigRef>(invalid_kind)
            .expect_err("profile config kind must be a release graph enum");
    }

    #[test]
    fn profile_json_ownership_has_min_capsem_not_current_binary() {
        let profile = ProfileDocument {
            version: "2026.07.02.1".to_string(),
            id: "co-work".to_string(),
            name: "Co-work".to_string(),
            revision: "2026.07.02.1".to_string(),
            status: Status::Current,
            min_capsem_version: Some("1.4.0".to_string()),
            architectures: vec![ProfileArchitectureImages {
                architecture: Architecture::Arm64,
                software: vec![software_row()],
                config: vec![ProfileConfigRef {
                    kind: ProfileConfigKind::Mcp,
                    path: "profiles/co-work/mcp.json".to_string(),
                    url: "/profiles/releases/2026.07.02.1/co-work/arm64/mcp.json".to_string(),
                    bytes: 12,
                    digest: digest_set(),
                    status: Status::Current,
                }],
                artifacts: vec![
                    ProfileImageArtifactRef {
                        kind: ProfileImageArtifactKind::Kernel,
                        name: "vmlinuz".to_string(),
                        url: "/profiles/releases/2026.07.02.1/co-work/arm64/vmlinuz".to_string(),
                        bytes: 42,
                        digest: digest_set(),
                        status: Status::Current,
                    },
                    ProfileImageArtifactRef {
                        kind: ProfileImageArtifactKind::Initrd,
                        name: "initrd.img".to_string(),
                        url: "/profiles/releases/2026.07.02.1/co-work/arm64/initrd.img".to_string(),
                        bytes: 42,
                        digest: digest_set(),
                        status: Status::Current,
                    },
                    ProfileImageArtifactRef {
                        kind: ProfileImageArtifactKind::Rootfs,
                        name: "rootfs.erofs".to_string(),
                        url: "/profiles/releases/2026.07.02.1/co-work/arm64/rootfs.erofs"
                            .to_string(),
                        bytes: 42,
                        digest: digest_set(),
                        status: Status::Current,
                    },
                ],
                evidence: vec![
                    EvidenceRef {
                        kind: "abom".to_string(),
                        url: "/profiles/releases/2026.07.02.1/co-work/arm64/abom.cdx.json"
                            .to_string(),
                        digest: digest_set(),
                    },
                    EvidenceRef {
                        kind: "obom".to_string(),
                        url: "/profiles/releases/2026.07.02.1/co-work/arm64/obom.cdx.json"
                            .to_string(),
                        digest: digest_set(),
                    },
                    EvidenceRef {
                        kind: "software_inventory".to_string(),
                        url:
                            "/profiles/releases/2026.07.02.1/co-work/arm64/software-inventory.json"
                                .to_string(),
                        digest: digest_set_with('c', 'd'),
                    },
                ],
            }],
        };

        profile
            .validate_profile_ownership()
            .expect("profile-owned graph validates");
        assert_eq!(profile.min_capsem_version.as_deref(), Some("1.4.0"));
        assert_eq!(profile.architectures[0].evidence.len(), 3);
    }

    #[test]
    fn profile_json_ownership_rejects_unversioned_software_rows() {
        let mut profile = profile_with_image_artifacts(
            "2026.07.02.1",
            vec![profile_image_artifact(
                ProfileImageArtifactKind::Rootfs,
                "rootfs.erofs",
                "2026.07.02.1",
            )],
        );
        profile.architectures[0].software[0].version = "unversioned".to_string();

        let error = profile
            .validate_profile_ownership()
            .expect_err("profile software rows must use real versions");

        assert!(error.to_string().contains("unversioned"), "{error}");
    }

    #[test]
    fn profile_json_ownership_rejects_reused_software_inventory_digest() {
        let mut profile = profile_with_image_artifacts(
            "2026.07.02.1",
            vec![profile_image_artifact(
                ProfileImageArtifactKind::Rootfs,
                "rootfs.erofs",
                "2026.07.02.1",
            )],
        );
        let inventory_digest = profile.architectures[0]
            .evidence
            .iter()
            .find(|evidence| evidence.kind == "software_inventory")
            .expect("software inventory evidence")
            .digest
            .clone();
        profile.architectures[0].software[0].digest = inventory_digest;

        let error = profile
            .validate_profile_ownership()
            .expect_err("software rows must not reuse inventory file digests");

        assert!(
            error
                .to_string()
                .contains("reuses software_inventory evidence digest"),
            "{error}"
        );
    }

    #[test]
    fn profile_json_ownership_rejects_current_binary_and_assets() {
        let invalid = serde_json::json!({
            "version": "2026.07.02.1",
            "id": "co-work",
            "name": "Co-work",
            "revision": "2026.07.02.1",
            "status": "current",
            "min_capsem_version": "1.4.0",
            "current_binary": "1.4.0",
            "current_assets": "2026.0627.8"
        });

        let error = serde_json::from_value::<ProfileDocument>(invalid)
            .expect_err("profile JSON must not contain channel-owned current binary/assets");
        assert!(
            error.to_string().contains("current_binary")
                || error.to_string().contains("current_assets"),
            "{error}"
        );
    }

    #[test]
    fn release_ledger_is_derived_from_channels_and_manifests() {
        let catalog: ChannelsCatalog = serde_json::from_value(serde_json::json!({
            "version": 1,
            "generated_at": "2030-01-01T00:00:00Z",
            "channels": {
                "stable": {
                    "label": "Stable",
                    "manifests": [
                        {
                            "version": "1.4.0",
                            "status": "current",
                            "url": "/manifests/stable/1.4.0/manifest.json",
                            "digest": digest_json()
                        }
                    ]
                },
                "nightly": {
                    "label": "Nightly",
                    "manifests": [
                        {
                            "version": "1.5.0-nightly.20300101",
                            "status": "current",
                            "url": "/manifests/nightly/1.5.0-nightly.20300101/manifest.json",
                            "digest": digest_json()
                        }
                    ]
                }
            }
        }))
        .expect("catalog shape");

        let mut profiles = BTreeMap::new();
        profiles.insert(
            "co-work".to_string(),
            ProfileDocument {
                version: "2026.07.02.1".to_string(),
                id: "co-work".to_string(),
                name: "Co-work".to_string(),
                revision: "2026.07.02.1".to_string(),
                status: Status::Current,
                min_capsem_version: Some("1.4.0".to_string()),
                architectures: vec![ProfileArchitectureImages {
                    architecture: Architecture::Arm64,
                    software: vec![software_row()],
                    config: vec![ProfileConfigRef {
                        kind: ProfileConfigKind::Mcp,
                        path: "profiles/co-work/mcp.json".to_string(),
                        url: "/profiles/releases/2026.07.02.1/co-work/arm64/mcp.json".to_string(),
                        bytes: 12,
                        digest: digest_set(),
                        status: Status::Current,
                    }],
                    artifacts: vec![ProfileImageArtifactRef {
                        kind: ProfileImageArtifactKind::Rootfs,
                        name: "rootfs.erofs".to_string(),
                        url: "/profiles/releases/2026.07.02.1/co-work/arm64/rootfs.erofs"
                            .to_string(),
                        bytes: 42,
                        digest: digest_set(),
                        status: Status::Current,
                    }],
                    evidence: vec![
                        EvidenceRef {
                            kind: "abom".to_string(),
                            url: "/profiles/releases/2026.07.02.1/co-work/arm64/abom.cdx.json"
                                .to_string(),
                            digest: digest_set(),
                        },
                        EvidenceRef {
                            kind: "obom".to_string(),
                            url: "/profiles/releases/2026.07.02.1/co-work/arm64/obom.cdx.json"
                                .to_string(),
                            digest: digest_set(),
                        },
                        EvidenceRef {
                            kind: "software_inventory".to_string(),
                            url: "/profiles/releases/2026.07.02.1/co-work/arm64/software-inventory.json"
                                .to_string(),
                            digest: digest_set_with('c', 'd'),
                        },
                    ],
                }],
            },
        );

        let mut manifests = BTreeMap::new();
        manifests.insert(
            "stable".to_string(),
            BTreeMap::from([(
                "1.4.0".to_string(),
                ReleaseManifest {
                    version: "1.4.0".to_string(),
                    status: Status::Current,
                    packages: vec![PackageInventoryRow {
                        name: "Capsem-1.4.0.pkg".to_string(),
                        version: "1.4.0".to_string(),
                        kind: PackageKind::MacosPkg,
                        platform: "macos".to_string(),
                        architecture: Architecture::Arm64,
                        url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
                        bytes: 42,
                        digest: digest_set(),
                        status: Status::Current,
                        binaries: vec![BinaryInventoryRow {
                            name: "capsem".to_string(),
                            version: "1.4.0".to_string(),
                            description: "Capsem executable fixture".to_string(),
                            installed_path: "/usr/local/bin/capsem".to_string(),
                            platform: "macos".to_string(),
                            architecture: Architecture::Arm64,
                            bytes: 7,
                            digest: digest_set(),
                            status: Status::Current,
                            sbom_component_ref: "SPDXRef-File-capsem".to_string(),
                        }],
                        evidence: Vec::new(),
                    }],
                    profiles,
                },
            )]),
        );

        let ledger = ReleaseLedger::derive(&catalog, &manifests);
        assert!(ledger.entries.iter().any(|entry| {
            entry.channel == "stable"
                && entry.kind == ReleaseLedgerKind::Package
                && entry.name == "Capsem-1.4.0.pkg"
        }));
        assert!(ledger.entries.iter().any(|entry| {
            entry.channel == "stable"
                && entry.kind == ReleaseLedgerKind::Binary
                && entry.name == "capsem"
        }));
        assert!(ledger.entries.iter().any(|entry| {
            entry.channel == "stable"
                && entry.kind == ReleaseLedgerKind::Profile
                && entry.profile.as_deref() == Some("co-work")
        }));
        assert!(ledger.entries.iter().any(|entry| {
            entry.channel == "stable"
                && entry.kind == ReleaseLedgerKind::ProfileImage
                && entry.profile.as_deref() == Some("co-work")
                && entry.architecture == Some(Architecture::Arm64)
        }));
        assert!(ledger.entries.iter().any(|entry| {
            entry.channel == "nightly" && entry.kind == ReleaseLedgerKind::Manifest
        }));
    }
}
