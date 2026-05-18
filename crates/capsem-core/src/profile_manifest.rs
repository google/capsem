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

    pub fn allows_install_or_update(self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn allows_new_vm(self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn allows_existing_vm(self) -> bool {
        matches!(self, Self::Active | Self::Deprecated)
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

    pub fn current_revision(&self, profile_id: &str) -> Result<ResolvedProfileRevision<'_>> {
        let (profile_id, profile) = self.profile_entry(profile_id)?;
        let (revision, record) = profile
            .revisions
            .get_key_value(&profile.current_revision)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "current revision '{}' for profile '{}' not found",
                    profile.current_revision,
                    profile_id
                )
            })?;
        Ok(ResolvedProfileRevision {
            profile_id,
            revision,
            record,
        })
    }

    pub fn revision(
        &self,
        profile_id: &str,
        revision: &str,
    ) -> Result<ResolvedProfileRevision<'_>> {
        let (profile_id, profile) = self.profile_entry(profile_id)?;
        let (revision, record) = profile.revisions.get_key_value(revision).ok_or_else(|| {
            anyhow::anyhow!("revision '{revision}' for profile '{profile_id}' not found")
        })?;
        Ok(ResolvedProfileRevision {
            profile_id,
            revision,
            record,
        })
    }

    fn profile_entry(&self, profile_id: &str) -> Result<(&str, &ManifestProfile)> {
        self.profiles
            .get_key_value(profile_id)
            .map(|(profile_id, profile)| (profile_id.as_str(), profile))
            .ok_or_else(|| anyhow::anyhow!("profile '{profile_id}' not found"))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedProfileRevision<'a> {
    pub profile_id: &'a str,
    pub revision: &'a str,
    pub record: &'a ManifestProfileRevision,
}

#[derive(Debug, Clone)]
pub struct VerifiedProfilePayload {
    pub profile_id: String,
    pub revision: String,
    pub payload_hash: String,
    pub payload_json: String,
    pub value: serde_json::Value,
}

pub fn verify_installable_profile_payload(
    revision: ResolvedProfileRevision<'_>,
    payload_json: &str,
) -> Result<VerifiedProfilePayload> {
    if !revision.record.status.allows_install_or_update() {
        bail!(
            "profile '{}' revision '{}' has status '{}' and cannot be installed or updated",
            revision.profile_id,
            revision.revision,
            revision.record.status.as_str()
        );
    }

    let payload_hash = format!("blake3:{}", blake3::hash(payload_json.as_bytes()).to_hex());
    if payload_hash != revision.record.profile_hash {
        bail!(
            "profile payload hash mismatch for '{}@{}' (expected {}, got {})",
            revision.profile_id,
            revision.revision,
            revision.record.profile_hash,
            payload_hash
        );
    }

    let value = crate::profile_payload_schema::validate_profile_payload_v2_json(payload_json)
        .map_err(|error| anyhow::anyhow!("profile payload schema validation failed: {error}"))?;
    let payload_profile_id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("profile payload id is missing"))?;
    if payload_profile_id != revision.profile_id {
        bail!(
            "profile payload id '{}' does not match manifest profile '{}'",
            payload_profile_id,
            revision.profile_id
        );
    }
    let payload_revision = value
        .get("revision")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("profile payload revision is missing"))?;
    if payload_revision != revision.revision {
        bail!(
            "profile payload revision '{}' does not match manifest revision '{}'",
            payload_revision,
            revision.revision
        );
    }

    Ok(VerifiedProfilePayload {
        profile_id: payload_profile_id.to_string(),
        revision: payload_revision.to_string(),
        payload_hash,
        payload_json: payload_json.to_string(),
        value,
    })
}

