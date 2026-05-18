use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Result};
use capsem_core::asset_manager::{hash_filename, ResolvedAssets};

use crate::api::SavedVmAssetDependency;
use crate::registry::{PersistentRegistry, PersistentVmEntry, SavedVmBaseAssets};

const LOGICAL_KERNEL: &str = "vmlinuz";
const LOGICAL_INITRD: &str = "initrd.img";
const LOGICAL_ROOTFS: &str = "rootfs.squashfs";
pub fn referenced_asset_filenames(entry: &PersistentVmEntry) -> Vec<String> {
    entry
        .base_assets
        .as_ref()
        .map(saved_asset_filenames)
        .unwrap_or_default()
}

pub fn registry_referenced_asset_filenames(registry: &PersistentRegistry) -> HashSet<String> {
    registry
        .list()
        .flat_map(referenced_asset_filenames)
        .collect()
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
