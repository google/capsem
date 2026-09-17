//! The installed profile catalog: what profiles exist, where they came from,
//! and which one a client gets when it names none.
//!
//! Split from `profile_contract`, which owns what one profile *is*. This owns
//! the set: loading a directory of ledgers, overlaying installed release
//! assets, and answering the catalog-level questions.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::profile_contract::{builtin_profile_configs, ProfileConfigFile, ProfileFileDescriptor};

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileCatalog {
    profiles: BTreeMap<String, ProfileConfigFile>,
    source: ProfileCatalogSource,
}

/// What a profile is the default for. Kept apart because the two diverge: a
/// container image brings its own userland, so the profile that boots a full
/// VM workstation is not the profile a container should get by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileRuntime {
    Vm,
    Container,
}

impl ProfileRuntime {
    pub const ALL: [Self; 2] = [Self::Vm, Self::Container];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vm => "vm",
            Self::Container => "container",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileCatalogSource {
    BuiltIn,
    Directory(PathBuf),
}

impl ProfileCatalog {
    pub fn builtin() -> Self {
        let profiles = builtin_profile_configs()
            .into_iter()
            .map(|profile| (profile.id.clone(), profile))
            .collect();
        Self {
            profiles,
            source: ProfileCatalogSource::BuiltIn,
        }
    }

    /// The profile a client gets when it names none for that runtime, if the
    /// catalog has one. A container and a VM claim separately: the image a
    /// container runs is not the workstation a VM boots, and the profile that
    /// suits one will not stay the right answer for the other.
    pub fn default_profile_id(&self, runtime: ProfileRuntime) -> Option<&str> {
        self.profiles
            .values()
            .find(|profile| profile.default_for.contains(&runtime))
            .map(|profile| profile.id.as_str())
    }

    /// Every runtime's default in one answer, keyed by runtime name, so a
    /// caller cannot publish the VM's default and forget the container's.
    pub fn default_profile_ids(&self) -> BTreeMap<&'static str, &str> {
        ProfileRuntime::ALL
            .into_iter()
            .filter_map(|runtime| self.default_profile_id(runtime).map(|id| (runtime.as_str(), id)))
            .collect()
    }

    pub fn load_from_dir(path: &Path) -> Result<Self, String> {
        let entries =
            fs::read_dir(path).map_err(|error| format!("read profile directory {}: {error}", path.display()))?;
        let mut profiles = BTreeMap::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("read profile directory entry: {error}"))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("read profile file type: {error}"))?;
            if !file_type.is_dir() {
                continue;
            }
            let profile_dir = entry.path();
            let path = profile_dir.join("profile.toml");
            let content =
                fs::read_to_string(&path).map_err(|error| format!("read profile {}: {error}", path.display()))?;
            let profile: ProfileConfigFile =
                toml::from_str(&content).map_err(|error| format!("parse profile {}: {error}", path.display()))?;
            profile
                .validate()
                .map_err(|error| format!("validate profile {}: {error}", path.display()))?;
            let dir_name = profile_dir.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
                format!(
                    "profile directory {} has no valid directory name",
                    profile_dir.display()
                )
            })?;
            if profile.id != dir_name {
                return Err(format!(
                    "profile file {} id mismatch: directory is {dir_name}, profile id is {}",
                    path.display(),
                    profile.id
                ));
            }
            if profiles.insert(profile.id.clone(), profile).is_some() {
                return Err(format!("duplicate profile id {dir_name}"));
            }
            for runtime in ProfileRuntime::ALL {
                let defaults: Vec<&str> = profiles
                    .values()
                    .filter(|profile| profile.default_for.contains(&runtime))
                    .map(|profile| profile.id.as_str())
                    .collect();
                if defaults.len() > 1 {
                    return Err(format!(
                        "more than one default {} profile: {}",
                        runtime.as_str(),
                        defaults.join(", ")
                    ));
                }
            }
        }
        if profiles.is_empty() {
            return Err(format!(
                "profile directory {} contains no profile directories with profile.toml",
                path.display()
            ));
        }
        Ok(Self {
            profiles,
            source: ProfileCatalogSource::Directory(path.to_path_buf()),
        })
    }

    pub fn load_default() -> Result<Self, String> {
        if let Ok(path) = std::env::var("CAPSEM_PROFILES_DIR") {
            if !path.is_empty() {
                return Self::load_from_dir(Path::new(&path));
            }
        }
        let installed = capsem_foundation::paths::capsem_home().join("profiles");
        if installed.is_dir() {
            let mut catalog = Self::load_from_dir(&installed)?;
            let manifest_path = capsem_foundation::paths::capsem_assets_dir().join("manifest.json");
            if manifest_path.is_file() {
                overlay_release_manifest_assets(&mut catalog, &manifest_path)?;
            }
            return Ok(catalog);
        }
        Ok(Self::builtin())
    }

    pub fn source(&self) -> &ProfileCatalogSource {
        &self.source
    }

    pub fn profiles(&self) -> impl Iterator<Item = &ProfileConfigFile> {
        self.profiles.values()
    }

    pub fn get(&self, profile_id: &str) -> Option<&ProfileConfigFile> {
        self.profiles.get(profile_id)
    }
}

