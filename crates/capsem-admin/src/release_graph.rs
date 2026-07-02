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
    pub hmac: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    Arm64,
    X86_64,
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
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryInventoryRow {
    pub name: String,
    pub version: String,
    pub package: String,
    pub install_path: String,
    pub platform: String,
    pub architecture: Architecture,
    pub bytes: u64,
    pub digest: DigestSet,
    pub status: Status,
    pub sbom_component_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub version: String,
    #[serde(default)]
    pub packages: Vec<PackageInventoryRow>,
    #[serde(default)]
    pub binaries: Vec<BinaryInventoryRow>,
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

impl DigestSet {
    fn validate(&self, context: &str) -> Result<()> {
        validate_hex_digest(&self.sha256, 64)
            .with_context(|| format!("{context} sha256 digest is invalid"))?;
        validate_hex_digest(&self.blake3, 64)
            .with_context(|| format!("{context} blake3 digest is invalid"))?;
        if self.hmac.trim().is_empty() {
            bail!("{context} hmac must not be empty");
        }
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
        for evidence in &self.evidence {
            evidence.validate(&format!("package {} {}", self.name, self.version))?;
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
        if self.package.trim().is_empty() {
            bail!("binary {} package must not be empty", self.name);
        }
        if self.install_path.trim().is_empty() {
            bail!("binary {} install_path must not be empty", self.name);
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

impl ReleaseManifest {
    pub fn validate_inventory_shape(&self) -> Result<()> {
        if self.version.trim().is_empty() {
            bail!("release manifest version must not be empty");
        }
        if self.packages.is_empty() {
            bail!("release manifest {} must list packages", self.version);
        }
        let packages: std::collections::BTreeSet<&str> = self
            .packages
            .iter()
            .map(|package| package.name.as_str())
            .collect();
        for package in &self.packages {
            package.validate()?;
        }
        for binary in &self.binaries {
            binary.validate()?;
            if !packages.contains(binary.package.as_str()) {
                bail!(
                    "binary {} references unknown package {}",
                    binary.name,
                    binary.package
                );
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_json() -> serde_json::Value {
        serde_json::json!({
            "sha256": "a".repeat(64),
            "blake3": "b".repeat(64),
            "hmac": "release-test-hmac"
        })
    }

    fn digest_set() -> DigestSet {
        DigestSet {
            sha256: "a".repeat(64),
            blake3: "b".repeat(64),
            hmac: "release-test-hmac".to_string(),
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
                                "blake3": "b".repeat(64),
                                "hmac": "release-test-hmac"
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
            hmac: "release-test-hmac".to_string(),
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
        let manifest = ReleaseManifest {
            version: "1.4.0".to_string(),
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
                evidence: vec![EvidenceRef {
                    kind: "notarization".to_string(),
                    url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg.notary.json".to_string(),
                    digest: digest_set(),
                }],
            }],
            binaries: vec![BinaryInventoryRow {
                name: "capsem".to_string(),
                version: "1.4.0".to_string(),
                package: "Capsem-1.4.0.pkg".to_string(),
                install_path: "/usr/local/bin/capsem".to_string(),
                platform: "macos".to_string(),
                architecture: Architecture::Arm64,
                bytes: 7,
                digest: digest_set(),
                status: Status::Current,
                sbom_component_ref: "SPDXRef-File-capsem".to_string(),
            }],
        };

        manifest
            .validate_inventory_shape()
            .expect("package and binary inventory is valid");
        assert_ne!(manifest.packages[0].name, manifest.binaries[0].name);
        assert_eq!(manifest.binaries[0].package, manifest.packages[0].name);
    }

    #[test]
    fn package_inventory_requires_sha256_blake3_and_hmac() {
        let manifest = ReleaseManifest {
            version: "1.4.0".to_string(),
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
                    hmac: "release-test-hmac".to_string(),
                },
                status: Status::Current,
                evidence: Vec::new(),
            }],
            binaries: Vec::new(),
        };

        let error = manifest
            .validate_inventory_shape()
            .expect_err("bad package digest is rejected");
        assert!(format!("{error:#}").contains("blake3"), "{error:#}");
    }
}
