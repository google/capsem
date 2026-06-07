use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Result};
use capsem_core::asset_manager::{hash_filename, ResolvedAssets};
use capsem_core::settings_profiles::ProfileRootSettings;

use crate::api::SavedVmAssetDependency;
use crate::registry::{PersistentRegistry, PersistentVmEntry, SavedVmBaseAssets};

const LOGICAL_KERNEL: &str = "vmlinuz";
const LOGICAL_INITRD: &str = "initrd.img";
const LOGICAL_ROOTFS: &str = "rootfs.squashfs";
pub fn referenced_asset_filenames(entry: &PersistentVmEntry) -> Vec<String> {
    let mut filenames = HashSet::new();
    if let Some(base_assets) = &entry.base_assets {
        filenames.extend(saved_asset_filenames(base_assets));
    }
    if let Some(base_assets) = entry
        .profile_pin
        .as_ref()
        .and_then(|pin| pin.base_assets.as_ref())
    {
        filenames.extend(saved_asset_filenames(base_assets));
    }
    let mut filenames = filenames.into_iter().collect::<Vec<_>>();
    filenames.sort();
    filenames
}

pub fn registry_referenced_asset_filenames(registry: &PersistentRegistry) -> HashSet<String> {
    registry
        .list()
        .flat_map(referenced_asset_filenames)
        .collect()
}

pub fn cleanup_retention_asset_filenames(
    registry: &PersistentRegistry,
    roots: &ProfileRootSettings,
) -> Result<HashSet<String>> {
    let mut filenames = registry_referenced_asset_filenames(registry);
    filenames.extend(capsem_core::settings_profiles::installed_profile_asset_filenames(roots)?);
    Ok(filenames)
}

pub fn saved_asset_filenames(base_assets: &SavedVmBaseAssets) -> Vec<String> {
    vec![
        hash_filename(LOGICAL_KERNEL, &base_assets.kernel_hash),
        hash_filename(LOGICAL_INITRD, &base_assets.initrd_hash),
        hash_filename(LOGICAL_ROOTFS, &base_assets.rootfs_hash),
    ]
}

pub fn resolve_saved_base_assets(
    base_dir: &Path,
    base_assets: &SavedVmBaseAssets,
) -> ResolvedAssets {
    let resolve_one = |logical_name: &str, hash: &str| {
        let filename = hash_filename(logical_name, hash);
        let flat = base_dir.join(&filename);
        if flat.exists() {
            return flat;
        }
        let arch_path = base_dir.join(&base_assets.arch).join(&filename);
        if arch_path.exists() {
            return arch_path;
        }
        flat
    };

    ResolvedAssets {
        kernel: resolve_one(LOGICAL_KERNEL, &base_assets.kernel_hash),
        initrd: resolve_one(LOGICAL_INITRD, &base_assets.initrd_hash),
        rootfs: resolve_one(LOGICAL_ROOTFS, &base_assets.rootfs_hash),
        asset_version: base_assets.asset_version.clone(),
    }
}

pub fn missing_saved_base_asset_names(
    base_dir: &Path,
    base_assets: &SavedVmBaseAssets,
) -> Vec<String> {
    let resolved = resolve_saved_base_assets(base_dir, base_assets);
    [
        (LOGICAL_KERNEL, resolved.kernel),
        (LOGICAL_INITRD, resolved.initrd),
        (LOGICAL_ROOTFS, resolved.rootfs),
    ]
    .into_iter()
    .filter_map(|(name, path)| (!path.exists()).then(|| name.to_string()))
    .collect()
}

pub fn ensure_saved_base_assets_available(
    vm_name: &str,
    base_dir: &Path,
    base_assets: &SavedVmBaseAssets,
) -> Result<ResolvedAssets> {
    let missing = missing_saved_base_asset_names(base_dir, base_assets);
    if !missing.is_empty() {
        bail!(
            "saved VM {vm_name} is missing pinned base assets (asset_version={}, arch={}): {}. Restore the missing asset files or purge/recreate the VM before resuming.",
            base_assets.asset_version,
            base_assets.arch,
            missing.join(", ")
        );
    }
    Ok(resolve_saved_base_assets(base_dir, base_assets))
}

