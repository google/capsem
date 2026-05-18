//! Signed profile catalog manifest types.
//!
//! S07a makes this manifest the profile catalog. This module is intentionally
//! about typed parsing and validation only; download, signature verification,
//! and VM pinning build on top of these types in later slices.

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const PROFILE_MANIFEST_FORMAT: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileRevisionStatus {
    Active,
    Deprecated,
    Revoked,
}

impl ProfileRevisionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Revoked => "revoked",
        }
    }

    pub fn can_be_current(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileManifest {
    pub format: u32,
    pub profiles: BTreeMap<String, ManifestProfile>,
}

impl ProfileManifest {
    pub fn from_json(content: &str) -> Result<Self> {
        let manifest: Self =
            serde_json::from_str(content).context("parse profile manifest JSON")?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != PROFILE_MANIFEST_FORMAT {
            bail!(
                "unsupported profile manifest format {}; expected {}",
                self.format,
                PROFILE_MANIFEST_FORMAT
            );
        }
        if self.profiles.is_empty() {
            bail!("profile manifest must contain at least one profile");
        }
        for (profile_id, profile) in &self.profiles {
            validate_profile_id(profile_id)
                .with_context(|| format!("profiles.{profile_id}: invalid profile id"))?;
            profile
                .validate(profile_id)
                .with_context(|| format!("profiles.{profile_id}"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestProfile {
    pub current_revision: String,
    pub revisions: BTreeMap<String, ManifestProfileRevision>,
}

impl ManifestProfile {
    fn validate(&self, profile_id: &str) -> Result<()> {
        validate_revision("current_revision", &self.current_revision)?;
        if self.revisions.is_empty() {
            bail!("revisions must not be empty");
        }
        let current = self.revisions.get(&self.current_revision).ok_or_else(|| {
            anyhow::anyhow!(
                "current_revision '{}' does not exist in revisions",
                self.current_revision
            )
        })?;
        if !current.status.can_be_current() {
            bail!(
                "current_revision '{}' for profile '{}' must be active, got {}",
                self.current_revision,
                profile_id,
                current.status.as_str()
            );
        }
        for (revision, record) in &self.revisions {
            validate_revision("revision", revision)
                .with_context(|| format!("revisions.{revision}: invalid revision"))?;
            record
                .validate()
                .with_context(|| format!("revisions.{revision}"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestProfileRevision {
    pub status: ProfileRevisionStatus,
    pub min_binary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_binary: Option<String>,
    pub profile_url: String,
    pub profile_hash: String,
    pub profile_signature_url: String,
}

impl ManifestProfileRevision {
    fn validate(&self) -> Result<()> {
        validate_non_empty("min_binary", &self.min_binary)?;
        if let Some(max_binary) = &self.max_binary {
            validate_non_empty("max_binary", max_binary)?;
        }
        validate_location("profile_url", &self.profile_url)?;
        validate_hash("profile_hash", &self.profile_hash)?;
        validate_location("profile_signature_url", &self.profile_signature_url)?;
        Ok(())
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} must not be empty");
    }
    Ok(())
}

fn validate_profile_id(value: &str) -> Result<()> {
    if value.len() < 3 || value.len() > 64 {
        bail!("profile id must be 3-64 characters");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        bail!("profile id may only contain lowercase letters, digits, and '-'");
    }
    Ok(())
}

fn validate_revision(field: &str, value: &str) -> Result<()> {
    let mut parts = value.split('.');
    let Some(year) = parts.next() else {
        bail!("{field} must use YYYY.MMDD.patch");
    };
    let Some(month_day) = parts.next() else {
        bail!("{field} must use YYYY.MMDD.patch");
    };
    let Some(patch) = parts.next() else {
        bail!("{field} must use YYYY.MMDD.patch");
    };
    if parts.next().is_some()
        || year.len() != 4
        || month_day.len() != 4
        || patch.is_empty()
        || !year.chars().all(|ch| ch.is_ascii_digit())
        || !month_day.chars().all(|ch| ch.is_ascii_digit())
        || !patch.chars().all(|ch| ch.is_ascii_digit())
    {
        bail!("{field} must use YYYY.MMDD.patch");
    }
    Ok(())
}

fn validate_hash(field: &str, value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("blake3:") else {
        bail!("{field} must use blake3:<64 lowercase hex>");
    };
    if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("{field} must use blake3:<64 lowercase hex>");
    }
    if hex.chars().any(|ch| ch.is_ascii_uppercase()) {
        bail!("{field} must use lowercase hex");
    }
    Ok(())
}

fn validate_location(field: &str, value: &str) -> Result<()> {
    validate_non_empty(field, value)?;
    if value.contains("..") || value.contains('\\') {
        bail!("{field} contains path traversal");
    }
    if value.starts_with("https://") || value.starts_with("file://") {
        return Ok(());
    }
    bail!("{field} must use https:// or file://");
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn manifest_json(status: &str) -> String {
        format!(
            r#"{{
              "format": 1,
              "profiles": {{
                "everyday-work": {{
                  "current_revision": "2026.0520.1",
                  "revisions": {{
                    "2026.0520.1": {{
                      "status": "{status}",
                      "min_binary": "1.0.0",
                      "max_binary": null,
                      "profile_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.1/profile.toml",
                      "profile_hash": "{HASH}",
                      "profile_signature_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.1/profile.toml.minisig"
                    }}
                  }}
                }}
              }}
            }}"#
        )
    }

    #[test]
    fn profile_manifest_accepts_active_current_revision() {
        let manifest = ProfileManifest::from_json(&manifest_json("active")).unwrap();
        let revision = &manifest.profiles["everyday-work"].revisions["2026.0520.1"];
        assert_eq!(revision.status, ProfileRevisionStatus::Active);
    }

    #[test]
    fn profile_manifest_accepts_deprecated_non_current_revision() {
        let json = format!(
            r#"{{
              "format": 1,
              "profiles": {{
                "everyday-work": {{
                  "current_revision": "2026.0520.2",
                  "revisions": {{
                    "2026.0520.1": {{
                      "status": "deprecated",
                      "min_binary": "1.0.0",
                      "profile_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.1/profile.toml",
                      "profile_hash": "{HASH}",
                      "profile_signature_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.1/profile.toml.minisig"
                    }},
                    "2026.0520.2": {{
                      "status": "active",
                      "min_binary": "1.0.0",
                      "profile_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.2/profile.toml",
                      "profile_hash": "{HASH}",
                      "profile_signature_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.2/profile.toml.minisig"
                    }}
                  }}
                }}
              }}
            }}"#
        );
        let manifest = ProfileManifest::from_json(&json).unwrap();
        let revision = &manifest.profiles["everyday-work"].revisions["2026.0520.1"];
        assert_eq!(revision.status, ProfileRevisionStatus::Deprecated);
    }

    #[test]
    fn profile_manifest_accepts_revoked_non_current_revision() {
        let json = format!(
            r#"{{
              "format": 1,
              "profiles": {{
                "everyday-work": {{
                  "current_revision": "2026.0520.1",
                  "revisions": {{
                    "2026.0520.0": {{
                      "status": "revoked",
                      "min_binary": "1.0.0",
                      "profile_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.0/profile.toml",
                      "profile_hash": "{HASH}",
                      "profile_signature_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.0/profile.toml.minisig"
                    }},
                    "2026.0520.1": {{
                      "status": "active",
                      "min_binary": "1.0.0",
                      "profile_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.1/profile.toml",
                      "profile_hash": "{HASH}",
                      "profile_signature_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.1/profile.toml.minisig"
                    }}
                  }}
                }}
              }}
            }}"#
        );
        let manifest = ProfileManifest::from_json(&json).unwrap();
        let revision = &manifest.profiles["everyday-work"].revisions["2026.0520.0"];
        assert_eq!(revision.status, ProfileRevisionStatus::Revoked);
    }

    #[test]
    fn profile_manifest_rejects_removed_status() {
        let error = ProfileManifest::from_json(&manifest_json("removed")).unwrap_err();
        assert!(format!("{error:#}").contains("unknown variant"));
    }

    #[test]
    fn profile_manifest_rejects_revoked_current_revision() {
        let error = ProfileManifest::from_json(&manifest_json("revoked")).unwrap_err();
        assert!(format!("{error:#}").contains("must be active"));
    }

    #[test]
    fn profile_manifest_rejects_deprecated_current_revision() {
        let error = ProfileManifest::from_json(&manifest_json("deprecated")).unwrap_err();
        assert!(format!("{error:#}").contains("must be active"));
    }

    #[test]
    fn profile_manifest_rejects_missing_current_revision() {
        let json = manifest_json("active").replace("2026.0520.1", "2026.0520.2");
        let json = json.replacen(
            r#""current_revision": "2026.0520.2""#,
            r#""current_revision": "2026.0520.1""#,
            1,
        );
        let error = ProfileManifest::from_json(&json).unwrap_err();
        assert!(format!("{error:#}").contains("does not exist"));
    }

    #[test]
    fn profile_manifest_rejects_bad_profile_hash() {
        let error = ProfileManifest::from_json(&manifest_json("active").replace(HASH, "aaaaaaaa"))
            .unwrap_err();
        assert!(format!("{error:#}").contains("profile_hash"));
    }

    #[test]
    fn profile_manifest_rejects_old_asset_manifest_format() {
        let error = ProfileManifest::from_json(
            &manifest_json("active").replace("\"format\": 1", "\"format\": 2"),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("unsupported profile manifest format"));
    }
}
