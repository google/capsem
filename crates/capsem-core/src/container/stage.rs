//! What the guest launcher needs in the VM's stage directory, independent of
//! who writes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::LAUNCHER;

/// One file written into [`super::STAGE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
    pub name: String,
    pub content: StagedContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedContent {
    /// A verified image layout file, copied whole.
    File(PathBuf),
    /// Generated launcher input.
    Bytes(Vec<u8>),
}

/// The stage for a pulled layout at `root`, in write order: every layout file
/// with content as its single part, then `transfer.json`, `options.json` (the
/// command override, container environment, where the container sees the
/// workspace, and the capabilities, syscall filter, user-namespace map and
/// resources the workload gets) and
/// the launcher.
pub fn stage_plan(
    root: &Path,
    files: &[PathBuf],
    args: &[String],
    env: &BTreeMap<String, String>,
    resources: super::WorkloadResources,
) -> Result<Vec<StagedFile>> {
    let labels = image_labels(root)?;
    let surface = super::seccomp::Surface::from_label(labels.get(SURFACE_LABEL).and_then(|v| v.as_str()))?;
    let transfer = capsem_assets::oci::transfer_manifest(root, files)?;
    let mut plan: Vec<StagedFile> = transfer
        .iter()
        .filter_map(|entry| {
            entry.part_name().map(|name| StagedFile {
                name,
                content: StagedContent::File(root.join(&entry.path)),
            })
        })
        .collect();
    plan.push(StagedFile {
        name: "transfer.json".into(),
        content: StagedContent::Bytes(serde_json::to_vec(&transfer)?),
    });
    plan.push(StagedFile {
        name: "options.json".into(),
        content: StagedContent::Bytes(serde_json::to_vec(&serde_json::json!({
            "args": args,
            "env": env,
            "workspace": super::CONTAINER_WORKSPACE,
            "capabilities": super::seccomp::WORKLOAD_CAPABILITIES,
            "seccomp": super::seccomp::workload_seccomp(oci_architecture()?, surface)?,
            "id_map": super::WORKLOAD_ID_MAP,
            "resources": resources,
            "surface": match surface {
                super::seccomp::Surface::Terminal => "terminal",
                super::seccomp::Surface::Xpra => "xpra",
            },
        }))?),
    });
    plan.push(StagedFile {
        name: "launch.py".into(),
        content: StagedContent::Bytes(LAUNCHER.to_vec()),
    });
    Ok(plan)
}

/// The label an image declares its surface with (`terminal` or `xpra`).
pub const SURFACE_LABEL: &str = "org.capsem.surface";

/// The image config's labels from a pulled single-image layout: index.json
/// names the manifest, the manifest names the config. The layout is the
/// puller's verified output, so its blobs are read as written.
pub fn image_labels(root: &Path) -> Result<serde_json::Map<String, serde_json::Value>> {
    let read = |path: PathBuf| -> Result<serde_json::Value> {
        serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("read {}", path.display()))?)
            .with_context(|| format!("parse {}", path.display()))
    };
    let blob = |digest: &serde_json::Value| -> Result<PathBuf> {
        let digest = digest.as_str().context("digest")?;
        let hex = capsem_assets::oci::Digest::parse(digest)?
            .as_str()
            .trim_start_matches("sha256:")
            .to_owned();
        Ok(root.join("blobs/sha256").join(hex))
    };
    let index = read(root.join("index.json"))?;
    // A layout naming no manifest declares nothing: the terminal surface,
    // the narrowest filter, is what that gives.
    if index["manifests"][0].is_null() {
        return Ok(serde_json::Map::new());
    }
    let manifest = read(blob(&index["manifests"][0]["digest"])?)?;
    let config = read(blob(&manifest["config"]["digest"])?)?;
    Ok(config["config"]["Labels"].as_object().cloned().unwrap_or_default())
}

/// The OCI platform architecture of this host.
pub fn oci_architecture() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Ok("arm64"),
        "x86_64" => Ok("amd64"),
        other => bail!("unsupported container architecture: {other}"),
    }
}

#[cfg(test)]
mod tests;
