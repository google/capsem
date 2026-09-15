//! What the guest launcher needs in the VM's stage directory, independent of
//! who writes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

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
/// command override and container environment) and the launcher.
pub fn stage_plan(
    root: &Path,
    files: &[PathBuf],
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<Vec<StagedFile>> {
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
        content: StagedContent::Bytes(serde_json::to_vec(&serde_json::json!({"args": args, "env": env}))?),
    });
    plan.push(StagedFile {
        name: "launch.py".into(),
        content: StagedContent::Bytes(LAUNCHER.to_vec()),
    });
    Ok(plan)
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