pub fn verify_profile_payload_signature(
    pubkey_file: &str,
    payload_bytes: &[u8],
    sig_file: &str,
) -> Result<()> {
    crate::asset_manager::verify_manifest_signature(pubkey_file, payload_bytes, sig_file)
        .context("profile payload signature verification failed")
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
    const VALID_PROFILE_PAYLOAD: &str =
        include_str!("../../../schemas/fixtures/profile-v2-valid.json");

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

    fn payload_hash(payload: &str) -> String {
        format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex())
    }

    fn manifest_json_with_revision(
        target_revision: &str,
        status: &str,
        profile_hash: &str,
    ) -> String {
        format!(
            r#"{{
              "format": 1,
              "profiles": {{
                "everyday-work": {{
                  "current_revision": "2026.0520.2",
                  "revisions": {{
                    "{target_revision}": {{
                      "status": "{status}",
                      "min_binary": "1.0.0",
                      "profile_url": "https://assets.capsem.dev/profiles/everyday-work/{target_revision}/profile.json",
                      "profile_hash": "{profile_hash}",
                      "profile_signature_url": "https://assets.capsem.dev/profiles/everyday-work/{target_revision}/profile.json.minisig"
                    }},
                    "2026.0520.2": {{
                      "status": "active",
                      "min_binary": "1.0.0",
                      "profile_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.2/profile.json",
                      "profile_hash": "{HASH}",
                      "profile_signature_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.2/profile.json.minisig"
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
    fn profile_revision_status_lifecycle_gates_are_explicit() {
        assert!(ProfileRevisionStatus::Active.can_be_current());
        assert!(ProfileRevisionStatus::Active.allows_install_or_update());
        assert!(ProfileRevisionStatus::Active.allows_new_vm());
        assert!(ProfileRevisionStatus::Active.allows_existing_vm());

        assert!(!ProfileRevisionStatus::Deprecated.can_be_current());
        assert!(!ProfileRevisionStatus::Deprecated.allows_install_or_update());
        assert!(!ProfileRevisionStatus::Deprecated.allows_new_vm());
        assert!(ProfileRevisionStatus::Deprecated.allows_existing_vm());

        assert!(!ProfileRevisionStatus::Revoked.can_be_current());
        assert!(!ProfileRevisionStatus::Revoked.allows_install_or_update());
        assert!(!ProfileRevisionStatus::Revoked.allows_new_vm());
        assert!(!ProfileRevisionStatus::Revoked.allows_existing_vm());
    }

    #[test]
    fn profile_manifest_resolves_current_and_specific_revision_records() {
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

        let current = manifest.current_revision("everyday-work").unwrap();
        assert_eq!(current.profile_id, "everyday-work");
        assert_eq!(current.revision, "2026.0520.2");
        assert_eq!(current.record.status, ProfileRevisionStatus::Active);
        assert!(current.record.status.allows_install_or_update());

        let deprecated = manifest.revision("everyday-work", "2026.0520.1").unwrap();
        assert_eq!(deprecated.profile_id, "everyday-work");
        assert_eq!(deprecated.revision, "2026.0520.1");
        assert_eq!(deprecated.record.status, ProfileRevisionStatus::Deprecated);
        assert!(deprecated.record.status.allows_existing_vm());
        assert!(!deprecated.record.status.allows_new_vm());
    }

    #[test]
    fn profile_manifest_resolution_reports_missing_profile_or_revision() {
        let manifest = ProfileManifest::from_json(&manifest_json("active")).unwrap();

        let missing_profile = manifest.current_revision("ghost").unwrap_err();
        assert!(format!("{missing_profile:#}").contains("profile 'ghost' not found"));

        let missing_revision = manifest
            .revision("everyday-work", "2026.0520.0")
            .unwrap_err();
        assert!(format!("{missing_revision:#}").contains("revision '2026.0520.0'"));
    }

    #[test]
    fn installable_profile_payload_verifies_manifest_hash_and_identity() {
        let profile_hash = payload_hash(VALID_PROFILE_PAYLOAD);
        let manifest = ProfileManifest::from_json(&manifest_json_with_revision(
            "2026.0520.1",
            "active",
            &profile_hash,
        ))
        .unwrap();
        let revision = manifest.revision("everyday-work", "2026.0520.1").unwrap();

        let verified = verify_installable_profile_payload(revision, VALID_PROFILE_PAYLOAD).unwrap();

        assert_eq!(verified.profile_id, "everyday-work");
        assert_eq!(verified.revision, "2026.0520.1");
        assert_eq!(verified.payload_hash, profile_hash);
        assert_eq!(verified.value["schema"], "capsem.profile.v2");
    }

    #[test]
    fn installable_profile_payload_rejects_non_active_status() {
        let profile_hash = payload_hash(VALID_PROFILE_PAYLOAD);
        let manifest = ProfileManifest::from_json(&manifest_json_with_revision(
            "2026.0520.1",
            "deprecated",
            &profile_hash,
        ))
        .unwrap();
        let revision = manifest.revision("everyday-work", "2026.0520.1").unwrap();

        let error =
            verify_installable_profile_payload(revision, VALID_PROFILE_PAYLOAD).unwrap_err();

        assert!(format!("{error:#}").contains("cannot be installed or updated"));
    }

    #[test]
    fn installable_profile_payload_rejects_hash_mismatch() {
        let manifest =
            ProfileManifest::from_json(&manifest_json_with_revision("2026.0520.1", "active", HASH))
                .unwrap();
        let revision = manifest.revision("everyday-work", "2026.0520.1").unwrap();

        let error =
            verify_installable_profile_payload(revision, VALID_PROFILE_PAYLOAD).unwrap_err();

        assert!(format!("{error:#}").contains("profile payload hash mismatch"));
    }

    #[test]
    fn installable_profile_payload_rejects_id_or_revision_mismatch() {
        let payload = VALID_PROFILE_PAYLOAD.replace(
            r#""revision": "2026.0520.1""#,
            r#""revision": "2026.0520.0""#,
        );
        let profile_hash = payload_hash(&payload);
        let manifest = ProfileManifest::from_json(&manifest_json_with_revision(
            "2026.0520.1",
            "active",
            &profile_hash,
        ))
        .unwrap();
        let revision = manifest.revision("everyday-work", "2026.0520.1").unwrap();

        let error = verify_installable_profile_payload(revision, &payload).unwrap_err();

        assert!(format!("{error:#}").contains("payload revision"));
    }

    const TEST_PUBKEY: &str = "untrusted comment: minisign public key D2FF2FA8B3C45D80\nRWSAXcSzqC//0ussmV+rXA7RVjSb7oBJxZA/Ao9jSOz3yVIv8vcHBOLS\n";
    const TEST_SIGNED_BYTES: &[u8] = b"{\"hello\":\"world\",\"format\":2}";
    const TEST_SIGNATURE: &str = "untrusted comment: capsem test fixture\nRUSAXcSzqC//0gYG4blIb+435YYxZ665oOig9zIb4BG6alNMXB5/WnDFnKR5SHSfxsi+yyJGNuyDkmPTku5gPusVanpI9YR1MQ4=\ntrusted comment: capsem test fixture\nwyK54SForvZTNYj5/Vn/sScn9kPTutpmSZ27MaZAV8QAspbtH1NKTrCuEw9VVb8r/EOOUWycImpo95puXB/KDg==\n";

    #[test]
    fn profile_payload_signature_uses_minisign_verification() {
        verify_profile_payload_signature(TEST_PUBKEY, TEST_SIGNED_BYTES, TEST_SIGNATURE).unwrap();
    }

    #[test]
    fn profile_payload_signature_rejects_tampered_payload() {
        let error = verify_profile_payload_signature(
            TEST_PUBKEY,
            b"{\"hello\":\"tampered\",\"format\":2}",
            TEST_SIGNATURE,
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("profile payload signature"));
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
