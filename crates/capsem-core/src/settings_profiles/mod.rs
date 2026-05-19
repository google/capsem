use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

pub mod corp;
pub mod resolver_trace;

pub use corp::{apply_corp_directives, CorpDirective, CorpDirectiveOperation, CorpOverrides};
pub use resolver_trace::{
    load_vm_effective_trace, vm_effective_trace_path, write_vm_effective_trace, ResolverTrace,
    ResolverTraceEvent, ResolverTraceOperation, ResolverTraceSourceKind, ResolverTraceSummary,
    VM_EFFECTIVE_TRACE_FILENAME,
};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;
pub const EVERYDAY_WORK_PROFILE_ID: &str = "everyday-work";
pub const VM_EFFECTIVE_SETTINGS_FILENAME: &str = "vm-effective-settings.toml";
pub const DEFAULT_PROFILE_ICON_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><rect x="6" y="8" width="36" height="32" rx="8" fill="none" stroke="currentColor" stroke-width="3"/><path d="M15 20h18M15 28h12" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"/></svg>"#;

#[derive(Debug, Error)]
pub enum SettingsProfilesError {
    #[error("failed to parse {kind} TOML: {details}")]
    Parse { kind: &'static str, details: String },
    #[error("failed to read {path:?}: {details}")]
    ReadFile { path: PathBuf, details: String },
    #[error("failed to write {path:?}: {details}")]
    WriteFile { path: PathBuf, details: String },
    #[error("failed to remove {path:?}: {details}")]
    RemoveFile { path: PathBuf, details: String },
    #[error("failed to serialize {kind}: {details}")]
    Serialize { kind: &'static str, details: String },
    #[error("duplicate profile id '{id}' from {first} and {second}")]
    DuplicateProfile {
        id: String,
        first: String,
        second: String,
    },
    #[error("profile '{id}' not found")]
    ProfileNotFound { id: String },
    #[error("profile '{id}' references unknown parent profile '{parent}'")]
    UnknownParentProfile { id: String, parent: String },
    #[error("profile inheritance cycle detected: {chain}")]
    InheritanceCycle { chain: String },
    #[error("profile inheritance for '{id}' exceeds the maximum depth of {max} (chain: {chain})")]
    InheritanceDepthExceeded {
        id: String,
        max: usize,
        chain: String,
    },
    #[error("profile operation forbidden: {message}")]
    Forbidden { message: String },
    #[error(
        "rule '{rule_id}' is managed by setting '{owner_setting_path}' and cannot be edited directly; modify the setting instead"
    )]
    RuleManagedBySetting {
        rule_id: String,
        owner_setting_path: String,
    },
    #[error("{path}: {message}")]
    Validation { path: String, message: String },
    #[error(
        "resolver violation at '{path}' (source layer: {source_layer}, controlling rule: {controlling_rule}): {message}"
    )]
    ResolverViolation {
        path: String,
        source_layer: String,
        controlling_rule: String,
        message: String,
    },
}

/// Maximum number of ancestors a profile may declare via
/// `extends_profile_id`. Set to 8 to comfortably cover plausible
/// corp/base/user/local layering without permitting unbounded
/// chains that complicate resolver tracing.
pub const MAX_PROFILE_INHERITANCE_DEPTH: usize = 8;

/// Valid priority range for any rule. Corp-only and catch-all
/// further restrict where in this range rules may land:
///   - `[-1000, -1]`: corp-exclusive. Rules at these priorities
///     are only valid inside [`ProfileSource::Corp`] profiles or
///     `corp_directives` entries; non-corp profiles are rejected.
///   - `0`: reserved by convention for system-generated
///     toggle-derived rules (provider toggles, MCP
///     `allowed_tools`). Users CAN write here if they hand-edit
///     their file; the UI defaults to `1`.
///   - `[1, 999]`: user-authored. Recommended range for
///     interactive rule editing.
///   - `1000`: catch-all reserved. Manual authoring at this
///     priority is rejected; only the resolver may emit
///     catch-all rules here.
pub const RULE_PRIORITY_RANGE: std::ops::RangeInclusive<i32> = -1000..=1000;

/// Priority value reserved for the per-type catch-all rules
/// emitted by the resolver. Manual authoring at this priority
/// is rejected.
pub const RULE_CATCH_ALL_PRIORITY: i32 = 1000;

/// Priority range that is corp-exclusive. Rules with priorities
/// in this range are only valid in corp profiles or
/// `corp_directives` entries.
pub const RULE_CORP_PRIORITY_RANGE: std::ops::RangeInclusive<i32> = -1000..=-1;

pub type Result<T> = std::result::Result<T, SettingsProfilesError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ServiceSettings {
    #[serde(default = "schema_version")]
    pub version: u32,
    #[serde(default)]
    pub app: AppSettings,
    #[serde(default)]
    pub profiles: ProfileRootSettings,
    #[serde(default)]
    pub assets: AssetLocationSettings,
    #[serde(default)]
    pub credentials: CredentialSettings,
    #[serde(default)]
    pub telemetry: TelemetrySettings,
    #[serde(default)]
    pub remote_policy: RemotePolicySettings,
    #[serde(default)]
    pub profile_catalog: ProfileCatalogSettings,
    /// Org-deployed overrides applied after profile inheritance
    /// merges. Empty by default; serialized only when non-empty
    /// so existing `service.toml` files round-trip unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corp_directives: Vec<CorpDirective>,
}

impl Default for ServiceSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_SCHEMA_VERSION,
            app: AppSettings::default(),
            profiles: ProfileRootSettings::default(),
            assets: AssetLocationSettings::default(),
            credentials: CredentialSettings::default(),
            telemetry: TelemetrySettings::default(),
            remote_policy: RemotePolicySettings::default(),
            profile_catalog: ProfileCatalogSettings::default(),
            corp_directives: Vec::new(),
        }
    }
}

impl ServiceSettings {
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let settings =
            toml::from_str::<Self>(input).map_err(|source| SettingsProfilesError::Parse {
                kind: "service settings",
                details: source.to_string(),
            })?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<()> {
        validate_schema_version("version", self.version)?;
        self.app.validate("app")?;
        self.profiles.validate("profiles")?;
        self.assets.validate("assets")?;
        self.credentials.validate("credentials")?;
        self.telemetry.validate("telemetry")?;
        self.remote_policy.validate("remote_policy")?;
        self.profile_catalog.validate("profile_catalog")?;
        for (idx, directive) in self.corp_directives.iter().enumerate() {
            directive.validate(&format!("corp_directives[{idx}]"))?;
        }
        Ok(())
    }
}

pub fn load_service_settings(path: impl AsRef<Path>) -> Result<ServiceSettings> {
    let path = path.as_ref();
    let input = fs::read_to_string(path).map_err(|source| SettingsProfilesError::ReadFile {
        path: path.to_path_buf(),
        details: source.to_string(),
    })?;
    ServiceSettings::from_toml_str(&input)
}

pub fn load_service_settings_or_default(path: impl AsRef<Path>) -> Result<ServiceSettings> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(input) => ServiceSettings::from_toml_str(&input),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(ServiceSettings::default())
        }
        Err(source) => Err(SettingsProfilesError::ReadFile {
            path: path.to_path_buf(),
            details: source.to_string(),
        }),
    }
}

pub fn write_service_settings(path: impl AsRef<Path>, settings: &ServiceSettings) -> Result<()> {
    let path = path.as_ref();
    settings.validate()?;
    let payload =
        toml::to_string_pretty(settings).map_err(|source| SettingsProfilesError::Serialize {
            kind: "service settings",
            details: source.to_string(),
        })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsProfilesError::WriteFile {
            path: parent.to_path_buf(),
            details: source.to_string(),
        })?;
    }
    fs::write(path, payload).map_err(|source| SettingsProfilesError::WriteFile {
        path: path.to_path_buf(),
        details: source.to_string(),
    })
}

