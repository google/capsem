//! The manifest the guest launcher reassembles and verifies an image layout from.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{ensure, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// One layout file as the guest launcher reads it back: parts named
/// `{key}-{n}` concatenate to `path`, whose SHA-256 must equal `sha256`.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TransferEntry {
    pub path: String,
    pub key: usize,
    pub parts: usize,
    pub sha256: String,
}

impl TransferEntry {
    /// The staged part name holding this file, when it has content.
    pub fn part_name(&self) -> Option<String> {
        (self.parts > 0).then(|| format!("{}-0", self.key))
    }
}

/// Describe `files` under `root` as single-part transfers.
///
/// Each file with content is staged whole as part `{key}-0`; empty files have
/// no parts. Paths must stay inside the layout, as the launcher also checks.
pub fn transfer_manifest(root: &Path, files: &[PathBuf]) -> Result<Vec<TransferEntry>> {
    files
        .iter()
        .enumerate()
        .map(|(key, path)| {
            ensure!(
                path.components().all(|part| matches!(part, Component::Normal(_))),
                "unsafe OCI layout path: {}",
                path.display()
            );
            let mut input = std::fs::File::open(root.join(path)).with_context(|| format!("open {}", path.display()))?;
            let mut hash = Sha256::new();
            let mut buffer = vec![0; 1024 * 1024];
            let mut length = 0u64;
            loop {
                let read = input.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hash.update(&buffer[..read]);
                length += read as u64;
            }
            Ok(TransferEntry {
                path: path.to_str().context("non-UTF8 OCI layout path")?.to_owned(),
                key,
                parts: usize::from(length > 0),
                sha256: format!("{:x}", hash.finalize()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
