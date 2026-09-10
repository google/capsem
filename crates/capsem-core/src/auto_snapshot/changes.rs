//! Compare one checkpoint to the live workspace using descriptor-relative reads.
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions, EntryKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
}

#[derive(Debug, Clone)]
pub struct WorkspaceChange {
    pub path: String,
    pub kind: ChangeKind,
    pub size: Option<u64>,
    pub is_symlink: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    size: u64,
    is_symlink: bool,
    digest: blake3::Hash,
}

fn digest_file(mut file: impl Read) -> io::Result<blake3::Hash> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize())
}

pub(crate) fn snapshot_entry_digest(path: &Path, is_symlink: bool) -> Option<blake3::Hash> {
    if is_symlink {
        let target = std::fs::read_link(path).ok()?;
        Some(blake3::hash(target.as_os_str().as_encoded_bytes()))
    } else {
        digest_file(std::fs::File::open(path).ok()?).ok()
    }
}

fn collect(dir: &ContainedDir, prefix: &str, files: &mut BTreeMap<String, Entry>) -> io::Result<()> {
    let mut pending = vec![(dir.try_clone()?, prefix.to_owned())];
    while let Some((dir, prefix)) = pending.pop() {
        for item in dir.entries()? {
            let Some(name) = item.name.to_str() else {
                // JSON cannot address these names without conflating distinct byte paths.
                continue;
            };
            let path = if prefix.is_empty() {
                name.into()
            } else {
                format!("{prefix}/{name}")
            };
            let (size, is_symlink, digest) = match item.kind {
                EntryKind::Directory => {
                    pending.push((dir.descend(&item.name)?, path));
                    continue;
                }
                EntryKind::File => {
                    let file = dir.open_file(&item.name, ContainedOpenOptions::read_only())?;
                    let size = file.metadata()?.len();
                    (size, false, digest_file(file)?)
                }
                EntryKind::Other => {
                    let Some(target) = dir.read_link(&item.name)? else {
                        continue;
                    };
                    (
                        target.as_encoded_bytes().len() as u64,
                        true,
                        blake3::hash(target.as_encoded_bytes()),
                    )
                }
            };
            files.insert(
                path,
                Entry {
                    size,
                    is_symlink,
                    digest,
                },
            );
        }
    }
    Ok(())
}

/// Compare contents and entry kind, preserving symlinks without following them.
/// Results are ordered by relative path. This is an observation of a live
/// workspace, not an atomic snapshot; unreadable/disappearing entries fail.
pub fn workspace_changes(checkpoint: &Path, workspace: &Path) -> io::Result<Vec<WorkspaceChange>> {
    let mut before = BTreeMap::new();
    let mut current = BTreeMap::new();
    collect(&ContainedDir::open_root(checkpoint)?, "", &mut before)?;
    collect(&ContainedDir::open_root(workspace)?, "", &mut current)?;
    let mut changes = BTreeMap::new();
    for (path, entry) in &current {
        let kind = match before.get(path) {
            Some(old) if old == entry => continue,
            Some(_) => ChangeKind::Modified,
            None => ChangeKind::Created,
        };
        changes.insert(
            path.clone(),
            WorkspaceChange {
                path: path.clone(),
                kind,
                size: Some(entry.size),
                is_symlink: entry.is_symlink,
            },
        );
    }
    for (path, entry) in before {
        if !current.contains_key(&path) {
            changes.insert(
                path.clone(),
                WorkspaceChange {
                    path,
                    kind: ChangeKind::Deleted,
                    size: None,
                    is_symlink: entry.is_symlink,
                },
            );
        }
    }
    Ok(changes.into_values().collect())
}

#[cfg(test)]
mod tests;