/// Install a corp-managed profile TOML into the configured corp profile roots.
///
/// This writes `<capsem_home>/service.toml` if needed to ensure at least one
/// corp profile directory is configured, then writes the parsed profile as
/// `<corp_dir>/<profile_id>.toml`.
pub fn install_corp_profile_toml(
    capsem_home: impl AsRef<Path>,
    toml_content: &str,
) -> Result<PathBuf> {
    let capsem_home = capsem_home.as_ref();
    let settings_path = capsem_home.join("service.toml");
    let mut settings = load_service_settings_or_default(&settings_path)?;
    let profile = Profile::from_toml_str(toml_content)?;

    let corp_dir = if let Some(first) = settings.profiles.corp_dirs.first() {
        first.clone()
    } else {
        capsem_home.join("profiles").join("corp")
    };
    if settings.profiles.corp_dirs.is_empty() {
        settings.profiles.corp_dirs.push(corp_dir.clone());
        write_service_settings(&settings_path, &settings)?;
    }

    fs::create_dir_all(&corp_dir).map_err(|source| SettingsProfilesError::WriteFile {
        path: corp_dir.clone(),
        details: source.to_string(),
    })?;
    let profile_path = corp_dir.join(format!("{}.toml", profile.id));
    let payload =
        toml::to_string_pretty(&profile).map_err(|source| SettingsProfilesError::Serialize {
            kind: "profile",
            details: source.to_string(),
        })?;
    fs::write(&profile_path, payload).map_err(|source| SettingsProfilesError::WriteFile {
        path: profile_path.clone(),
        details: source.to_string(),
    })?;
    Ok(profile_path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledProfileRevision {
    pub profile_id: String,
    pub revision: String,
    pub payload_hash: String,
    pub runtime_profile_path: PathBuf,
    pub payload_path: PathBuf,
    pub current_record_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstalledProfileRevisionRecord {
    pub profile_id: String,
    pub revision: String,
    pub payload_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileRevisionReconcileOutcome {
    Installed(InstalledProfileRevision),
    Unchanged(InstalledProfileRevisionRecord),
    DeprecatedKept(InstalledProfileRevisionRecord),
    DeprecatedNotInstalled {
        profile_id: String,
        revision: String,
    },
    RevokedRemoved {
        profile_id: String,
        revision: String,
    },
    RevokedNotInstalled {
        profile_id: String,
        revision: String,
    },
    AbsentRemoved {
        profile_id: String,
        revision: String,
    },
}

pub async fn reconcile_profile_revision_from_manifest(
    roots: &ProfileRootSettings,
    revision: crate::profile_manifest::ResolvedProfileRevision<'_>,
    profile_payload_pubkey: &str,
) -> anyhow::Result<ProfileRevisionReconcileOutcome> {
    roots.validate("profiles")?;
    match revision.record.status {
        crate::profile_manifest::ProfileRevisionStatus::Active => {
            if let Some(installed) = load_installed_profile_revision(roots, revision.profile_id)? {
                if installed.revision == revision.revision
                    && installed.payload_hash == revision.record.profile_hash
                    && installed_profile_revision_is_complete(roots, &installed)?
                {
                    return Ok(ProfileRevisionReconcileOutcome::Unchanged(installed));
                }
            }
            let verified = crate::profile_manifest::fetch_installable_profile_payload(
                revision,
                profile_payload_pubkey,
            )
            .await?;
            let installed = install_verified_profile_payload(roots, &verified)?;
            Ok(ProfileRevisionReconcileOutcome::Installed(installed))
        }
        crate::profile_manifest::ProfileRevisionStatus::Deprecated => {
            if let Some(installed) = load_installed_profile_revision(roots, revision.profile_id)? {
                if installed.revision == revision.revision {
                    return Ok(ProfileRevisionReconcileOutcome::DeprecatedKept(installed));
                }
            }
            Ok(ProfileRevisionReconcileOutcome::DeprecatedNotInstalled {
                profile_id: revision.profile_id.to_string(),
                revision: revision.revision.to_string(),
            })
        }
        crate::profile_manifest::ProfileRevisionStatus::Revoked => {
            if let Some(installed) = load_installed_profile_revision(roots, revision.profile_id)? {
                if installed.revision == revision.revision {
                    remove_launchable_installed_profile_revision(roots, revision.profile_id)?;
                    return Ok(ProfileRevisionReconcileOutcome::RevokedRemoved {
                        profile_id: revision.profile_id.to_string(),
                        revision: revision.revision.to_string(),
                    });
                }
            }
            Ok(ProfileRevisionReconcileOutcome::RevokedNotInstalled {
                profile_id: revision.profile_id.to_string(),
                revision: revision.revision.to_string(),
            })
        }
    }
}

pub fn reconcile_absent_installed_profiles_from_manifest(
    roots: &ProfileRootSettings,
    manifest: &crate::profile_manifest::ProfileManifest,
) -> Result<Vec<ProfileRevisionReconcileOutcome>> {
    roots.validate("profiles")?;
    let installed = list_installed_profile_revisions(roots)?;
    let manifest_profiles = manifest.profiles.keys().collect::<BTreeSet<_>>();
    let mut outcomes = Vec::new();
    for record in installed {
        if manifest_profiles.contains(&record.profile_id) {
            continue;
        }
        remove_launchable_installed_profile_revision(roots, &record.profile_id)?;
        outcomes.push(ProfileRevisionReconcileOutcome::AbsentRemoved {
            profile_id: record.profile_id,
            revision: record.revision,
        });
    }
    Ok(outcomes)
}

pub fn installed_profile_asset_filenames(roots: &ProfileRootSettings) -> Result<BTreeSet<String>> {
    roots.validate("profiles")?;
    let mut filenames = BTreeSet::new();
    for installed in list_installed_profile_revisions(roots)? {
        let Some(corp_dir) = roots.corp_dirs.first() else {
            break;
        };
        let payload_path = corp_dir
            .join(".catalog")
            .join("profiles")
            .join(&installed.profile_id)
            .join(&installed.revision)
            .join("profile.json");
        if !payload_path.exists() {
            continue;
        }
        let payload = fs::read_to_string(&payload_path).map_err(|source| {
            SettingsProfilesError::ReadFile {
                path: payload_path.clone(),
                details: source.to_string(),
            }
        })?;
        let value = serde_json::from_str::<serde_json::Value>(&payload).map_err(|source| {
            SettingsProfilesError::Parse {
                kind: "installed profile payload",
                details: source.to_string(),
            }
        })?;
        collect_profile_payload_asset_filenames(&value, &mut filenames);
    }
    Ok(filenames)
}

fn collect_profile_payload_asset_filenames(
    payload: &serde_json::Value,
    filenames: &mut BTreeSet<String>,
) {
    let Some(assets_by_arch) = payload
        .get("vm")
        .and_then(|vm| vm.get("assets"))
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    for assets in assets_by_arch.values() {
        for (logical_name, key) in [
            ("vmlinuz", "kernel"),
            ("initrd.img", "initrd"),
            ("rootfs.squashfs", "rootfs"),
        ] {
            let Some(hash) = assets
                .get(key)
                .and_then(|asset| asset.get("hash"))
                .and_then(serde_json::Value::as_str)
                .and_then(|hash| hash.strip_prefix("blake3:"))
            else {
                continue;
            };
            filenames.insert(crate::asset_manager::hash_filename(logical_name, hash));
        }
    }
}

pub fn install_verified_profile_payload(
    roots: &ProfileRootSettings,
    verified: &crate::profile_manifest::VerifiedProfilePayload,
) -> Result<InstalledProfileRevision> {
    roots.validate("profiles")?;
    let corp_dir = roots
        .corp_dirs
        .first()
        .ok_or_else(|| SettingsProfilesError::Forbidden {
            message: "no corp profile directory is configured".to_string(),
        })?;
    let profile = Profile::from_profile_payload_v2_value(verified.value.clone())?;
    if profile.id != verified.profile_id {
        return Err(SettingsProfilesError::Validation {
            path: "profile_payload.id".to_string(),
            message: format!(
                "runtime profile id '{}' does not match verified profile '{}'",
                profile.id, verified.profile_id
            ),
        });
    }

    let revision_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join(&verified.profile_id)
        .join(&verified.revision);
    fs::create_dir_all(&revision_dir).map_err(|source| SettingsProfilesError::WriteFile {
        path: revision_dir.clone(),
        details: source.to_string(),
    })?;
    let payload_path = revision_dir.join("profile.json");
    fs::write(&payload_path, &verified.payload_json).map_err(|source| {
        SettingsProfilesError::WriteFile {
            path: payload_path.clone(),
            details: source.to_string(),
        }
    })?;

    fs::create_dir_all(corp_dir).map_err(|source| SettingsProfilesError::WriteFile {
        path: corp_dir.clone(),
        details: source.to_string(),
    })?;
    let runtime_profile_path = corp_dir.join(format!("{}.toml", profile.id));
    let runtime_payload =
        toml::to_string_pretty(&profile).map_err(|source| SettingsProfilesError::Serialize {
            kind: "profile",
            details: source.to_string(),
        })?;
    fs::write(&runtime_profile_path, runtime_payload).map_err(|source| {
        SettingsProfilesError::WriteFile {
            path: runtime_profile_path.clone(),
            details: source.to_string(),
        }
    })?;

    let current_record_path = corp_profile_revision_current_path(corp_dir, &verified.profile_id);
    let current_record = InstalledProfileRevisionRecord {
        profile_id: verified.profile_id.clone(),
        revision: verified.revision.clone(),
        payload_hash: verified.payload_hash.clone(),
    };
    let current_record_payload =
        serde_json::to_string_pretty(&current_record).map_err(|source| {
            SettingsProfilesError::Serialize {
                kind: "installed profile revision",
                details: source.to_string(),
            }
        })?;
    fs::write(&current_record_path, current_record_payload).map_err(|source| {
        SettingsProfilesError::WriteFile {
            path: current_record_path.clone(),
            details: source.to_string(),
        }
    })?;

    Ok(InstalledProfileRevision {
        profile_id: verified.profile_id.clone(),
        revision: verified.revision.clone(),
        payload_hash: verified.payload_hash.clone(),
        runtime_profile_path,
        payload_path,
        current_record_path,
    })
}

pub fn load_installed_profile_revision(
    roots: &ProfileRootSettings,
    profile_id: &str,
) -> Result<Option<InstalledProfileRevisionRecord>> {
    validate_profile_id("profile_id", profile_id)?;
    let Some(corp_dir) = roots.corp_dirs.first() else {
        return Ok(None);
    };
    let path = corp_profile_revision_current_path(corp_dir, profile_id);
    if !path.exists() {
        return Ok(None);
    }
    let input = fs::read_to_string(&path).map_err(|source| SettingsProfilesError::ReadFile {
        path: path.clone(),
        details: source.to_string(),
    })?;
    let record =
        serde_json::from_str::<InstalledProfileRevisionRecord>(&input).map_err(|source| {
            SettingsProfilesError::Parse {
                kind: "installed profile revision",
                details: source.to_string(),
            }
        })?;
    if record.profile_id != profile_id {
        return Err(SettingsProfilesError::Validation {
            path: "installed_profile_revision.profile_id".to_string(),
            message: format!(
                "installed profile revision id '{}' does not match requested profile '{}'",
                record.profile_id, profile_id
            ),
        });
    }
    validate_profile_id("installed_profile_revision.profile_id", &record.profile_id)?;
    Ok(Some(record))
}

fn list_installed_profile_revisions(
    roots: &ProfileRootSettings,
) -> Result<Vec<InstalledProfileRevisionRecord>> {
    let Some(corp_dir) = roots.corp_dirs.first() else {
        return Ok(Vec::new());
    };
    let catalog_profiles_dir = corp_dir.join(".catalog").join("profiles");
    if !catalog_profiles_dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(&catalog_profiles_dir)
        .map_err(|source| SettingsProfilesError::ReadFile {
            path: catalog_profiles_dir.clone(),
            details: source.to_string(),
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| SettingsProfilesError::ReadFile {
            path: catalog_profiles_dir.clone(),
            details: source.to_string(),
        })?;
    entries.sort_by_key(|entry| entry.path());

    let mut installed = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(profile_id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(record) = load_installed_profile_revision(roots, profile_id)? {
            installed.push(record);
        }
    }
    Ok(installed)
}

fn corp_profile_revision_current_path(corp_dir: &Path, profile_id: &str) -> PathBuf {
    corp_dir
        .join(".catalog")
        .join("profiles")
        .join(profile_id)
        .join("current.json")
}

fn installed_profile_revision_is_complete(
    roots: &ProfileRootSettings,
    installed: &InstalledProfileRevisionRecord,
) -> Result<bool> {
    let Some(corp_dir) = roots.corp_dirs.first() else {
        return Ok(false);
    };
    let runtime_profile_path = corp_dir.join(format!("{}.toml", installed.profile_id));
    let payload_path = corp_dir
        .join(".catalog")
        .join("profiles")
        .join(&installed.profile_id)
        .join(&installed.revision)
        .join("profile.json");
    Ok(runtime_profile_path.is_file() && payload_path.is_file())
}

fn remove_launchable_installed_profile_revision(
    roots: &ProfileRootSettings,
    profile_id: &str,
) -> Result<()> {
    validate_profile_id("profile_id", profile_id)?;
    let corp_dir = roots
        .corp_dirs
        .first()
        .ok_or_else(|| SettingsProfilesError::Forbidden {
            message: "no corp profile directory is configured".to_string(),
        })?;
    remove_file_if_exists(&corp_dir.join(format!("{profile_id}.toml")))?;
    remove_file_if_exists(&corp_profile_revision_current_path(corp_dir, profile_id))?;
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SettingsProfilesError::RemoveFile {
            path: path.to_path_buf(),
            details: source.to_string(),
        }),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceSettingOrigin {
    Cli,
    ServiceSettings,
    Default,
}

impl ServiceSettingOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::ServiceSettings => "service_settings",
            Self::Default => "default",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedServiceAssetLocations {
    pub assets_dir: PathBuf,
    pub assets_dir_origin: ServiceSettingOrigin,
    pub image_roots: Vec<PathBuf>,
    pub image_roots_origin: ServiceSettingOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_base_url: Option<String>,
}

pub fn resolve_service_asset_locations(
    settings: &ServiceSettings,
    cli_assets_dir: Option<PathBuf>,
    installed_default_assets_dir: Option<PathBuf>,
    fallback_assets_dir: PathBuf,
) -> Result<ResolvedServiceAssetLocations> {
    settings.validate()?;
    validate_path("assets.fallback_assets_dir", &fallback_assets_dir)?;

    let (assets_dir, assets_dir_origin) = if let Some(path) = cli_assets_dir {
        validate_path("assets.assets_dir", &path)?;
        (path, ServiceSettingOrigin::Cli)
    } else if let Some(path) = settings.assets.assets_dir.clone() {
        (path, ServiceSettingOrigin::ServiceSettings)
    } else if let Some(path) = installed_default_assets_dir {
        validate_path("assets.installed_default_assets_dir", &path)?;
        (path, ServiceSettingOrigin::Default)
    } else {
        (fallback_assets_dir, ServiceSettingOrigin::Default)
    };

    let (image_roots, image_roots_origin) = if settings.assets.image_roots.is_empty() {
        (Vec::new(), ServiceSettingOrigin::Default)
    } else {
        (
            settings.assets.image_roots.clone(),
            ServiceSettingOrigin::ServiceSettings,
        )
    };

    Ok(ResolvedServiceAssetLocations {
        assets_dir,
        assets_dir_origin,
        image_roots,
        image_roots_origin,
        download_base_url: settings.assets.download_base_url.clone(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct AssetLocationSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets_dir: Option<PathBuf>,
    #[serde(default)]
    pub image_roots: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_base_url: Option<String>,
}

impl AssetLocationSettings {
    fn validate(&self, path: &str) -> Result<()> {
        if let Some(assets_dir) = &self.assets_dir {
            validate_path(&format!("{path}.assets_dir"), assets_dir)?;
        }
        validate_paths(&format!("{path}.image_roots"), &self.image_roots)?;
        if let Some(endpoint) = self.download_base_url.as_deref() {
            validate_endpoint(&format!("{path}.download_base_url"), endpoint)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppSettings {
    #[serde(default = "default_true")]
    pub auto_launch: bool,
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_config_path: Option<PathBuf>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_launch: true,
            appearance: AppearanceSettings::default(),
            google_config_path: None,
        }
    }
}

impl AppSettings {
    fn validate(&self, path: &str) -> Result<()> {
        if let Some(config_path) = &self.google_config_path {
            if config_path.as_os_str().is_empty() {
                validation_error(path, "google_config_path cannot be empty")?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppearanceSettings {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "default_accent")]
    pub accent: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: default_accent(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileRootSettings {
    #[serde(default = "default_base_profile_dirs")]
    pub base_dirs: Vec<PathBuf>,
    #[serde(default)]
    pub corp_dirs: Vec<PathBuf>,
    #[serde(default = "default_user_profile_dirs")]
    pub user_dirs: Vec<PathBuf>,
    #[serde(default = "default_profile_id")]
    pub default_profile: String,
    #[serde(default = "default_true")]
    pub allow_user_profiles: bool,
    #[serde(default = "default_true")]
    pub allow_user_fork: bool,
    #[serde(default = "default_true")]
    pub allow_user_delete: bool,
}

impl Default for ProfileRootSettings {
    fn default() -> Self {
        Self {
            base_dirs: default_base_profile_dirs(),
            corp_dirs: Vec::new(),
            user_dirs: default_user_profile_dirs(),
            default_profile: default_profile_id(),
            allow_user_profiles: true,
            allow_user_fork: true,
            allow_user_delete: true,
        }
    }
}

impl ProfileRootSettings {
    fn validate(&self, path: &str) -> Result<()> {
        validate_profile_id(&format!("{path}.default_profile"), &self.default_profile)?;
        if self.base_dirs.is_empty() {
            validation_error(
                &format!("{path}.base_dirs"),
                "at least one base profile directory is required",
            )?;
        }
        validate_paths(&format!("{path}.base_dirs"), &self.base_dirs)?;
        validate_paths(&format!("{path}.corp_dirs"), &self.corp_dirs)?;
        validate_paths(&format!("{path}.user_dirs"), &self.user_dirs)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CredentialSettings {
    #[serde(default)]
    pub backend: CredentialBackend,
    #[serde(default)]
    pub items: BTreeMap<String, TomlCredential>,
}

impl Default for CredentialSettings {
    fn default() -> Self {
        Self {
            backend: CredentialBackend::Toml,
            items: BTreeMap::new(),
        }
    }
}

impl CredentialSettings {
    fn validate(&self, path: &str) -> Result<()> {
        for (id, credential) in &self.items {
            validate_config_id(&format!("{path}.items"), id)?;
            credential.validate(&format!("{path}.items.{id}"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialBackend {
    #[default]
    Toml,
    Keychain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TomlCredential {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub value: String,
}

impl TomlCredential {
    fn validate(&self, path: &str) -> Result<()> {
        if self.value.trim().is_empty() {
            validation_error(&format!("{path}.value"), "credential value cannot be empty")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TelemetrySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_telemetry_batch_max_events")]
    pub batch_max_events: u16,
    #[serde(default = "default_telemetry_flush_interval_ms")]
    pub flush_interval_ms: u64,
    #[serde(default = "default_true")]
    pub redact_secrets: bool,
    #[serde(default = "default_telemetry_retry_attempts")]
    pub retry_attempts: u8,
    #[serde(default)]
    pub failure_mode: TelemetryFailureMode,
}

impl Default for TelemetrySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            headers: BTreeMap::new(),
            batch_max_events: default_telemetry_batch_max_events(),
            flush_interval_ms: default_telemetry_flush_interval_ms(),
            redact_secrets: true,
            retry_attempts: default_telemetry_retry_attempts(),
            failure_mode: TelemetryFailureMode::Drop,
        }
    }
}

impl TelemetrySettings {
    fn validate(&self, path: &str) -> Result<()> {
        validate_optional_endpoint(path, self.enabled, self.endpoint.as_deref())?;
        if self.batch_max_events == 0 {
            validation_error(
                &format!("{path}.batch_max_events"),
                "batch_max_events must be greater than zero",
            )?;
        }
        if self.flush_interval_ms == 0 {
            validation_error(
                &format!("{path}.flush_interval_ms"),
                "flush_interval_ms must be greater than zero",
            )?;
        }
        for (header, value) in &self.headers {
            if header.trim().is_empty() || value.trim().is_empty() {
                validation_error(
                    &format!("{path}.headers"),
                    "header names and values cannot be empty",
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TelemetryFailureMode {
    #[default]
    Drop,
    Disable,
    Backpressure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RemotePolicySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(default = "default_remote_policy_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub failure_mode: RemotePolicyFailureMode,
}

impl Default for RemotePolicySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            auth_token: None,
            timeout_ms: default_remote_policy_timeout_ms(),
            failure_mode: RemotePolicyFailureMode::FailClosed,
        }
    }
}

impl RemotePolicySettings {
    fn validate(&self, path: &str) -> Result<()> {
        validate_optional_endpoint(path, self.enabled, self.endpoint.as_deref())?;
        if self.timeout_ms < 100 || self.timeout_ms > 60_000 {
            validation_error(
                &format!("{path}.timeout_ms"),
                "timeout_ms must be between 100 and 60000",
            )?;
        }
        if self
            .auth_token
            .as_deref()
            .is_some_and(|token| token.trim().is_empty())
        {
            validation_error(&format!("{path}.auth_token"), "auth_token cannot be empty")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RemotePolicyFailureMode {
    FailOpen,
    #[default]
    FailClosed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileCatalogSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_payload_pubkey: Option<String>,
    #[serde(default = "default_profile_catalog_check_interval_secs")]
    pub check_interval_secs: u64,
}

impl Default for ProfileCatalogSettings {
    fn default() -> Self {
        Self {
            manifest_url: None,
            profile_payload_pubkey: None,
            check_interval_secs: default_profile_catalog_check_interval_secs(),
        }
    }
}

impl ProfileCatalogSettings {
    pub fn is_configured(&self) -> bool {
        self.manifest_url.is_some() || self.profile_payload_pubkey.is_some()
    }

    pub fn validate(&self, path: &str) -> Result<()> {
        match (
            self.manifest_url.as_deref(),
            self.profile_payload_pubkey.as_deref(),
        ) {
            (None, None) => {}
            (Some(url), Some(pubkey)) => {
                crate::profile_manifest::parse_profile_catalog_manifest_url(url).map_err(
                    |source| SettingsProfilesError::Validation {
                        path: format!("{path}.manifest_url"),
                        message: source.to_string(),
                    },
                )?;
                if pubkey.trim().is_empty() {
                    validation_error(
                        &format!("{path}.profile_payload_pubkey"),
                        "profile_payload_pubkey cannot be empty",
                    )?;
                }
            }
            (Some(_), None) => validation_error(
                &format!("{path}.profile_payload_pubkey"),
                "profile_payload_pubkey is required when manifest_url is set",
            )?,
            (None, Some(_)) => validation_error(
                &format!("{path}.manifest_url"),
                "manifest_url is required when profile_payload_pubkey is set",
            )?,
        }
        if self.check_interval_secs < 60 {
            validation_error(
                &format!("{path}.check_interval_secs"),
                "check_interval_secs must be at least 60",
            )?;
        }
        Ok(())
    }
}

fn default_profile_catalog_check_interval_secs() -> u64 {
    6 * 60 * 60
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default = "schema_version")]
    pub version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub best_for: String,
    #[serde(default)]
    pub profile_type: ProfileType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_svg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends_profile_id: Option<String>,
    #[serde(default)]
    pub general: ProfileGeneralSettings,
    #[serde(default)]
    pub appearance: ProfileAppearanceSettings,
    #[serde(default)]
    pub ai: AiProvidersProfileSettings,
    #[serde(default)]
    pub mcp: McpConnectorsProfileSettings,
    #[serde(default)]
    pub skills: SkillsProfileSettings,
    #[serde(default)]
    pub packages: ProfilePackageContract,
    #[serde(default)]
    pub tools: BTreeMap<String, ProfileToolContract>,
    #[serde(default)]
    pub vm: VmProfileSettings,
    #[serde(default)]
    pub security: SecurityProfileSettings,
}

impl Profile {
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let profile =
            toml::from_str::<Self>(input).map_err(|source| SettingsProfilesError::Parse {
                kind: "profile",
                details: source.to_string(),
            })?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn from_profile_payload_v2_value(mut value: serde_json::Value) -> Result<Self> {
        let Some(object) = value.as_object_mut() else {
            return Err(SettingsProfilesError::Validation {
                path: "profile_payload".to_string(),
                message: "profile payload must be an object".to_string(),
            });
        };
        object.remove("schema");
        object.remove("revision");
        object.remove("compatibility");
        object.remove("extends_profile_revision");
        object.insert("version".to_string(), json!(SETTINGS_SCHEMA_VERSION));
        if let Some(vm) = object
            .get_mut("vm")
            .and_then(serde_json::Value::as_object_mut)
        {
            vm.remove("disk_mib");
        }

        let profile = serde_json::from_value::<Self>(value).map_err(|source| {
            SettingsProfilesError::Parse {
                kind: "profile",
                details: source.to_string(),
            }
        })?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn everyday_work() -> Self {
        Self {
            version: SETTINGS_SCHEMA_VERSION,
            id: EVERYDAY_WORK_PROFILE_ID.to_string(),
            name: "Everyday Work".to_string(),
            description: "Balanced defaults for daily work sessions.".to_string(),
            best_for: "Daily work with useful tools and measured security prompts.".to_string(),
            profile_type: ProfileType::EverydayWork,
            icon_svg: None,
            extends_profile_id: None,
            general: ProfileGeneralSettings::default(),
            appearance: ProfileAppearanceSettings::default(),
            ai: AiProvidersProfileSettings::default(),
            mcp: McpConnectorsProfileSettings::default(),
            skills: SkillsProfileSettings::default(),
            packages: ProfilePackageContract::default(),
            tools: BTreeMap::new(),
            vm: VmProfileSettings::default(),
            security: everyday_work_security_settings(),
        }
    }

    pub fn icon_svg_or_default(&self) -> &str {
        self.icon_svg.as_deref().unwrap_or(DEFAULT_PROFILE_ICON_SVG)
    }

    pub fn validate(&self) -> Result<()> {
        validate_schema_version("version", self.version)?;
        validate_profile_id("id", &self.id)?;
        if self.name.trim().is_empty() {
            validation_error("name", "profile name cannot be empty")?;
        }
        if self.best_for.trim().is_empty() {
            validation_error("best_for", "profile best_for cannot be empty")?;
        }
        if let Some(svg) = &self.icon_svg {
            let trimmed = svg.trim_start();
            if !trimmed.starts_with("<svg") {
                validation_error("icon_svg", "profile icon must be inline SVG")?;
            }
        }
        if let Some(parent_id) = self.extends_profile_id.as_deref() {
            validate_profile_id("extends_profile_id", parent_id)?;
            if parent_id == self.id {
                validation_error(
                    "extends_profile_id",
                    "extends_profile_id cannot reference the profile itself",
                )?;
            }
        }
        self.ai.validate("ai")?;
        self.mcp.validate("mcp")?;
        self.skills.validate("skills")?;
        self.packages.validate("packages")?;
        validate_tool_contracts("tools", &self.tools)?;
        self.vm.validate("vm")?;
        self.security.validate("security")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileType {
    #[default]
    EverydayWork,
    Coding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ProfileGeneralSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileAppearanceSettings {
    #[serde(default)]
    pub theme: ProfileTheme,
    #[serde(default = "default_accent")]
    pub accent: String,
}

impl Default for ProfileAppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ProfileTheme::InheritService,
            accent: default_accent(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileTheme {
    #[default]
    InheritService,
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct AiProvidersProfileSettings {
    #[serde(default)]
    pub providers: BTreeMap<String, AiProviderConfig>,
}

impl AiProvidersProfileSettings {
    fn validate(&self, path: &str) -> Result<()> {
        for (id, provider) in &self.providers {
            validate_config_id(&format!("{path}.providers"), id)?;
            provider.validate(&format!("{path}.providers.{id}"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiProviderConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub credential_refs: Vec<String>,
    /// Rules nested under this provider host (corp authors
    /// usually own this; user profiles can use it too -- their
    /// file, their choice). The resolver picks these up at
    /// materialization time and tags each emitted rule with
    /// `owner_setting_path = "ai.providers.<name>"`.
    #[serde(default, skip_serializing_if = "SecurityRules::is_empty")]
    pub rules: SecurityRules,
}

impl AiProviderConfig {
    fn validate(&self, path: &str) -> Result<()> {
        if let Some(base_url) = self.base_url.as_deref() {
            validate_endpoint(&format!("{path}.base_url"), base_url)?;
        }
        validate_string_ids(&format!("{path}.credential_refs"), &self.credential_refs)?;
        self.rules.validate(&format!("{path}.rules"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct McpConnectorsProfileSettings {
    #[serde(default)]
    pub connectors: BTreeMap<String, McpConnectorConfig>,
}

impl McpConnectorsProfileSettings {
    fn validate(&self, path: &str) -> Result<()> {
        for (id, connector) in &self.connectors {
            validate_config_id(&format!("{path}.connectors"), id)?;
            connector.validate(&format!("{path}.connectors.{id}"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpConnectorConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub connector_type: ConnectorType,
    #[serde(default)]
    pub credential_refs: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Rules nested under this connector host. Resolver tags
    /// each emitted rule with
    /// `owner_setting_path = "mcp.connectors.<name>"`.
    #[serde(default, skip_serializing_if = "SecurityRules::is_empty")]
    pub rules: SecurityRules,
}

impl McpConnectorConfig {
    fn validate(&self, path: &str) -> Result<()> {
        validate_string_ids(&format!("{path}.credential_refs"), &self.credential_refs)?;
        for tool in &self.allowed_tools {
            if tool.trim().is_empty() {
                validation_error(&format!("{path}.allowed_tools"), "tool id cannot be empty")?;
            }
        }
        self.rules.validate(&format!("{path}.rules"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectorType {
    #[default]
    Mcp,
    Repository,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct SkillsProfileSettings {
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub enabled: Vec<String>,
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl SkillsProfileSettings {
    fn validate(&self, path: &str) -> Result<()> {
        validate_string_ids(&format!("{path}.groups"), &self.groups)?;
        validate_string_ids(&format!("{path}.enabled"), &self.enabled)?;
        validate_string_ids(&format!("{path}.disabled"), &self.disabled)?;
        ensure_no_duplicate_ids(&format!("{path}.enabled"), &self.enabled)?;
        ensure_no_duplicate_ids(&format!("{path}.disabled"), &self.disabled)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ProfilePackageContract {
    #[serde(default)]
    pub runtimes: BTreeMap<String, String>,
    #[serde(default)]
    pub python_modules: BTreeMap<String, String>,
    #[serde(default)]
    pub node_packages: BTreeMap<String, String>,
    #[serde(default)]
    pub system: SystemPackageContract,
}

impl ProfilePackageContract {
    fn validate(&self, path: &str) -> Result<()> {
        validate_package_version_map(&format!("{path}.runtimes"), &self.runtimes)?;
        validate_package_version_map(&format!("{path}.python_modules"), &self.python_modules)?;
        validate_package_version_map(&format!("{path}.node_packages"), &self.node_packages)?;
        self.system.validate(&format!("{path}.system"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct SystemPackageContract {
    #[serde(default)]
    pub distro: String,
    #[serde(default)]
    pub release: String,
    #[serde(default)]
    pub apt: BTreeMap<String, String>,
}

impl SystemPackageContract {
    fn validate(&self, path: &str) -> Result<()> {
        validate_optional_non_empty_string(&format!("{path}.distro"), &self.distro)?;
        validate_optional_non_empty_string(&format!("{path}.release"), &self.release)?;
        if self.distro.is_empty() != self.release.is_empty() {
            validation_error(
                path,
                "system package contract requires both distro and release",
            )?;
        }
        validate_package_version_map(&format!("{path}.apt"), &self.apt)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileToolContract {
    pub version: String,
    pub required: bool,
    pub source: ProfileToolSource,
}

impl ProfileToolContract {
    fn validate(&self, path: &str) -> Result<()> {
        validate_required_non_empty_string(&format!("{path}.version"), &self.version)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileToolSource {
    Guest,
    Host,
    Profile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VmProfileSettings {
    #[serde(default = "default_memory_mib")]
    pub memory_mib: u32,
    #[serde(default = "default_vcpu_count")]
    pub cpus: u8,
    #[serde(default)]
    pub network: VmNetworkMode,
    #[serde(default = "default_true")]
    pub track_rootfs_dependencies: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rootfs_image: Option<PathBuf>,
    #[serde(default)]
    pub assets: BTreeMap<String, VmArchAssets>,
}

impl Default for VmProfileSettings {
    fn default() -> Self {
        Self {
            memory_mib: default_memory_mib(),
            cpus: default_vcpu_count(),
            network: VmNetworkMode::Proxied,
            track_rootfs_dependencies: true,
            rootfs_image: None,
            assets: BTreeMap::new(),
        }
    }
}

impl VmProfileSettings {
    fn validate(&self, path: &str) -> Result<()> {
        if self.memory_mib < 512 {
            validation_error(
                &format!("{path}.memory_mib"),
                "memory_mib must be at least 512",
            )?;
        }
        if self.cpus == 0 {
            validation_error(&format!("{path}.cpus"), "cpus must be greater than zero")?;
        }
        if let Some(rootfs_image) = &self.rootfs_image {
            if rootfs_image.as_os_str().is_empty() {
                validation_error(
                    &format!("{path}.rootfs_image"),
                    "rootfs_image cannot be empty",
                )?;
            }
        }
        for (arch, assets) in &self.assets {
            validate_arch_id(&format!("{path}.assets"), arch)?;
            assets.validate(&format!("{path}.assets.{arch}"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VmArchAssets {
    pub kernel: VmAssetDeclaration,
    pub initrd: VmAssetDeclaration,
    pub rootfs: VmAssetDeclaration,
}

impl VmArchAssets {
    fn validate(&self, path: &str) -> Result<()> {
        self.kernel.validate(&format!("{path}.kernel"))?;
        self.initrd.validate(&format!("{path}.initrd"))?;
        self.rootfs.validate(&format!("{path}.rootfs"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VmAssetDeclaration {
    pub url: String,
    pub hash: String,
    pub signature_url: String,
    pub size: u64,
    pub content_type: String,
}

impl VmAssetDeclaration {
    fn validate(&self, path: &str) -> Result<()> {
        validate_profile_asset_location(&format!("{path}.url"), &self.url)?;
        validate_profile_hash(&format!("{path}.hash"), &self.hash)?;
        validate_profile_asset_location(&format!("{path}.signature_url"), &self.signature_url)?;
        if self.size == 0 {
            validation_error(&format!("{path}.size"), "size must be greater than zero")?;
        }
        validate_required_non_empty_string(&format!("{path}.content_type"), &self.content_type)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum VmNetworkMode {
    #[default]
    Proxied,
    Disabled,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct SecurityProfileSettings {
    #[serde(default)]
    pub capabilities: SecurityCapabilities,
    #[serde(default)]
    pub rules: SecurityRules,
}

impl SecurityProfileSettings {
    fn validate(&self, path: &str) -> Result<()> {
        self.rules.validate(&format!("{path}.rules"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct SecurityRules {
    #[serde(default)]
    pub mcp: BTreeMap<String, ProfileRule>,
    #[serde(default)]
    pub http: BTreeMap<String, ProfileRule>,
    #[serde(default)]
    pub dns: BTreeMap<String, ProfileRule>,
    #[serde(default)]
    pub model: BTreeMap<String, ProfileRule>,
    #[serde(default)]
    pub hook: BTreeMap<String, ProfileRule>,
}

impl SecurityRules {
    fn validate(&self, path: &str) -> Result<()> {
        validate_rule_map(path, "mcp", &self.mcp)?;
        validate_rule_map(path, "http", &self.http)?;
        validate_rule_map(path, "dns", &self.dns)?;
        validate_rule_map(path, "model", &self.model)?;
        validate_rule_map(path, "hook", &self.hook)?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.mcp.is_empty()
            && self.http.is_empty()
            && self.dns.is_empty()
            && self.model.is_empty()
            && self.hook.is_empty()
    }
}

fn everyday_work_security_settings() -> SecurityProfileSettings {
    let mut security = SecurityProfileSettings::default();
    for domain in [
        "elie.net",
        "*.elie.net",
        "en.wikipedia.org",
        "*.wikipedia.org",
    ] {
        let name = safe_rule_name(domain);
        security.rules.dns.insert(
            format!("allow_{name}"),
            ProfileRule {
                callback: "dns.request".to_string(),
                condition: format!("qname == '{domain}'"),
                decision: RuleDecision::Allow,
                priority: 1,
                rewrite_target: None,
                rewrite_value: None,
                strip_request_headers: Vec::new(),
                strip_response_headers: Vec::new(),
                reason: Some("Everyday Work default read allowlist".to_string()),
            },
        );
        security.rules.http.insert(
            format!("allow_{name}"),
            ProfileRule {
                callback: "http.request".to_string(),
                condition: format!("request.host == '{domain}'"),
                decision: RuleDecision::Allow,
                priority: 1,
                rewrite_target: None,
                rewrite_value: None,
                strip_request_headers: Vec::new(),
                strip_response_headers: Vec::new(),
                reason: Some("Everyday Work default read allowlist".to_string()),
            },
        );
    }
    security
}

fn safe_rule_name(input: &str) -> String {
    input
        .replace('*', "wildcard")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SecurityCapabilities {
    #[serde(default = "default_ask")]
    pub credential_brokerage: CapabilityMode,
    #[serde(default = "default_ask")]
    pub pii_detection: CapabilityMode,
    #[serde(default = "default_ask")]
    pub mcp_rag: CapabilityMode,
    #[serde(default = "default_ask")]
    pub mcp_tools: CapabilityMode,
    #[serde(default = "default_ask")]
    pub network_egress: CapabilityMode,
    #[serde(default = "default_ask")]
    pub file_boundaries: CapabilityMode,
    #[serde(default = "default_audit")]
    pub audit: CapabilityMode,
}

impl Default for SecurityCapabilities {
    fn default() -> Self {
        Self {
            credential_brokerage: CapabilityMode::Ask,
            pii_detection: CapabilityMode::Ask,
            mcp_rag: CapabilityMode::Ask,
            mcp_tools: CapabilityMode::Ask,
            network_egress: CapabilityMode::Ask,
            file_boundaries: CapabilityMode::Ask,
            audit: CapabilityMode::Audit,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityMode {
    Allow,
    Ask,
    Block,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileRule {
    #[serde(rename = "on")]
    pub callback: String,
    #[serde(rename = "if")]
    pub condition: String,
    pub decision: RuleDecision,
    #[serde(default = "default_rule_priority")]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strip_request_headers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strip_response_headers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ProfileRule {
    fn validate(&self, path: &str) -> Result<()> {
        if self.callback.trim().is_empty() {
            validation_error(&format!("{path}.on"), "callback cannot be empty")?;
        }
        if self.condition.trim().is_empty() {
            validation_error(&format!("{path}.if"), "condition cannot be empty")?;
        }
        if !RULE_PRIORITY_RANGE.contains(&self.priority) {
            validation_error(
                &format!("{path}.priority"),
                &format!(
                    "priority must be in [{min}, {max}], got {value}",
                    min = *RULE_PRIORITY_RANGE.start(),
                    max = *RULE_PRIORITY_RANGE.end(),
                    value = self.priority,
                ),
            )?;
        }
        if self.priority == RULE_CATCH_ALL_PRIORITY {
            validation_error(
                &format!("{path}.priority"),
                &format!(
                    "priority {RULE_CATCH_ALL_PRIORITY} is reserved for the system catch-all rule",
                ),
            )?;
        }
        let has_target = self
            .rewrite_target
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_value = self
            .rewrite_value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_header_strip =
            !self.strip_request_headers.is_empty() || !self.strip_response_headers.is_empty();
        match self.decision {
            RuleDecision::Rewrite => {
                if has_target != has_value {
                    validation_error(
                        path,
                        "rewrite decisions require both rewrite_target and rewrite_value",
                    )?;
                }
                if !has_target && !has_header_strip {
                    validation_error(
                        path,
                        "rewrite decisions require rewrite_target and rewrite_value or header strip fields",
                    )?;
                }
                if has_target {
                    validate_rewrite_target_and_value(
                        &format!("{path}.rewrite_target"),
                        self.rewrite_target.as_deref().unwrap_or_default(),
                        self.rewrite_value.as_deref().unwrap_or_default(),
                    )?;
                }
                validate_header_names(
                    &format!("{path}.strip_request_headers"),
                    &self.strip_request_headers,
                )?;
                validate_header_names(
                    &format!("{path}.strip_response_headers"),
                    &self.strip_response_headers,
                )?;
            }
            RuleDecision::Allow | RuleDecision::Ask | RuleDecision::Block => {
                if self.rewrite_target.is_some()
                    || self.rewrite_value.is_some()
                    || !self.strip_request_headers.is_empty()
                    || !self.strip_response_headers.is_empty()
                {
                    validation_error(
                        path,
                        "only rewrite decisions may include rewrite_target/rewrite_value or header strip fields",
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleDecision {
    Allow,
    Ask,
    Block,
    Rewrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SettingDescriptor {
    pub path: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub scope: SettingScope,
    pub widget: SettingWidget,
    pub default_value: serde_json::Value,
    pub sensitive: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SettingScope {
    Service,
    Profile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SettingWidget {
    Toggle,
    Text,
    Password,
    Select,
    Number,
    DirectoryList,
    Endpoint,
    CredentialMap,
    RuleBuilder,
    InfoBox,
}

pub fn service_setting_descriptors() -> Vec<SettingDescriptor> {
    vec![
        SettingDescriptor {
            path: "app.auto_launch",
            label: "Auto-launch",
            description: "Start Capsem's service companion at login.",
            scope: SettingScope::Service,
            widget: SettingWidget::Toggle,
            default_value: json!(true),
            sensitive: false,
        },
        SettingDescriptor {
            path: "profiles.base_dirs",
            label: "Base profile directories",
            description: "Root directories that contain package-provided profiles.",
            scope: SettingScope::Service,
            widget: SettingWidget::DirectoryList,
            default_value: json!(["/Library/Application Support/Capsem/profiles/base"]),
            sensitive: false,
        },
        SettingDescriptor {
            path: "credentials.items",
            label: "Credentials",
            description: "Credential values stored in service settings for the cutover.",
            scope: SettingScope::Service,
            widget: SettingWidget::CredentialMap,
            default_value: json!({}),
            sensitive: true,
        },
        SettingDescriptor {
            path: "assets.assets_dir",
            label: "Assets directory",
            description: "Directory for downloaded or installed VM boot assets.",
            scope: SettingScope::Service,
            widget: SettingWidget::Text,
            default_value: serde_json::Value::Null,
            sensitive: false,
        },
        SettingDescriptor {
            path: "assets.image_roots",
            label: "Image roots",
            description: "Directories containing custom or saved VM images.",
            scope: SettingScope::Service,
            widget: SettingWidget::DirectoryList,
            default_value: json!([]),
            sensitive: false,
        },
        SettingDescriptor {
            path: "assets.download_base_url",
            label: "Asset download endpoint",
            description: "Base endpoint used to download managed VM assets.",
            scope: SettingScope::Service,
            widget: SettingWidget::Endpoint,
            default_value: serde_json::Value::Null,
            sensitive: false,
        },
        SettingDescriptor {
            path: "telemetry.endpoint",
            label: "OpenTelemetry endpoint",
            description: "Service-scoped endpoint for event export.",
            scope: SettingScope::Service,
            widget: SettingWidget::Endpoint,
            default_value: serde_json::Value::Null,
            sensitive: false,
        },
        SettingDescriptor {
            path: "remote_policy.endpoint",
            label: "Remote policy endpoint",
            description: "Service-scoped endpoint for remote policy decisions.",
            scope: SettingScope::Service,
            widget: SettingWidget::Endpoint,
            default_value: serde_json::Value::Null,
            sensitive: false,
        },
    ]
}

pub fn profile_setting_descriptors() -> Vec<SettingDescriptor> {
    vec![
        SettingDescriptor {
            path: "name",
            label: "Name",
            description: "Human-readable profile name.",
            scope: SettingScope::Profile,
            widget: SettingWidget::Text,
            default_value: json!(""),
            sensitive: false,
        },
        SettingDescriptor {
            path: "best_for",
            label: "Best for",
            description: "Short guidance shown when choosing a profile.",
            scope: SettingScope::Profile,
            widget: SettingWidget::Text,
            default_value: json!(""),
            sensitive: false,
        },
        SettingDescriptor {
            path: "extends_profile_id",
            label: "Parent profile",
            description: "Optional parent profile id used for inheritance.",
            scope: SettingScope::Profile,
            widget: SettingWidget::Text,
            default_value: serde_json::Value::Null,
            sensitive: false,
        },
        SettingDescriptor {
            path: "ai.providers",
            label: "AI providers",
            description: "Profile-scoped AI provider availability.",
            scope: SettingScope::Profile,
            widget: SettingWidget::InfoBox,
            default_value: json!({}),
            sensitive: false,
        },
        SettingDescriptor {
            path: "mcp.connectors",
            label: "MCP and connectors",
            description: "Profile-scoped connector availability.",
            scope: SettingScope::Profile,
            widget: SettingWidget::InfoBox,
            default_value: json!({}),
            sensitive: false,
        },
        SettingDescriptor {
            path: "security.capabilities",
            label: "Security capabilities",
            description: "High-level profile controls that generate policy rules.",
            scope: SettingScope::Profile,
            widget: SettingWidget::InfoBox,
            default_value: json!({}),
            sensitive: false,
        },
        SettingDescriptor {
            path: "packages",
            label: "Package contract",
            description: "Guest package and runtime versions required by this profile.",
            scope: SettingScope::Profile,
            widget: SettingWidget::InfoBox,
            default_value: json!({}),
            sensitive: false,
        },
        SettingDescriptor {
            path: "tools",
            label: "Tool contract",
            description: "Guest, host, or profile-provided tools required by this profile.",
            scope: SettingScope::Profile,
            widget: SettingWidget::InfoBox,
            default_value: json!({}),
            sensitive: false,
        },
        SettingDescriptor {
            path: "vm.assets",
            label: "VM assets",
            description:
                "Per-architecture kernel, initrd, and rootfs assets required by this profile.",
            scope: SettingScope::Profile,
            widget: SettingWidget::InfoBox,
            default_value: json!({}),
            sensitive: false,
        },
        SettingDescriptor {
            path: "security.rules",
            label: "Rules",
            description: "Advanced policy rule tables by type.",
            scope: SettingScope::Profile,
            widget: SettingWidget::RuleBuilder,
            default_value: json!({}),
            sensitive: false,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileRecord {
    pub profile: Profile,
    pub source: ProfileSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub locked: bool,
}

impl ProfileRecord {
    fn new(profile: Profile, source: ProfileSource, path: Option<PathBuf>) -> Self {
        let locked = !matches!(source, ProfileSource::User);
        Self {
            profile,
            source,
            path,
            locked,
        }
    }

    fn location(&self) -> String {
        self.path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| format!("{:?}", self.source))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSource {
    BuiltIn,
    Base,
    Corp,
    User,
}

impl ProfileSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuiltIn => "built-in",
            Self::Base => "base",
            Self::Corp => "corp",
            Self::User => "user",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ProfileCatalog {
    pub profiles: BTreeMap<String, ProfileRecord>,
}

impl ProfileCatalog {
    pub fn list(&self) -> impl Iterator<Item = &ProfileRecord> {
        self.profiles.values()
    }

    pub fn get(&self, id: &str) -> Option<&ProfileRecord> {
        self.profiles.get(id)
    }

    fn insert(&mut self, record: ProfileRecord) -> Result<()> {
        let id = record.profile.id.clone();
        if let Some(existing) = self.profiles.get(&id) {
            if existing.source == ProfileSource::BuiltIn && record.source != ProfileSource::BuiltIn
            {
                self.profiles.insert(id, record);
                return Ok(());
            }
            return Err(SettingsProfilesError::DuplicateProfile {
                id,
                first: existing.location(),
                second: record.location(),
            });
        }
        self.profiles.insert(id, record);
        Ok(())
    }
}

pub fn discover_profiles(roots: &ProfileRootSettings) -> Result<ProfileCatalog> {
    roots.validate("profiles")?;
    let mut catalog = ProfileCatalog::default();
    catalog.insert(ProfileRecord::new(
        Profile::everyday_work(),
        ProfileSource::BuiltIn,
        None,
    ))?;
    discover_profile_dirs(&mut catalog, &roots.base_dirs, ProfileSource::Base)?;
    discover_profile_dirs(&mut catalog, &roots.corp_dirs, ProfileSource::Corp)?;
    discover_profile_dirs(&mut catalog, &roots.user_dirs, ProfileSource::User)?;
    validate_parent_chain(&catalog)?;
    validate_corp_priority_scope(&catalog)?;
    Ok(catalog)
}

/// Reject rules with priorities in `RULE_CORP_PRIORITY_RANGE`
/// when the owning profile is NOT
/// [`ProfileSource::Corp`]. Profile-level shape validation in
/// `ProfileRule::validate` enforces the absolute bounds and the
/// catch-all reservation; this pass adds the source-aware
/// restriction that requires the full catalog (source) to be
/// known.
fn validate_corp_priority_scope(catalog: &ProfileCatalog) -> Result<()> {
    for record in catalog.list() {
        if matches!(record.source, ProfileSource::Corp) {
            continue;
        }
        let rules = &record.profile.security.rules;
        for (rule_type, map) in [
            ("mcp", &rules.mcp),
            ("http", &rules.http),
            ("dns", &rules.dns),
            ("model", &rules.model),
            ("hook", &rules.hook),
        ] {
            for (name, rule) in map {
                if RULE_CORP_PRIORITY_RANGE.contains(&rule.priority) {
                    validation_error(
                        &format!(
                            "profiles.{}.security.rules.{rule_type}.{name}.priority",
                            record.profile.id
                        ),
                        &format!(
                            "priority {value} is corp-exclusive (range [{min}, {max}]); profile source is '{source}'",
                            value = rule.priority,
                            min = *RULE_CORP_PRIORITY_RANGE.start(),
                            max = *RULE_CORP_PRIORITY_RANGE.end(),
                            source = record.source.as_str(),
                        ),
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// Validate the inheritance graph across the catalog. Rejects
/// `extends_profile_id` references to unknown profiles, cycles
/// of any length, and chains deeper than
/// [`MAX_PROFILE_INHERITANCE_DEPTH`]. Profiles without a parent
/// trivially pass.
pub fn validate_parent_chain(catalog: &ProfileCatalog) -> Result<()> {
    for record in catalog.list() {
        walk_parent_chain(catalog, &record.profile.id)?;
    }
    Ok(())
}

/// Return the resolved ancestor chain in root-to-leaf order
/// (oldest ancestor first; selected profile last). Validates the
/// chain shape on the fly, so callers do not need to invoke
/// [`validate_parent_chain`] separately if they only care about
/// a single profile.
pub fn resolve_ancestor_chain<'a>(
    catalog: &'a ProfileCatalog,
    profile_id: &str,
) -> Result<Vec<&'a ProfileRecord>> {
    let chain = walk_parent_chain(catalog, profile_id)?;
    let mut records = Vec::with_capacity(chain.len());
    for id in chain.iter().rev() {
        let record = catalog
            .get(id)
            .ok_or_else(|| SettingsProfilesError::ProfileNotFound { id: id.clone() })?;
        records.push(record);
    }
    Ok(records)
}

/// Walk the inheritance chain starting at `profile_id`, returning
/// the visited profile ids in leaf-to-root order. Errors on
/// missing parent, cycle, or depth overflow. The depth budget is
/// counted in *edges*, so a chain of length `N` (a leaf with `N`
/// ancestors) has `N` extends_profile_id transitions and must
/// satisfy `N <= MAX_PROFILE_INHERITANCE_DEPTH`.
fn walk_parent_chain(catalog: &ProfileCatalog, profile_id: &str) -> Result<Vec<String>> {
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut chain: Vec<String> = Vec::new();
    let mut current = profile_id.to_string();
    let mut is_leaf = true;
    loop {
        if !visited.insert(current.clone()) {
            chain.push(current);
            return Err(SettingsProfilesError::InheritanceCycle {
                chain: chain.join(" -> "),
            });
        }
        chain.push(current.clone());
        let record = catalog.get(&current).ok_or_else(|| {
            if is_leaf {
                SettingsProfilesError::ProfileNotFound {
                    id: current.clone(),
                }
            } else {
                SettingsProfilesError::UnknownParentProfile {
                    id: profile_id.to_string(),
                    parent: current.clone(),
                }
            }
        })?;
        let Some(parent) = record.profile.extends_profile_id.clone() else {
            return Ok(chain);
        };
        if chain.len() > MAX_PROFILE_INHERITANCE_DEPTH {
            chain.push(parent);
            return Err(SettingsProfilesError::InheritanceDepthExceeded {
                id: profile_id.to_string(),
                max: MAX_PROFILE_INHERITANCE_DEPTH,
                chain: chain.join(" -> "),
            });
        }
        current = parent;
        is_leaf = false;
    }
}

pub fn create_user_profile(roots: &ProfileRootSettings, profile: Profile) -> Result<ProfileRecord> {
    write_user_profile(roots, profile, WriteMode::Create)
}

pub fn update_user_profile(roots: &ProfileRootSettings, profile: Profile) -> Result<ProfileRecord> {
    write_user_profile(roots, profile, WriteMode::Update)
}

pub fn fork_user_profile(
    roots: &ProfileRootSettings,
    source_profile_id: &str,
    new_profile_id: &str,
    new_name: &str,
) -> Result<ProfileRecord> {
    if !roots.allow_user_fork {
        return Err(SettingsProfilesError::Forbidden {
            message: "user profile forking is disabled by service settings".to_string(),
        });
    }
    validate_profile_id("source_profile_id", source_profile_id)?;
    validate_profile_id("new_profile_id", new_profile_id)?;
    if new_name.trim().is_empty() {
        validation_error("new_name", "forked profile name cannot be empty")?;
    }

    let catalog = discover_profiles(roots)?;
    let source =
        catalog
            .get(source_profile_id)
            .ok_or_else(|| SettingsProfilesError::ProfileNotFound {
                id: source_profile_id.to_string(),
            })?;
    if let Some(existing) = catalog.get(new_profile_id) {
        return Err(SettingsProfilesError::DuplicateProfile {
            id: new_profile_id.to_string(),
            first: existing.location(),
            second: profile_file_path(roots, new_profile_id)?
                .display()
                .to_string(),
        });
    }

    let mut forked = source.profile.clone();
    forked.id = new_profile_id.to_string();
    forked.name = new_name.trim().to_string();
    create_user_profile(roots, forked)
}

pub fn delete_user_profile(roots: &ProfileRootSettings, profile_id: &str) -> Result<()> {
    if !roots.allow_user_delete {
        return Err(SettingsProfilesError::Forbidden {
            message: "user profile deletion is disabled by service settings".to_string(),
        });
    }
    validate_profile_id("profile_id", profile_id)?;
    for dir in &roots.user_dirs {
        let path = dir.join(format!("{profile_id}.toml"));
        if path.exists() {
            fs::remove_file(&path).map_err(|source| SettingsProfilesError::WriteFile {
                path: path.clone(),
                details: source.to_string(),
            })?;
            return Ok(());
        }
    }
    Err(SettingsProfilesError::ProfileNotFound {
        id: profile_id.to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectiveVmSettings {
    pub profile_id: String,
    pub profile_name: String,
    pub profile_type: ProfileType,
    pub profile: ProvenancedProfileIdentity,
    pub ai: EffectiveSection<AiProvidersProfileSettings>,
    pub mcp: EffectiveSection<McpConnectorsProfileSettings>,
    pub skills: EffectiveSection<SkillsProfileSettings>,
    pub packages: EffectiveSection<ProfilePackageContract>,
    pub tools: EffectiveSection<BTreeMap<String, ProfileToolContract>>,
    pub vm: EffectiveSection<VmProfileSettings>,
    pub security: EffectiveSection<SecurityProfileSettings>,
    pub rules: Vec<EffectiveRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvenancedProfileIdentity {
    pub name: String,
    pub description: String,
    pub best_for: String,
    pub icon_svg: String,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectiveSection<T> {
    pub value: T,
    pub provenance: Provenance,
    /// Profile ids whose layered output contributed to this
    /// section, listed root-to-leaf and excluding the leaf
    /// itself. Empty when the section was materialized from a
    /// single profile with no ancestors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inherited_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectiveRule {
    pub id: String,
    #[serde(rename = "on")]
    pub callback: String,
    #[serde(rename = "if")]
    pub condition: String,
    pub decision: RuleDecision,
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strip_request_headers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strip_response_headers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub derived: bool,
    pub provenance: Provenance,
    /// Dotted path of the owning setting when the rule was
    /// generated from a non-rule setting (e.g.
    /// `ai.providers.openai.enabled`,
    /// `mcp.connectors.github.allowed_tools`,
    /// `security.capabilities.network_egress`). `None` for
    /// hand-authored rules whose home IS a rule block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_setting_path: Option<String>,
    /// Human-readable label for the owning setting, used by
    /// status / debug surfaces and the UI "managed by …"
    /// affordance. Pairs with `owner_setting_path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_setting_label: Option<String>,
    /// `false` for rules generated from a non-rule setting --
    /// the rule mutation gate refuses direct edits and points
    /// callers at `owner_setting_path`. Defaults to `true` so
    /// hand-authored rules are editable.
    #[serde(default = "default_rule_editable")]
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub profile_id: String,
    pub source: ProfileSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub toml_path: String,
    pub locked: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SettingsProfilesDebugSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceSettingsDebugSummary>,
    #[serde(default)]
    pub profiles: Vec<ProfileDebugSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective: Option<EffectiveVmSettingsDebugSummary>,
    /// Compact summary of the resolver trace for the active
    /// session, when available. Surfaces "why does the final
    /// state look like this?" data to status/debug consumers
    /// without dragging the full event log into every report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolver_trace: Option<ResolverTraceSummary>,
}

impl SettingsProfilesDebugSnapshot {
    pub fn from_parts(
        settings: &ServiceSettings,
        catalog: &ProfileCatalog,
        effective: Option<&EffectiveVmSettings>,
    ) -> Self {
        Self::from_parts_with_trace(settings, catalog, effective, None)
    }

    pub fn from_parts_with_trace(
        settings: &ServiceSettings,
        catalog: &ProfileCatalog,
        effective: Option<&EffectiveVmSettings>,
        trace: Option<&ResolverTrace>,
    ) -> Self {
        Self {
            load_error: None,
            service: Some(ServiceSettingsDebugSummary::from_settings(settings)),
            profiles: catalog
                .list()
                .map(ProfileDebugSummary::from_record)
                .collect(),
            selected_profile_id: effective.map(|effective| effective.profile_id.clone()),
            effective: effective.map(EffectiveVmSettingsDebugSummary::from_effective),
            resolver_trace: trace.map(|trace| trace.summary(DEFAULT_TRACE_SUMMARY_TAIL)),
        }
    }

    pub fn from_error(error: impl Into<String>) -> Self {
        Self {
            load_error: Some(error.into()),
            service: None,
            profiles: Vec::new(),
            selected_profile_id: None,
            effective: None,
            resolver_trace: None,
        }
    }
}

/// Number of trailing trace events surfaced in a debug
/// snapshot. Large enough to capture the typical
/// schema-default + ancestor + a handful of corp directives +
/// final rule events without bloating the report.
pub const DEFAULT_TRACE_SUMMARY_TAIL: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceSettingsDebugSummary {
    pub default_profile: String,
    pub base_dirs: Vec<String>,
    pub corp_dirs: Vec<String>,
    pub user_dirs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets_dir: Option<String>,
    pub image_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_download_base_url: Option<String>,
    pub allow_user_profiles: bool,
    pub allow_user_fork: bool,
    pub allow_user_delete: bool,
    pub telemetry_enabled: bool,
    pub telemetry_endpoint_configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_endpoint: Option<String>,
    pub remote_policy_enabled: bool,
    pub remote_policy_endpoint_configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_policy_endpoint: Option<String>,
    pub credential_ids: Vec<String>,
}

impl ServiceSettingsDebugSummary {
    fn from_settings(settings: &ServiceSettings) -> Self {
        Self {
            default_profile: settings.profiles.default_profile.clone(),
            base_dirs: paths_to_strings(&settings.profiles.base_dirs),
            corp_dirs: paths_to_strings(&settings.profiles.corp_dirs),
            user_dirs: paths_to_strings(&settings.profiles.user_dirs),
            assets_dir: settings
                .assets
                .assets_dir
                .as_ref()
                .map(|path| path.display().to_string()),
            image_roots: paths_to_strings(&settings.assets.image_roots),
            asset_download_base_url: settings.assets.download_base_url.clone(),
            allow_user_profiles: settings.profiles.allow_user_profiles,
            allow_user_fork: settings.profiles.allow_user_fork,
            allow_user_delete: settings.profiles.allow_user_delete,
            telemetry_enabled: settings.telemetry.enabled,
            telemetry_endpoint_configured: settings.telemetry.endpoint.is_some(),
            telemetry_endpoint: settings.telemetry.endpoint.clone(),
            remote_policy_enabled: settings.remote_policy.enabled,
            remote_policy_endpoint_configured: settings.remote_policy.endpoint.is_some(),
            remote_policy_endpoint: settings.remote_policy.endpoint.clone(),
            credential_ids: settings.credentials.items.keys().cloned().collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileDebugSummary {
    pub id: String,
    pub name: String,
    pub profile_type: ProfileType,
    pub best_for: String,
    pub source: ProfileSource,
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl ProfileDebugSummary {
    fn from_record(record: &ProfileRecord) -> Self {
        Self {
            id: record.profile.id.clone(),
            name: record.profile.name.clone(),
            profile_type: record.profile.profile_type,
            best_for: record.profile.best_for.clone(),
            source: record.source,
            locked: record.locked,
            path: record.path.as_ref().map(|path| path.display().to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectiveVmSettingsDebugSummary {
    pub profile_id: String,
    pub profile_name: String,
    pub vm_memory_mib: u32,
    pub vm_cpus: u8,
    pub vm_network: VmNetworkMode,
    pub mcp_connector_ids: Vec<String>,
    pub enabled_mcp_connector_ids: Vec<String>,
    pub skill_groups: Vec<String>,
    pub enabled_skills: Vec<String>,
    pub disabled_skills: Vec<String>,
    pub rule_count: usize,
    pub derived_rule_count: usize,
    pub raw_rule_count: usize,
}

impl EffectiveVmSettingsDebugSummary {
    fn from_effective(effective: &EffectiveVmSettings) -> Self {
        let mcp_connector_ids = effective
            .mcp
            .value
            .connectors
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let enabled_mcp_connector_ids = effective
            .mcp
            .value
            .connectors
            .iter()
            .filter(|(_, connector)| connector.enabled)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let derived_rule_count = effective.rules.iter().filter(|rule| rule.derived).count();
        Self {
            profile_id: effective.profile_id.clone(),
            profile_name: effective.profile_name.clone(),
            vm_memory_mib: effective.vm.value.memory_mib,
            vm_cpus: effective.vm.value.cpus,
            vm_network: effective.vm.value.network,
            mcp_connector_ids,
            enabled_mcp_connector_ids,
            skill_groups: effective.skills.value.groups.clone(),
            enabled_skills: effective.skills.value.enabled.clone(),
            disabled_skills: effective.skills.value.disabled.clone(),
            rule_count: effective.rules.len(),
            derived_rule_count,
            raw_rule_count: effective.rules.len() - derived_rule_count,
        }
    }
}

pub fn resolve_effective_vm_settings(
    roots: &ProfileRootSettings,
    profile_id: Option<&str>,
) -> Result<EffectiveVmSettings> {
    resolve_effective_vm_settings_with_trace(roots, profile_id).map(|(effective, _trace)| effective)
}

/// Resolve effective VM settings *and* the resolver trace
/// artifact in a single pass. Callers persisting the trace
/// should prefer this over [`resolve_effective_vm_settings`]
/// + a second resolver pass.
pub fn resolve_effective_vm_settings_with_trace(
    roots: &ProfileRootSettings,
    profile_id: Option<&str>,
) -> Result<(EffectiveVmSettings, ResolverTrace)> {
    let selected_id = profile_id.unwrap_or(&roots.default_profile);
    validate_profile_id("profile_id", selected_id)?;
    let catalog = discover_profiles(roots)?;
    let chain = resolve_ancestor_chain(&catalog, selected_id)?;
    let merged = merge_profile_chain(&chain);
    let mut trace = emit_baseline_trace(&chain);
    let effective = effective_settings_from_merged(&chain, &merged, &CorpOverrides::default());
    emit_rule_events(&mut trace, &effective);
    Ok((effective, trace))
}

/// Same as [`resolve_effective_vm_settings_with_trace`], but
/// applies the corp directives from [`ServiceSettings::corp_directives`]
/// after the profile chain is merged and before the trace's
/// rule events are emitted. Corp-touched paths attribute to
/// `source_kind = corp` in the trace and to a synthetic `corp`
/// provenance on per-rule output.
pub fn resolve_effective_vm_settings_with_corp(
    settings: &ServiceSettings,
    profile_id: Option<&str>,
) -> Result<(EffectiveVmSettings, ResolverTrace)> {
    let selected_id = profile_id.unwrap_or(&settings.profiles.default_profile);
    validate_profile_id("profile_id", selected_id)?;
    let catalog = discover_profiles(&settings.profiles)?;
    let chain = resolve_ancestor_chain(&catalog, selected_id)?;
    let mut merged = merge_profile_chain(&chain);
    let mut trace = emit_baseline_trace(&chain);
    let overrides = apply_corp_directives(&mut merged, &settings.corp_directives, &mut trace)?;
    let effective = effective_settings_from_merged(&chain, &merged, &overrides);
    emit_rule_events(&mut trace, &effective);
    Ok((effective, trace))
}

pub fn vm_effective_settings_path(session_dir: impl AsRef<Path>) -> PathBuf {
    session_dir.as_ref().join(VM_EFFECTIVE_SETTINGS_FILENAME)
}

pub fn load_vm_effective_settings(session_dir: impl AsRef<Path>) -> Result<EffectiveVmSettings> {
    let path = vm_effective_settings_path(session_dir);
    let input = fs::read_to_string(&path).map_err(|source| SettingsProfilesError::ReadFile {
        path: path.clone(),
        details: source.to_string(),
    })?;
    toml::from_str::<EffectiveVmSettings>(&input).map_err(|source| SettingsProfilesError::Parse {
        kind: "vm-effective settings",
        details: source.to_string(),
    })
}

pub fn write_vm_effective_settings(
    session_dir: impl AsRef<Path>,
    effective: &EffectiveVmSettings,
) -> Result<PathBuf> {
    let path = vm_effective_settings_path(session_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsProfilesError::WriteFile {
            path: parent.to_path_buf(),
            details: source.to_string(),
        })?;
    }
    let payload =
        toml::to_string_pretty(effective).map_err(|source| SettingsProfilesError::Serialize {
            kind: "vm-effective settings",
            details: source.to_string(),
        })?;
    fs::write(&path, payload).map_err(|source| SettingsProfilesError::WriteFile {
        path: path.clone(),
        details: source.to_string(),
    })?;
    Ok(path)
}

/// Materialize effective settings from an ancestor chain
/// (root-to-leaf order, as produced by [`resolve_ancestor_chain`]).
///
/// Merge contract:
/// - Map-shaped sections (`ai.providers`, `mcp.connectors`,
///   `security.rules.*`) merge by key, with later layers
///   overriding earlier layers per key.
/// - Skill string lists (`skills.groups`, `enabled`, `disabled`)
///   are unioned with dedup, preserving leaf ordering when keys
///   collide.
/// - Scalar-shaped sections (`general`, `appearance`, `vm`,
///   `security.capabilities`) take the leaf value entirely;
///   parents do not "fill in" individual scalar fields, because
///   the on-disk schema cannot distinguish "explicitly set to
///   default" from "unset" without breaking `serde(default)`.
/// - Per-rule provenance points at the actual contributing
///   profile (leaf if the leaf re-declared the rule, otherwise
///   the originating ancestor).
fn effective_settings_from_merged(
    chain: &[&ProfileRecord],
    merged_profile: &Profile,
    overrides: &CorpOverrides,
) -> EffectiveVmSettings {
    let leaf = *chain
        .last()
        .expect("ancestor chain must contain at least the selected profile");
    let ancestor_ids: Vec<String> = chain
        .iter()
        .take(chain.len().saturating_sub(1))
        .map(|record| record.profile.id.clone())
        .collect();
    let inherited = !ancestor_ids.is_empty();
    let section_reason = |base: &str| -> String {
        if inherited {
            format!("{base} (layered from ancestor chain)")
        } else {
            base.to_string()
        }
    };

    EffectiveVmSettings {
        profile_id: leaf.profile.id.clone(),
        profile_name: leaf.profile.name.clone(),
        profile_type: leaf.profile.profile_type,
        profile: ProvenancedProfileIdentity {
            name: leaf.profile.name.clone(),
            description: leaf.profile.description.clone(),
            best_for: leaf.profile.best_for.clone(),
            icon_svg: leaf.profile.icon_svg_or_default().to_string(),
            provenance: provenance(leaf, "profile", "selected profile identity"),
        },
        ai: EffectiveSection {
            value: merged_profile.ai.clone(),
            provenance: provenance(
                leaf,
                "ai",
                &section_reason("profile-scoped AI provider settings"),
            ),
            inherited_from: ancestor_ids.clone(),
        },
        mcp: EffectiveSection {
            value: merged_profile.mcp.clone(),
            provenance: provenance(
                leaf,
                "mcp",
                &section_reason("profile-scoped MCP and connector settings"),
            ),
            inherited_from: ancestor_ids.clone(),
        },
        skills: EffectiveSection {
            value: merged_profile.skills.clone(),
            provenance: provenance(
                leaf,
                "skills",
                &section_reason("profile-scoped skill settings"),
            ),
            inherited_from: ancestor_ids.clone(),
        },
        packages: EffectiveSection {
            value: merged_profile.packages.clone(),
            provenance: provenance(
                leaf,
                "packages",
                &section_reason("profile package contract"),
            ),
            inherited_from: ancestor_ids.clone(),
        },
        tools: EffectiveSection {
            value: merged_profile.tools.clone(),
            provenance: provenance(leaf, "tools", &section_reason("profile tool contract")),
            inherited_from: ancestor_ids.clone(),
        },
        vm: EffectiveSection {
            value: merged_profile.vm.clone(),
            provenance: provenance(leaf, "vm", &section_reason("profile-scoped VM settings")),
            inherited_from: ancestor_ids.clone(),
        },
        security: EffectiveSection {
            value: merged_profile.security.clone(),
            provenance: provenance(
                leaf,
                "security",
                &section_reason("profile-scoped security settings"),
            ),
            inherited_from: ancestor_ids.clone(),
        },
        rules: effective_rules_from_chain_and_overrides(chain, merged_profile, overrides),
    }
}

/// Fold the ancestor chain into a single merged `Profile`. The
/// returned value's identity fields (id, name, etc.) reflect the
/// leaf; only its substantive sections are layered.
fn merge_profile_chain(chain: &[&ProfileRecord]) -> Profile {
    let mut acc = chain[0].profile.clone();
    for record in &chain[1..] {
        let child = &record.profile;

        acc.version = child.version;
        acc.id = child.id.clone();
        acc.name = child.name.clone();
        acc.description = child.description.clone();
        acc.best_for = child.best_for.clone();
        acc.profile_type = child.profile_type;
        acc.icon_svg = child.icon_svg.clone();
        acc.extends_profile_id = child.extends_profile_id.clone();

        acc.general = child.general.clone();
        acc.appearance = child.appearance.clone();
        acc.vm = merge_vm_profile_settings(&acc.vm, &child.vm);
        acc.security.capabilities = child.security.capabilities.clone();

        merge_btreemap(&mut acc.ai.providers, &child.ai.providers);
        merge_btreemap(&mut acc.mcp.connectors, &child.mcp.connectors);
        merge_package_contract(&mut acc.packages, &child.packages);
        merge_btreemap(&mut acc.tools, &child.tools);
        merge_btreemap(&mut acc.security.rules.mcp, &child.security.rules.mcp);
        merge_btreemap(&mut acc.security.rules.http, &child.security.rules.http);
        merge_btreemap(&mut acc.security.rules.dns, &child.security.rules.dns);
        merge_btreemap(&mut acc.security.rules.model, &child.security.rules.model);
        merge_btreemap(&mut acc.security.rules.hook, &child.security.rules.hook);

        merge_str_list_dedup(&mut acc.skills.groups, &child.skills.groups);
        merge_str_list_dedup(&mut acc.skills.enabled, &child.skills.enabled);
        merge_str_list_dedup(&mut acc.skills.disabled, &child.skills.disabled);
    }
    acc
}

/// Emit the resolver trace's baseline events:
///
/// 1. A `default`/`set` event at path `*` (schema defaults).
/// 2. One `profile`/`set` event per ancestor and the leaf,
///    root-to-leaf, at path `profiles.<id>`.
///
/// Corp directive events (slice 6.4) and rule events
/// (`emit_rule_events`) append after this baseline.
fn emit_baseline_trace(chain: &[&ProfileRecord]) -> ResolverTrace {
    let mut trace = ResolverTrace::new();
    trace.append(ResolverTraceEvent {
        step: 0,
        path: "*".to_string(),
        operation: ResolverTraceOperation::Set,
        source_kind: ResolverTraceSourceKind::Default,
        source_profile_id: None,
        source_label: "schema defaults".to_string(),
        before: None,
        after: None,
        locked: false,
        reason: Some("baseline before ancestor chain".to_string()),
    });
    for record in chain {
        trace.append(ResolverTraceEvent {
            step: 0,
            path: format!("profiles.{}", record.profile.id),
            operation: ResolverTraceOperation::Set,
            source_kind: ResolverTraceSourceKind::Profile,
            source_profile_id: Some(record.profile.id.clone()),
            source_label: format!("{} profile applied", record.source.as_str()),
            before: None,
            after: None,
            locked: record.locked,
            reason: None,
        });
    }
    trace
}

/// Append one event per declared effective rule (and `derive`
/// events for derived capability rules) to a trace whose
/// baseline + any corp directive events have already been
/// emitted. Per-rule attribution comes from
/// [`EffectiveRule::provenance`], so a corp-touched rule lands
/// with `source_kind = corp` automatically.
fn emit_rule_events(trace: &mut ResolverTrace, effective: &EffectiveVmSettings) {
    for rule in &effective.rules {
        let (operation, source_kind) = if rule.derived {
            (
                ResolverTraceOperation::Derive,
                ResolverTraceSourceKind::Derived,
            )
        } else if matches!(rule.provenance.source, ProfileSource::Corp)
            && rule.provenance.profile_id == "corp"
        {
            (ResolverTraceOperation::Set, ResolverTraceSourceKind::Corp)
        } else {
            (
                ResolverTraceOperation::Set,
                ResolverTraceSourceKind::Profile,
            )
        };
        trace.append(ResolverTraceEvent {
            step: 0,
            path: format!("security.rules.{}", rule.id),
            operation,
            source_kind,
            source_profile_id: Some(rule.provenance.profile_id.clone()),
            source_label: rule.provenance.reason.clone(),
            before: None,
            after: serde_json::to_value(rule).ok(),
            locked: rule.provenance.locked,
            reason: rule.reason.clone(),
        });
    }
}

fn merge_btreemap<V: Clone>(acc: &mut BTreeMap<String, V>, child: &BTreeMap<String, V>) {
    for (key, value) in child {
        acc.insert(key.clone(), value.clone());
    }
}

fn merge_package_contract(acc: &mut ProfilePackageContract, child: &ProfilePackageContract) {
    merge_btreemap(&mut acc.runtimes, &child.runtimes);
    merge_btreemap(&mut acc.python_modules, &child.python_modules);
    merge_btreemap(&mut acc.node_packages, &child.node_packages);
    if !child.system.distro.is_empty() {
        acc.system.distro = child.system.distro.clone();
    }
    if !child.system.release.is_empty() {
        acc.system.release = child.system.release.clone();
    }
    merge_btreemap(&mut acc.system.apt, &child.system.apt);
}

fn merge_vm_profile_settings(
    parent: &VmProfileSettings,
    child: &VmProfileSettings,
) -> VmProfileSettings {
    let mut merged = child.clone();
    let mut assets = parent.assets.clone();
    merge_btreemap(&mut assets, &child.assets);
    merged.assets = assets;
    merged
}

/// Union with dedup. Child entries override their previous
/// position so the leaf's intent ("I want X near the end")
/// survives, but no string appears twice.
fn merge_str_list_dedup(acc: &mut Vec<String>, child: &[String]) {
    for item in child {
        if let Some(idx) = acc.iter().position(|existing| existing == item) {
            acc.remove(idx);
        }
        acc.push(item.clone());
    }
}

fn effective_rules_from_chain_and_overrides(
    chain: &[&ProfileRecord],
    merged_profile: &Profile,
    overrides: &CorpOverrides,
) -> Vec<EffectiveRule> {
    let leaf = *chain
        .last()
        .expect("ancestor chain must contain at least the selected profile");
    let mut rules = derived_catch_all_rules(leaf);
    for (rule_type, rule_map) in [
        ("mcp", &merged_profile.security.rules.mcp),
        ("http", &merged_profile.security.rules.http),
        ("dns", &merged_profile.security.rules.dns),
        ("model", &merged_profile.security.rules.model),
        ("hook", &merged_profile.security.rules.hook),
    ] {
        let contributors = rule_contributors_for_type(chain, rule_type);
        for (name, rule) in rule_map {
            // Prefer the corp-attributed provenance when corp
            // touched this name for this type; otherwise fall
            // back to the originating chain record.
            let corp_touched = overrides
                .rules
                .get(name)
                .map(|owning_type| owning_type == rule_type)
                .unwrap_or(false);
            if corp_touched {
                rules.push(effective_rule_with_corp_provenance(rule_type, name, rule));
            } else if let Some((record, _)) = contributors.get(name) {
                rules.push(effective_rule_from(record, rule_type, name, rule));
            } else {
                // Should be unreachable: a rule is in the merged
                // profile but neither corp nor chain attributed
                // it. Fall back to leaf attribution so callers
                // still see a provenance entry rather than
                // silently dropping the rule.
                rules.push(effective_rule_from(leaf, rule_type, name, rule));
            }
        }
    }

    // Nested rules: collect rules authored under setting hosts
    // (AI providers, MCP connectors). They land in the same
    // effective rules list but carry `owner_setting_path`
    // pointing at the host's dotted path so callers know
    // "this rule was authored co-located with the openai
    // provider config". They remain editable -- ownership here
    // is about file structure, not about the mutation gate.
    rules.extend(collect_nested_rules_for_hosts(leaf, merged_profile));

    rules.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    rules
}

/// Walk every nestable rule host on the merged profile and
/// emit one [`EffectiveRule`] per nested rule. Provenance
/// points at the leaf record (the merged profile's identity);
/// `owner_setting_path` tags each rule with the host's dotted
/// path so the UI / debug surfaces can show "managed by
/// ai.providers.openai".
fn collect_nested_rules_for_hosts(
    leaf: &ProfileRecord,
    merged_profile: &Profile,
) -> Vec<EffectiveRule> {
    let mut out = Vec::new();
    for (provider_id, provider) in &merged_profile.ai.providers {
        let host_path = format!("ai.providers.{provider_id}");
        let host_label = format!("AI provider · {provider_id}");
        push_nested_rules_from(&mut out, leaf, &provider.rules, &host_path, &host_label);
    }
    for (connector_id, connector) in &merged_profile.mcp.connectors {
        let host_path = format!("mcp.connectors.{connector_id}");
        let host_label = format!("MCP connector · {connector_id}");
        push_nested_rules_from(&mut out, leaf, &connector.rules, &host_path, &host_label);
    }
    out.extend(derived_provider_toggle_rules(leaf, merged_profile));
    out.extend(derived_mcp_allowed_tools_rules(leaf, merged_profile));
    out
}

/// Static mapping from a built-in AI provider id to the hosts
/// that need DNS/HTTP allow (or deny) when the provider is
/// enabled (or disabled). Pulled forward from the V1
/// `NetworkPolicy::default_dev()` metadata; unknown providers
/// fall back to deriving the host from the configured
/// `base_url`.
fn well_known_provider_hosts(provider_id: &str) -> &'static [&'static str] {
    match provider_id {
        "openai" => &["api.openai.com"],
        "anthropic" => &["api.anthropic.com"],
        "google" => &["generativelanguage.googleapis.com"],
        _ => &[],
    }
}

/// Slice 6b.6: derive priority-`0` rules from
/// `ai.providers.<name>.enabled` toggles. A `true` toggle emits
/// allow rules for the provider's hosts; `false` emits deny
/// rules. Each rule attributes ownership to
/// `ai.providers.<name>.enabled` so the mutation gate (slice
/// 6b.8) refuses direct edits and the UI surfaces "managed by
/// AI provider · openai".
fn derived_provider_toggle_rules(
    record: &ProfileRecord,
    merged_profile: &Profile,
) -> Vec<EffectiveRule> {
    let mut out = Vec::new();
    for (provider_id, provider) in &merged_profile.ai.providers {
        let hosts = well_known_provider_hosts(provider_id);
        // Provider not in the static map and no base_url -> no
        // derived rules. Authors that need an unknown provider
        // to drive policy can still nest rules under
        // `ai.providers.<name>.rules.*` (slice 6b.3).
        let base_host_owned = provider
            .base_url
            .as_deref()
            .and_then(extract_host_from_base_url);
        let base_host_slice: [&str; 1];
        let derived_hosts: &[&str] = if !hosts.is_empty() {
            hosts
        } else if let Some(base) = base_host_owned.as_deref() {
            base_host_slice = [base];
            &base_host_slice
        } else {
            continue;
        };

        let owner_path = format!("ai.providers.{provider_id}.enabled");
        let owner_label = format!("AI provider · {provider_id}");
        let decision = if provider.enabled {
            RuleDecision::Allow
        } else {
            RuleDecision::Block
        };
        let action_word = if provider.enabled { "allow" } else { "block" };

        for host in derived_hosts {
            for (rule_type, callback, condition) in [
                ("dns", "dns.request", format!("qname == '{host}'")),
                ("http", "http.request", format!("request.host == '{host}'")),
            ] {
                let safe_host = host.replace('.', "-").replace('*', "wild");
                let id = format!("{rule_type}.provider_{provider_id}_{action_word}_{safe_host}");
                out.push(EffectiveRule {
                    id,
                    callback: callback.to_string(),
                    condition,
                    decision,
                    priority: 0,
                    rewrite_target: None,
                    rewrite_value: None,
                    strip_request_headers: Vec::new(),
                    strip_response_headers: Vec::new(),
                    reason: Some(format!(
                        "Derived from ai.providers.{provider_id}.enabled = {}",
                        provider.enabled
                    )),
                    derived: true,
                    provenance: provenance(record, &owner_path, "AI provider toggle catch"),
                    owner_setting_path: Some(owner_path.clone()),
                    owner_setting_label: Some(owner_label.clone()),
                    editable: false,
                });
            }
        }
    }
    out
}

/// Best-effort hostname extraction from a configured
/// `base_url`. Failures (relative URLs, malformed scheme, etc.)
/// return None and the caller skips the provider.
fn extract_host_from_base_url(base_url: &str) -> Option<String> {
    let after_scheme = base_url.split("://").nth(1)?;
    let host = after_scheme.split('/').next()?.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Slice 6b.7 placeholder -- implemented in the same hunk so
/// the resolver can find the symbol; the body lands as part of
/// slice 6b.7's commit.
fn derived_mcp_allowed_tools_rules(
    record: &ProfileRecord,
    merged_profile: &Profile,
) -> Vec<EffectiveRule> {
    let mut out = Vec::new();
    for (connector_id, connector) in &merged_profile.mcp.connectors {
        if connector.allowed_tools.is_empty() {
            continue;
        }
        let owner_path = format!("mcp.connectors.{connector_id}.allowed_tools");
        let owner_label = format!("MCP connector · {connector_id}");
        for tool in &connector.allowed_tools {
            let safe_tool = tool.replace('.', "-");
            out.push(EffectiveRule {
                id: format!("mcp.connector_{connector_id}_allow_{safe_tool}"),
                callback: "mcp.request".to_string(),
                condition: format!("tool.name == '{tool}'"),
                decision: RuleDecision::Allow,
                priority: 0,
                rewrite_target: None,
                rewrite_value: None,
                strip_request_headers: Vec::new(),
                strip_response_headers: Vec::new(),
                reason: Some(format!(
                    "Derived from mcp.connectors.{connector_id}.allowed_tools"
                )),
                derived: true,
                provenance: provenance(record, &owner_path, "MCP connector allowed_tools"),
                owner_setting_path: Some(owner_path.clone()),
                owner_setting_label: Some(owner_label.clone()),
                editable: false,
            });
        }
    }
    out
}

fn push_nested_rules_from(
    out: &mut Vec<EffectiveRule>,
    leaf: &ProfileRecord,
    rules: &SecurityRules,
    host_path: &str,
    host_label: &str,
) {
    for (rule_type, rule_map) in [
        ("mcp", &rules.mcp),
        ("http", &rules.http),
        ("dns", &rules.dns),
        ("model", &rules.model),
        ("hook", &rules.hook),
    ] {
        for (name, rule) in rule_map {
            out.push(EffectiveRule {
                id: format!("{rule_type}.{name}"),
                callback: rule.callback.clone(),
                condition: rule.condition.clone(),
                decision: rule.decision,
                priority: rule.priority,
                rewrite_target: rule.rewrite_target.clone(),
                rewrite_value: rule.rewrite_value.clone(),
                strip_request_headers: rule.strip_request_headers.clone(),
                strip_response_headers: rule.strip_response_headers.clone(),
                reason: rule.reason.clone(),
                derived: false,
                provenance: provenance(
                    leaf,
                    &format!("{host_path}.rules.{rule_type}.{name}"),
                    "profile rule nested under setting host",
                ),
                owner_setting_path: Some(host_path.to_string()),
                owner_setting_label: Some(host_label.to_string()),
                editable: true,
            });
        }
    }
}

fn effective_rule_with_corp_provenance(
    rule_type: &str,
    name: &str,
    rule: &ProfileRule,
) -> EffectiveRule {
    EffectiveRule {
        id: format!("{rule_type}.{name}"),
        callback: rule.callback.clone(),
        condition: rule.condition.clone(),
        decision: rule.decision,
        priority: rule.priority,
        rewrite_target: rule.rewrite_target.clone(),
        rewrite_value: rule.rewrite_value.clone(),
        strip_request_headers: rule.strip_request_headers.clone(),
        strip_response_headers: rule.strip_response_headers.clone(),
        reason: rule.reason.clone(),
        derived: false,
        provenance: Provenance {
            profile_id: "corp".to_string(),
            source: ProfileSource::Corp,
            path: None,
            toml_path: format!("security.rules.{rule_type}.{name}"),
            locked: false,
            reason: "corp directive override".to_string(),
        },
        // Corp directive replacements are policy edits, not
        // setting-derived. They remain editable BY corp (via
        // another corp directive); only setting-derived rules
        // are flagged uneditable.
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    }
}

/// For a given rule type, walk the ancestor chain root-to-leaf
/// collecting the *last* record that declared each rule name.
/// The returned `ProfileRule` is cloned from the contributing
/// record so callers can build `EffectiveRule` directly.
/// Slice 6b.8: mutation gate enforced in core so UDS/CLI
/// surfaces (S07-S09) inherit consistent refusal behavior.
/// Returns `Ok(())` if the target rule is editable; otherwise
/// returns a typed [`SettingsProfilesError::RuleManagedBySetting`]
/// that names both the rule and the owning setting so callers
/// can render an actionable error.
///
/// Callers attempting to mutate a rule must consult the
/// effective rules list (since ownership lives on
/// [`EffectiveRule`], not on the raw profile `ProfileRule`).
/// For a future S07 mutation API the flow is:
///
/// 1. Resolve effective settings for the target profile.
/// 2. Look up the rule by id in `effective.rules`.
/// 3. Call [`ensure_rule_editable`].
/// 4. If `Ok`, perform the mutation against the on-disk
///    profile file.
pub fn ensure_rule_editable(rule: &EffectiveRule) -> Result<()> {
    if rule.editable {
        return Ok(());
    }
    let owner = rule
        .owner_setting_path
        .clone()
        .unwrap_or_else(|| "<unknown setting>".to_string());
    Err(SettingsProfilesError::RuleManagedBySetting {
        rule_id: rule.id.clone(),
        owner_setting_path: owner,
    })
}

fn rule_contributors_for_type<'a>(
    chain: &[&'a ProfileRecord],
    rule_type: &str,
) -> BTreeMap<String, (&'a ProfileRecord, ProfileRule)> {
    let mut map: BTreeMap<String, (&'a ProfileRecord, ProfileRule)> = BTreeMap::new();
    for record in chain {
        let rules = match rule_type {
            "mcp" => &record.profile.security.rules.mcp,
            "http" => &record.profile.security.rules.http,
            "dns" => &record.profile.security.rules.dns,
            "model" => &record.profile.security.rules.model,
            "hook" => &record.profile.security.rules.hook,
            _ => continue,
        };
        for (name, rule) in rules {
            map.insert(name.clone(), (*record, rule.clone()));
        }
    }
    map
}

fn effective_rule_from(
    record: &ProfileRecord,
    rule_type: &str,
    name: &str,
    rule: &ProfileRule,
) -> EffectiveRule {
    EffectiveRule {
        id: format!("{rule_type}.{name}"),
        callback: rule.callback.clone(),
        condition: rule.condition.clone(),
        decision: rule.decision,
        priority: rule.priority,
        rewrite_target: rule.rewrite_target.clone(),
        rewrite_value: rule.rewrite_value.clone(),
        strip_request_headers: rule.strip_request_headers.clone(),
        strip_response_headers: rule.strip_response_headers.clone(),
        reason: rule.reason.clone(),
        derived: false,
        provenance: provenance(
            record,
            &format!("security.rules.{rule_type}.{name}"),
            "profile rule",
        ),
        // Hand-authored profile rules have no owning non-rule
        // setting; they ARE the rule. Slice 6b.3 will populate
        // ownership for rules nested under setting hosts like
        // `ai.providers.<name>` or `mcp.connectors.<name>`.
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    }
}

/// Slice 6b.5: emit the per-rule-type catch-all rules at
/// priority [`RULE_CATCH_ALL_PRIORITY`] (`1000`). One catch-all
/// per real runtime callback, with `condition = "true"` so it
/// matches everything that nothing else above caught. Decisions
/// derive from the relevant `security.capabilities.*` setting:
/// `network_egress` drives DNS / HTTP / model defaults;
/// `mcp_tools` drives the MCP default. Ownership points at the
/// originating capability path so the mutation gate refuses
/// direct edits and the UI surfaces "managed by Security
/// capability · network_egress".
fn derived_catch_all_rules(record: &ProfileRecord) -> Vec<EffectiveRule> {
    let capabilities = &record.profile.security.capabilities;
    let net = capabilities.network_egress;
    let mcp = capabilities.mcp_tools;

    let mut out = Vec::new();
    for (id, callback, capability_path, mode) in [
        (
            "dns.default",
            "dns.request",
            "security.capabilities.network_egress",
            net,
        ),
        (
            "http.default_read",
            "http.read",
            "security.capabilities.network_egress",
            net,
        ),
        (
            "http.default_write",
            "http.write",
            "security.capabilities.network_egress",
            net,
        ),
        (
            "model.default",
            "model.request",
            "security.capabilities.network_egress",
            net,
        ),
        (
            "mcp.default",
            "mcp.request",
            "security.capabilities.mcp_tools",
            mcp,
        ),
    ] {
        out.push(EffectiveRule {
            id: id.to_string(),
            callback: callback.to_string(),
            condition: "true".to_string(),
            decision: mode.into(),
            priority: RULE_CATCH_ALL_PRIORITY,
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
            reason: Some(format!("Catch-all from {capability_path} = {mode:?}")),
            derived: true,
            provenance: provenance(
                record,
                capability_path,
                "catch-all rule derived from capability",
            ),
            owner_setting_path: Some(capability_path.to_string()),
            owner_setting_label: Some(format!("Capability default · {callback}")),
            editable: false,
        });
    }
    out
}

impl From<CapabilityMode> for RuleDecision {
    fn from(value: CapabilityMode) -> Self {
        match value {
            CapabilityMode::Allow | CapabilityMode::Audit => RuleDecision::Allow,
            CapabilityMode::Ask => RuleDecision::Ask,
            CapabilityMode::Block => RuleDecision::Block,
        }
    }
}

fn provenance(record: &ProfileRecord, toml_path: &str, reason: &str) -> Provenance {
    Provenance {
        profile_id: record.profile.id.clone(),
        source: record.source,
        path: record.path.clone(),
        toml_path: toml_path.to_string(),
        locked: record.locked,
        reason: reason.to_string(),
    }
}

fn paths_to_strings(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}

#[derive(Debug, Clone, Copy)]
enum WriteMode {
    Create,
    Update,
}

fn write_user_profile(
    roots: &ProfileRootSettings,
    profile: Profile,
    mode: WriteMode,
) -> Result<ProfileRecord> {
    if !roots.allow_user_profiles {
        return Err(SettingsProfilesError::Forbidden {
            message: "user profile creation is disabled by service settings".to_string(),
        });
    }
    profile.validate()?;
    let path = profile_file_path(roots, &profile.id)?;
    match mode {
        WriteMode::Create if path.exists() => {
            return Err(SettingsProfilesError::DuplicateProfile {
                id: profile.id.clone(),
                first: path.display().to_string(),
                second: path.display().to_string(),
            });
        }
        WriteMode::Update if !path.exists() => {
            return Err(SettingsProfilesError::ProfileNotFound {
                id: profile.id.clone(),
            });
        }
        _ => {}
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsProfilesError::WriteFile {
            path: parent.to_path_buf(),
            details: source.to_string(),
        })?;
    }
    let payload =
        toml::to_string_pretty(&profile).map_err(|source| SettingsProfilesError::Serialize {
            kind: "profile",
            details: source.to_string(),
        })?;
    fs::write(&path, payload).map_err(|source| SettingsProfilesError::WriteFile {
        path: path.clone(),
        details: source.to_string(),
    })?;
    Ok(ProfileRecord::new(profile, ProfileSource::User, Some(path)))
}

fn profile_file_path(roots: &ProfileRootSettings, profile_id: &str) -> Result<PathBuf> {
    validate_profile_id("profile_id", profile_id)?;
    let dir = roots
        .user_dirs
        .first()
        .ok_or_else(|| SettingsProfilesError::Forbidden {
            message: "no user profile directory is configured".to_string(),
        })?;
    Ok(dir.join(format!("{profile_id}.toml")))
}

fn discover_profile_dirs(
    catalog: &mut ProfileCatalog,
    dirs: &[PathBuf],
    source: ProfileSource,
) -> Result<()> {
    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        let mut entries = fs::read_dir(dir)
            .map_err(|error| SettingsProfilesError::ReadFile {
                path: dir.clone(),
                details: error.to_string(),
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| SettingsProfilesError::ReadFile {
                path: dir.clone(),
                details: error.to_string(),
            })?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            let profile = read_profile_file(&path)?;
            catalog.insert(ProfileRecord::new(profile, source, Some(path)))?;
        }
    }
    Ok(())
}

fn read_profile_file(path: &Path) -> Result<Profile> {
    let input = fs::read_to_string(path).map_err(|source| SettingsProfilesError::ReadFile {
        path: path.to_path_buf(),
        details: source.to_string(),
    })?;
    Profile::from_toml_str(&input)
}

fn schema_version() -> u32 {
    SETTINGS_SCHEMA_VERSION
}

fn default_true() -> bool {
    true
}

fn default_accent() -> String {
    "blue".to_string()
}

fn default_profile_id() -> String {
    EVERYDAY_WORK_PROFILE_ID.to_string()
}

fn default_base_profile_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from(
        "/Library/Application Support/Capsem/profiles/base",
    )]
}

fn default_user_profile_dirs() -> Vec<PathBuf> {
    vec![crate::paths::capsem_home().join("profiles")]
}

fn default_telemetry_batch_max_events() -> u16 {
    128
}

fn default_telemetry_flush_interval_ms() -> u64 {
    5_000
}

fn default_telemetry_retry_attempts() -> u8 {
    3
}

fn default_remote_policy_timeout_ms() -> u64 {
    1_500
}

fn default_memory_mib() -> u32 {
    4096
}

fn default_vcpu_count() -> u8 {
    4
}

fn default_ask() -> CapabilityMode {
    CapabilityMode::Ask
}

fn default_audit() -> CapabilityMode {
    CapabilityMode::Audit
}

fn default_rule_priority() -> i32 {
    1
}

fn default_rule_editable() -> bool {
    true
}

fn validate_schema_version(path: &str, version: u32) -> Result<()> {
    if version != SETTINGS_SCHEMA_VERSION {
        validation_error(
            path,
            &format!("expected schema version {SETTINGS_SCHEMA_VERSION}, got {version}"),
        )?;
    }
    Ok(())
}

fn validate_paths(path: &str, paths: &[PathBuf]) -> Result<()> {
    for (index, path_value) in paths.iter().enumerate() {
        validate_path(&format!("{path}[{index}]"), path_value)?;
    }
    Ok(())
}

fn validate_path(path: &str, path_value: &Path) -> Result<()> {
    if path_value.as_os_str().is_empty() {
        validation_error(path, "path cannot be empty")?;
    }
    Ok(())
}

fn validate_optional_endpoint(path: &str, enabled: bool, endpoint: Option<&str>) -> Result<()> {
    match (enabled, endpoint) {
        (true, Some(value)) => validate_endpoint(&format!("{path}.endpoint"), value),
        (true, None) => validation_error(
            &format!("{path}.endpoint"),
            "endpoint is required when enabled is true",
        ),
        (false, Some(value)) if value.trim().is_empty() => {
            validation_error(&format!("{path}.endpoint"), "endpoint cannot be empty")
        }
        _ => Ok(()),
    }
}

fn validate_endpoint(path: &str, endpoint: &str) -> Result<()> {
    let value = endpoint.trim();
    if value.is_empty() {
        validation_error(path, "endpoint cannot be empty")?;
    }
    if !value.starts_with("https://") && !value.starts_with("http://") {
        validation_error(path, "endpoint must start with http:// or https://")?;
    }
    Ok(())
}

fn validate_profile_id(path: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        validation_error(path, "profile id cannot be empty")?;
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        Ok(())
    } else {
        validation_error(
            path,
            "profile id may only contain lowercase letters, digits, and '-'",
        )
    }
}

fn validate_config_id(path: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        validation_error(path, "id cannot be empty")?;
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        validation_error(
            path,
            "id may only contain lowercase letters, digits, '-', '_', and '.'",
        )
    }
}

fn validate_arch_id(path: &str, value: &str) -> Result<()> {
    match value {
        "arm64" | "x86_64" => Ok(()),
        _ => validation_error(path, "arch must be 'arm64' or 'x86_64'"),
    }
}

fn validate_tool_contracts(
    path: &str,
    tools: &BTreeMap<String, ProfileToolContract>,
) -> Result<()> {
    for (name, tool) in tools {
        validate_config_id(path, name)?;
        tool.validate(&format!("{path}.{name}"))?;
    }
    Ok(())
}

fn validate_package_version_map(path: &str, values: &BTreeMap<String, String>) -> Result<()> {
    for (name, version) in values {
        validate_package_name(path, name)?;
        validate_required_non_empty_string(&format!("{path}.{name}"), version)?;
    }
    Ok(())
}

fn validate_package_name(path: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        validation_error(path, "package name cannot be empty")?;
    }
    if value.contains("..") || value.contains('\\') {
        validation_error(path, "package name cannot contain path traversal")?;
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '@' | '/' | '-' | '_' | '.' | '+'))
    {
        Ok(())
    } else {
        validation_error(
            path,
            "package name may only contain ASCII letters, digits, '@', '/', '-', '_', '.', and '+'",
        )
    }
}

fn validate_optional_non_empty_string(path: &str, value: &str) -> Result<()> {
    if value.is_empty() || !value.trim().is_empty() {
        Ok(())
    } else {
        validation_error(path, "value cannot be only whitespace")
    }
}

fn validate_required_non_empty_string(path: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        validation_error(path, "value cannot be empty")?;
    }
    Ok(())
}

fn validate_profile_asset_location(path: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        validation_error(path, "asset location cannot be empty")?;
    }
    if !value.starts_with("https://") && !value.starts_with("file://") {
        validation_error(path, "asset location must start with https:// or file://")?;
    }
    if value.contains("..") || value.contains('\\') {
        validation_error(path, "asset location cannot contain path traversal")?;
    }
    Ok(())
}

fn validate_profile_hash(path: &str, value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("blake3:") else {
        return validation_error(path, "hash must be canonical blake3:<64 lowercase hex>");
    };
    if hex.len() == 64
        && hex
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
    {
        Ok(())
    } else {
        validation_error(path, "hash must be canonical blake3:<64 lowercase hex>")
    }
}

fn validate_rule_name(path: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        validation_error(path, "rule name cannot be empty")?;
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        Ok(())
    } else {
        validation_error(
            path,
            "rule name may only contain ASCII letters, digits, '-', and '_'",
        )
    }
}

fn validate_rule_map(
    path: &str,
    rule_type: &str,
    rules: &BTreeMap<String, ProfileRule>,
) -> Result<()> {
    for (name, rule) in rules {
        validate_rule_name(&format!("{path}.{rule_type}"), name)?;
        let rule_path = format!("{path}.{rule_type}.{name}");
        rule.validate(&rule_path)?;
        validate_rule_callback_for_type(&rule_path, rule_type, &rule.callback)?;
    }
    Ok(())
}

fn validate_rule_callback_for_type(path: &str, rule_type: &str, callback: &str) -> Result<()> {
    let allowed: &[&str] = match rule_type {
        "mcp" => &["mcp.request", "mcp.response"],
        "http" => &["http.request", "http.read", "http.write", "http.response"],
        "dns" => &["dns.request", "dns.response"],
        "model" => &[
            "model.request",
            "model.response",
            "model.tool_call",
            "model.tool_response",
        ],
        "hook" => &["hook.decision"],
        _ => {
            validation_error(path, &format!("unsupported rule type '{rule_type}'"))?;
            return Ok(());
        }
    };

    if allowed.contains(&callback) {
        Ok(())
    } else if let Some(replacement) = renamed_callback(callback) {
        validation_error(
            &format!("{path}.on"),
            &format!("callback '{callback}' was renamed to '{replacement}'; use '{replacement}'"),
        )
    } else {
        validation_error(
            &format!("{path}.on"),
            &format!("callback '{callback}' is not allowed for rule type '{rule_type}'"),
        )
    }
}

fn renamed_callback(callback: &str) -> Option<&'static str> {
    match callback {
        "dns.query" => Some("dns.request"),
        _ => None,
    }
}

fn validate_rewrite_target_and_value(path: &str, target: &str, value: &str) -> Result<()> {
    let target = target.trim();
    if target.is_empty() {
        validation_error(path, "rewrite_target must not be empty")?;
    }

    let captures = rewrite_target_captures(path, target)?;
    let replacement_references = replacement_capture_references(path, value)?;
    for reference in replacement_references {
        if !captures.contains(reference.as_str()) {
            validation_error(
                &format!("{path}.replace"),
                &format!("rewrite_value references unknown capture '{reference}'"),
            )?;
        }
    }
    Ok(())
}

fn rewrite_target_captures(path: &str, target: &str) -> Result<BTreeSet<String>> {
    let Some((_, rhs)) = target.split_once("=~") else {
        return Ok(BTreeSet::new());
    };
    let regex_text = rhs.trim();
    if regex_text.len() < 2 {
        validation_error(path, "rewrite_target regex must be quoted")?;
    }
    let quote = regex_text.as_bytes()[0] as char;
    if quote != '"' && quote != '\'' {
        validation_error(path, "rewrite_target regex must be quoted")?;
    }
    let end = if let Some(index) = regex_text[1..].rfind(quote) {
        index
    } else {
        return validation_error(path, "rewrite_target regex is missing a closing quote");
    };
    let trailing = &regex_text[end + 2..];
    if !trailing.trim().is_empty() {
        validation_error(
            path,
            "rewrite_target regex has trailing content after closing quote",
        )?;
    }
    let pattern = &regex_text[1..=end];
    let compiled = Regex::new(pattern).map_err(|error| SettingsProfilesError::Validation {
        path: path.to_string(),
        message: format!("invalid rewrite_target regex: {error}"),
    })?;
    Ok(compiled
        .capture_names()
        .flatten()
        .map(ToOwned::to_owned)
        .collect())
}

fn replacement_capture_references(path: &str, value: &str) -> Result<Vec<String>> {
    let reference_re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").map_err(|error| {
        SettingsProfilesError::Validation {
            path: path.to_string(),
            message: format!("invalid replacement reference regex: {error}"),
        }
    })?;
    Ok(reference_re
        .captures_iter(value)
        .filter_map(|caps| caps.get(1).map(|capture| capture.as_str().to_string()))
        .collect())
}

fn validate_header_names(path: &str, headers: &[String]) -> Result<()> {
    for header in headers {
        let trimmed = header.trim();
        if trimmed.is_empty() {
            validation_error(path, "HTTP header name cannot be empty")?;
        }
        http::header::HeaderName::from_bytes(trimmed.as_bytes()).map_err(|_| {
            SettingsProfilesError::Validation {
                path: path.to_string(),
                message: format!("invalid HTTP header name '{header}'"),
            }
        })?;
    }
    Ok(())
}

fn validate_string_ids(path: &str, values: &[String]) -> Result<()> {
    for value in values {
        validate_config_id(path, value)?;
    }
    Ok(())
}

fn ensure_no_duplicate_ids(path: &str, values: &[String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value.as_str()) {
            validation_error(path, &format!("duplicate id '{value}'"))?;
        }
    }
    Ok(())
}

fn validation_error<T>(path: &str, message: &str) -> Result<T> {
    Err(SettingsProfilesError::Validation {
        path: path.to_string(),
        message: message.to_string(),
    })
}

#[cfg(test)]
mod tests;