fn overlay_release_manifest_assets(catalog: &mut ProfileCatalog, manifest_path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(manifest_path)
        .map_err(|error| format!("read installed manifest {}: {error}", manifest_path.display()))?;
    let document: serde_json::Value = serde_json::from_str(&content)
        .map_err(|error| format!("parse installed manifest {}: {error}", manifest_path.display()))?;
    let Some(profiles) = document.get("profiles").and_then(serde_json::Value::as_object) else {
        return Ok(());
    };

    let source_label = catalog.source_path_label();
    for (profile_id, profile_value) in profiles {
        let profile = catalog.profiles.get_mut(profile_id).ok_or_else(|| {
            format!(
                "installed manifest profile {profile_id} has no verified profile config in {}",
                source_label
            )
        })?;
        let revision = profile_value
            .get("revision")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("installed manifest profile {profile_id} is missing revision"))?;
        profile.revision = revision.to_string();
        let architectures = profile_value
            .get("architectures")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| format!("installed manifest profile {profile_id} is missing architectures"))?;
        for architecture in architectures {
            let arch = architecture
                .get("architecture")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("installed manifest profile {profile_id} has an unnamed architecture"))?;
            let Some(profile_arch) = profile.assets.arch.get_mut(arch) else {
                continue;
            };
            let images = architecture
                .get("images")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| {
                    format!("installed manifest profile {profile_id} architecture {arch} is missing images")
                })?;
            let mut overlaid = BTreeSet::new();
            for image in images {
                if image.get("status").and_then(serde_json::Value::as_str) == Some("revoked") {
                    continue;
                }
                let kind = image.get("kind").and_then(serde_json::Value::as_str).ok_or_else(|| {
                    format!("installed manifest profile {profile_id} architecture {arch} has an image without kind")
                })?;
                let descriptor = match kind {
                    "kernel" => &mut profile_arch.kernel,
                    "initrd" => &mut profile_arch.initrd,
                    "rootfs" => &mut profile_arch.rootfs,
                    _ => continue,
                };
                let name = image
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("installed manifest {profile_id}/{arch}/{kind} is missing name"))?;
                let url = image
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("installed manifest {profile_id}/{arch}/{kind} is missing URL"))?;
                let size = image
                    .get("bytes")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| format!("installed manifest {profile_id}/{arch}/{kind} is missing byte size"))?;
                let blake3 = image
                    .pointer("/digest/blake3")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("installed manifest {profile_id}/{arch}/{kind} is missing BLAKE3"))?;
                if blake3.len() != 64 || !blake3.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(format!(
                        "installed manifest {profile_id}/{arch}/{kind} has invalid BLAKE3"
                    ));
                }
                descriptor.name = name.to_string();
                descriptor.url = if url.starts_with('/') {
                    let assets_dir = manifest_path.parent().ok_or_else(|| {
                        format!("installed manifest {} has no asset directory", manifest_path.display())
                    })?;
                    format!("file://{}", assets_dir.join(arch).join(name).display())
                } else {
                    url.to_string()
                };
                descriptor.hash = Some(format!("blake3:{blake3}"));
                descriptor.size = Some(size);
                overlaid.insert(kind);
            }
            for required in ["kernel", "initrd", "rootfs"] {
                if !overlaid.contains(required) {
                    return Err(format!(
                        "installed manifest profile {profile_id} architecture {arch} is missing {required} image"
                    ));
                }
            }
            if let Some(configs) = architecture.get("config").and_then(serde_json::Value::as_array) {
                for config in configs {
                    if config.get("status").and_then(serde_json::Value::as_str) == Some("revoked") {
                        continue;
                    }
                    let kind = config.get("kind").and_then(serde_json::Value::as_str).ok_or_else(|| {
                        format!("installed manifest profile {profile_id} architecture {arch} has config without kind")
                    })?;
                    let slot = match kind {
                        "enforcement" => &mut profile.files.enforcement,
                        "detection" => &mut profile.files.detection,
                        "mcp" => &mut profile.files.mcp,
                        "apt_packages" => &mut profile.files.apt_packages,
                        "python_requirements" => &mut profile.files.python_requirements,
                        "python_requirements_lock" => &mut profile.files.python_requirements_lock,
                        "npm_packages" => &mut profile.files.npm_packages,
                        "npm_package_lock" => &mut profile.files.npm_package_lock,
                        "build" => &mut profile.files.build,
                        "tips" => &mut profile.files.tips,
                        "root_manifest" => &mut profile.files.root_manifest,
                        _ => continue,
                    };
                    let path = config
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| format!("installed manifest {profile_id}/{arch}/{kind} is missing path"))?;
                    let size = config
                        .get("bytes")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or_else(|| format!("installed manifest {profile_id}/{arch}/{kind} is missing byte size"))?;
                    let blake3 = config
                        .pointer("/digest/blake3")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| format!("installed manifest {profile_id}/{arch}/{kind} is missing BLAKE3"))?;
                    if blake3.len() != 64 || !blake3.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(format!(
                            "installed manifest {profile_id}/{arch}/{kind} has invalid BLAKE3"
                        ));
                    }
                    *slot = Some(ProfileFileDescriptor {
                        path: path.to_string(),
                        hash: Some(format!("blake3:{blake3}")),
                        size: Some(size),
                    });
                }
            }
        }
    }
    Ok(())
}

impl ProfileCatalog {
    fn source_path_label(&self) -> String {
        match &self.source {
            ProfileCatalogSource::BuiltIn => "built-in profiles".to_string(),
            ProfileCatalogSource::Directory(path) => path.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
