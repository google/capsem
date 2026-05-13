//! Pasteable debug report for Settings -> About.
//!
//! This is intentionally smaller than `capsem support-bundle`: it produces
//! redacted text that users can paste into a bug without unpacking a tarball.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug)]
pub struct DebugReportInput {
    pub generated_at: String,
    pub version: String,
    pub build_hash: String,
    pub build_ts: String,
    pub platform: String,
    pub capsem_home: PathBuf,
    pub run_dir: PathBuf,
    pub assets_dir: PathBuf,
    pub manifest: Option<capsem_core::asset_manager::ManifestV2>,
    pub running_vm_count: usize,
    pub total_vm_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugReport {
    pub text: String,
}

pub fn build_debug_report(input: DebugReportInput) -> Result<DebugReport> {
    let mut lines = Vec::new();
    lines.push("Capsem Debug Report".to_string());
    lines.push("redacted: true".to_string());
    lines.push(format!("generated_at: {}", input.generated_at));
    lines.push(String::new());
    lines.push("[version]".to_string());
    lines.push(format!("capsem_version: {}", input.version));
    lines.push(format!("build_hash: {}", input.build_hash));
    lines.push(format!("build_ts: {}", input.build_ts));
    lines.push(format!("platform: {}", input.platform));
    lines.push(String::new());
    lines.push("[paths]".to_string());
    lines.push(format!(
        "capsem_home: {}",
        redact_path_for_report(&input.capsem_home)
    ));
    lines.push(format!(
        "run_dir: {}",
        redact_path_for_report(&input.run_dir)
    ));
    lines.push(format!(
        "assets_dir: {}",
        redact_path_for_report(&input.assets_dir)
    ));
    lines.push(String::new());
    lines.push("[runtime]".to_string());
    lines.push(format!("running_vm_count: {}", input.running_vm_count));
    lines.push(format!("total_vm_count: {}", input.total_vm_count));
    lines.push(String::new());
    lines.push("[assets]".to_string());
    append_asset_report(&mut lines, &input)?;

    Ok(DebugReport {
        text: lines.join("\n"),
    })
}

pub fn redact_path_for_report(path: &Path) -> String {
    redact_home_prefix(&path.display().to_string())
}

fn append_asset_report(lines: &mut Vec<String>, input: &DebugReportInput) -> Result<()> {
    let Some(manifest) = input.manifest.as_ref() else {
        lines.push("manifest_present: false".to_string());
        return Ok(());
    };

    let arch = capsem_core::asset_manager::host_manifest_arch();
    lines.push("manifest_present: true".to_string());
    lines.push(format!(
        "manifest_assets_current: {}",
        manifest.assets.current
    ));
    lines.push(format!(
        "manifest_binaries_current: {}",
        manifest.binaries.current
    ));
    lines.push(format!("manifest_arch: {arch}"));

    let resolved = manifest
        .resolve(&input.version, arch, &input.assets_dir)
        .context("resolve manifest assets for debug report")?;
    lines.push(format!(
        "asset_version_for_binary: {}",
        resolved.asset_version
    ));

    let release = manifest
        .assets
        .releases
        .get(&resolved.asset_version)
        .with_context(|| format!("asset version {} not found", resolved.asset_version))?;
    let arch_assets = release.arches.get(arch).with_context(|| {
        format!(
            "arch {arch} not found in asset release {}",
            resolved.asset_version
        )
    })?;

    for (label, logical, path) in [
        ("kernel", "vmlinuz", resolved.kernel.as_path()),
        ("initrd", "initrd.img", resolved.initrd.as_path()),
        ("rootfs", "rootfs.squashfs", resolved.rootfs.as_path()),
    ] {
        append_one_asset(lines, label, logical, path, arch_assets)?;
    }

    Ok(())
}

fn append_one_asset(
    lines: &mut Vec<String>,
    label: &str,
    logical: &str,
    path: &Path,
    arch_assets: &std::collections::HashMap<String, capsem_core::asset_manager::AssetEntry>,
) -> Result<()> {
    let expected = arch_assets
        .get(logical)
        .with_context(|| format!("{logical} missing from manifest arch assets"))?;
    lines.push(format!("{label}_manifest_hash: {}", expected.hash));
    lines.push(format!("{label}_path: {}", redact_path_for_report(path)));
    let exists = path.exists();
    lines.push(format!("{label}_exists: {exists}"));
    if exists {
        let actual = capsem_core::asset_manager::hash_file(path)
            .with_context(|| format!("hash {}", path.display()))?;
        lines.push(format!("{label}_actual_hash: {actual}"));
        lines.push(format!(
            "{label}_actual_hash_matches_manifest: {}",
            actual == expected.hash
        ));
    } else {
        lines.push(format!("{label}_actual_hash: <missing>"));
        lines.push(format!("{label}_actual_hash_matches_manifest: false"));
    }
    Ok(())
}

fn redact_home_prefix(value: &str) -> String {
    if let Some(idx) = value.find("/Users/") {
        redact_user_segment(value, idx, "/Users/".len())
    } else if let Some(idx) = value.find("/home/") {
        redact_user_segment(value, idx, "/home/".len())
    } else {
        value.to_string()
    }
}

fn redact_user_segment(value: &str, idx: usize, prefix_len: usize) -> String {
    let mut out = value.to_string();
    if let Some(end) = out[idx + prefix_len..].find('/') {
        let abs_end = idx + prefix_len + end + 1;
        out.replace_range(idx..abs_end, "~/");
    }
    out
}

#[cfg(test)]
mod tests;