pub fn saved_vm_dependency_issues(
    registry: &PersistentRegistry,
    base_dir: &Path,
) -> Vec<SavedVmAssetDependency> {
    let mut issues: Vec<SavedVmAssetDependency> = registry
        .list()
        .filter_map(|entry| {
            let base_assets = entry.base_assets.as_ref()?;
            let missing = missing_saved_base_asset_names(base_dir, base_assets);
            (!missing.is_empty()).then(|| SavedVmAssetDependency {
                vm: entry.name.clone(),
                asset_version: base_assets.asset_version.clone(),
                arch: base_assets.arch.clone(),
                missing,
                recovery_hint: "Restore the missing saved-VM asset files or purge/recreate the VM."
                    .to_string(),
            })
        })
        .collect();
    issues.sort_by(|left, right| left.vm.cmp(&right.vm));
    issues
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use capsem_core::asset_manager::cleanup_unreferenced_assets_preserving;
    use capsem_core::settings_profiles::ProfileRootSettings;

    use super::*;
    use crate::registry::{PersistentRegistry, SavedVmProfilePin};

    fn base_assets(label: &str, kernel: char, initrd: char, rootfs: char) -> SavedVmBaseAssets {
        let hash = |ch: char| std::iter::repeat_n(ch, 64).collect::<String>();
        SavedVmBaseAssets {
            asset_version: format!("{label}@2026.0520.1"),
            arch: "arm64".to_string(),
            kernel_hash: hash(kernel),
            initrd_hash: hash(initrd),
            rootfs_hash: hash(rootfs),
            guest_abi: Some("capsem-guest-v2".to_string()),
        }
    }

    fn entry_with_profile_pin_assets() -> PersistentVmEntry {
        entry_with_profile_pin_base_assets(base_assets("profile-a", 'a', 'b', 'c'))
    }

    fn entry_with_profile_pin_base_assets(pinned_assets: SavedVmBaseAssets) -> PersistentVmEntry {
        PersistentVmEntry {
            name: "saved-vm".to_string(),
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.0.0".to_string(),
            base_assets: None,
            profile_pin: Some(SavedVmProfilePin {
                profile_id: "everyday-work".to_string(),
                profile_revision: Some("2026.0520.1".to_string()),
                profile_payload_hash: Some(
                    "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
                        .to_string(),
                ),
                package_contract_hash:
                    "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                        .to_string(),
                base_assets: Some(pinned_assets),
            }),
            created_at: "0".to_string(),
            session_dir: PathBuf::from("/tmp/saved-vm"),
            forked_from: None,
            description: None,
            suspended: false,
            defunct: false,
            last_error: None,
            checkpoint_path: None,
            env: None,
        }
    }

    fn install_current_profile_payload(corp_dir: &std::path::Path) {
        let record_dir = corp_dir
            .join(".catalog")
            .join("profiles")
            .join("everyday-work");
        std::fs::create_dir_all(record_dir.join("2026.0520.1")).unwrap();
        std::fs::write(
            record_dir.join("2026.0520.1").join("profile.json"),
            include_str!("../../../schemas/fixtures/profile-v2-valid.json"),
        )
        .unwrap();
        std::fs::write(
            record_dir.join("current.json"),
            r#"{
              "profile_id": "everyday-work",
              "revision": "2026.0520.1",
              "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn referenced_asset_filenames_include_profile_pin_assets() {
        let entry = entry_with_profile_pin_assets();

        let filenames = referenced_asset_filenames(&entry);

        assert!(filenames.contains(&"vmlinuz-aaaaaaaaaaaaaaaa".to_string()));
        assert!(filenames.contains(&"initrd-bbbbbbbbbbbbbbbb.img".to_string()));
        assert!(filenames.contains(&"rootfs-cccccccccccccccc.squashfs".to_string()));
    }

    #[test]
    fn cleanup_retention_filenames_preserve_installed_profiles_and_profile_pins() {
        let temp = tempfile::tempdir().unwrap();
        let assets_dir = temp.path().join("assets");
        let corp_dir = temp.path().join("profiles").join("corp");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&corp_dir).unwrap();
        for filename in [
            "vmlinuz-aaaaaaaaaaaaaaaa",
            "initrd-bbbbbbbbbbbbbbbb.img",
            "rootfs-cccccccccccccccc.squashfs",
            "vmlinuz-dddddddddddddddd",
            "initrd-eeeeeeeeeeeeeeee.img",
            "rootfs-ffffffffffffffff.squashfs",
        ] {
            std::fs::write(assets_dir.join(filename), filename.as_bytes()).unwrap();
        }
        let disposable = assets_dir.join("rootfs-1111111111111111.squashfs");
        std::fs::write(&disposable, b"delete me").unwrap();
        install_current_profile_payload(&corp_dir);

        let roots = ProfileRootSettings {
            base_dirs: vec![temp.path().join("profiles").join("base")],
            corp_dirs: vec![corp_dir],
            user_dirs: vec![temp.path().join("profiles").join("user")],
            ..ProfileRootSettings::default()
        };
        let registry_path = temp.path().join("registry.json");
        let mut registry = PersistentRegistry::load(registry_path);
        registry.data.vms.insert(
            "saved-vm".to_string(),
            entry_with_profile_pin_base_assets(base_assets("profile-d", 'd', 'e', 'f')),
        );

        let retention = cleanup_retention_asset_filenames(&registry, &roots).unwrap();
        let removed = cleanup_unreferenced_assets_preserving(&assets_dir, retention).unwrap();

        assert_eq!(removed, vec![disposable]);
        assert!(assets_dir.join("vmlinuz-aaaaaaaaaaaaaaaa").exists());
        assert!(assets_dir.join("initrd-bbbbbbbbbbbbbbbb.img").exists());
        assert!(assets_dir.join("rootfs-cccccccccccccccc.squashfs").exists());
        assert!(assets_dir.join("vmlinuz-dddddddddddddddd").exists());
        assert!(assets_dir.join("initrd-eeeeeeeeeeeeeeee.img").exists());
        assert!(assets_dir.join("rootfs-ffffffffffffffff.squashfs").exists());
    }
}
