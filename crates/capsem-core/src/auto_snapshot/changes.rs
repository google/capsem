//! Compare one checkpoint to the live workspace using descriptor-relative reads.
//!
//! A checkpoint records the stat identity of every live entry before cloning
//! the workspace (`WorkspaceManifest`). A comparison then reads only entries
//! whose identity moved: an unchanged workspace is answered from metadata
//! alone instead of hashing both trees on every page of the listing.
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use capsem_foundation::unix::contained::{
    ContainedDir, ContainedEntry, ContainedOpenOptions, EntryIdentity, EntryKind,
};
use serde::{Deserialize, Serialize};

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

/// Written into the checkpoint's slot directory, which the guest cannot reach.
pub const MANIFEST_FILE: &str = "workspace-manifest.json";

/// The stat identity of each live entry, taken before a checkpoint clones it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    /// ctime of a marker written *before* the walk. An entry is trusted only if
    /// its ctime is strictly older: an edit in the same timestamp tick as its
    /// stat can leave ctime unchanged (the racy-git problem), while any edit
    /// after the clock was read produces a ctime that is not older than it.
    clock: (i64, i64),
    entries: BTreeMap<String, Identity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Identity {
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl From<EntryIdentity> for Identity {
    fn from(identity: EntryIdentity) -> Self {
        Self {
            ino: identity.ino,
            size: identity.size,
            mtime: identity.mtime,
            ctime: identity.ctime,
        }
    }
}

impl WorkspaceManifest {
    /// Record the live workspace. Must run before the workspace is cloned: an
    /// entry that changes after its stat then no longer matches, so the
    /// comparison reads it instead of trusting a clone of the newer contents.
    pub fn capture(workspace: &Path, clock_dir: &Path) -> io::Result<Self> {
        let marker = clock_dir.join(".manifest-clock");
        std::fs::write(&marker, [])?;
        let stamp = std::fs::symlink_metadata(&marker)?;
        std::fs::remove_file(&marker)?;
        let mut entries = BTreeMap::new();
        walk(&ContainedDir::open_root(workspace)?, &mut |path, entry| {
            if !matches!(entry.kind, EntryKind::Directory) {
                entries.insert(path.to_owned(), entry.identity.into());
            }
        })?;
        Ok(Self {
            clock: (stamp.ctime(), stamp.ctime_nsec()),
            entries,
        })
    }

    pub fn save(&self, slot_dir: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        std::fs::write(slot_dir.join(MANIFEST_FILE), bytes)
    }

    /// Absent or unreadable means an exact comparison, never a trusted one.
    pub fn load(slot_dir: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(slot_dir.join(MANIFEST_FILE)).ok()?).ok()
    }

    fn trusts(&self, path: &str, live: &EntryIdentity) -> bool {
        self.entries
            .get(path)
            .is_some_and(|recorded| *recorded == Identity::from(*live) && recorded.ctime < self.clock)
    }
}

enum Seen {
    File(EntryIdentity),
    Symlink(std::ffi::OsString),
}

impl Seen {
    fn size(&self) -> u64 {
        match self {
            Self::File(identity) => identity.size,
            Self::Symlink(target) => target.as_encoded_bytes().len() as u64,
        }
    }
}

/// Every non-directory entry, keyed by relative path, without following links.
fn walk(root: &ContainedDir, visit: &mut dyn FnMut(&str, &ContainedEntry)) -> io::Result<()> {
    let mut pending = vec![(root.try_clone()?, String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        for item in dir.entries()? {
            let Some(name) = item.name.to_str() else {
                // JSON cannot address these names without conflating distinct byte paths.
                continue;
            };
            let path = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            if item.kind == EntryKind::Directory {
                pending.push((dir.descend(&item.name)?, path.clone()));
            }
            visit(&path, &item);
        }
    }
    Ok(())
}

fn collect(root: &ContainedDir) -> io::Result<BTreeMap<String, Seen>> {
    let mut seen = BTreeMap::new();
    let mut failure = None;
    walk(root, &mut |path, entry| {
        if failure.is_some() {
            return;
        }
        match entry.kind {
            EntryKind::Directory => {}
            EntryKind::File => {
                seen.insert(path.to_owned(), Seen::File(entry.identity));
            }
            EntryKind::Other => match parent_and_name(root, path).and_then(|(dir, name)| dir.read_link(name)) {
                Ok(Some(target)) => {
                    seen.insert(path.to_owned(), Seen::Symlink(target));
                }
                Ok(None) => {}
                Err(error) => failure = Some(error),
            },
        }
    })?;
    failure.map_or(Ok(seen), Err)
}

fn parent_and_name<'a>(root: &ContainedDir, path: &'a str) -> io::Result<(ContainedDir, &'a OsStr)> {
    let path = Path::new(path);
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("entry without a name"))?;
    Ok((root.walk(path.parent().unwrap_or(Path::new("")))?, name))
}

fn open(root: &ContainedDir, path: &str) -> io::Result<std::fs::File> {
    let (dir, name) = parent_and_name(root, path)?;
    dir.open_file(name, ContainedOpenOptions::read_only())
}

/// Compare contents and entry kind, preserving symlinks without following them.
/// Results are ordered by relative path. This is an observation of a live
/// workspace, not an atomic snapshot; unreadable/disappearing entries fail.
pub fn workspace_changes(checkpoint: &Path, workspace: &Path) -> io::Result<Vec<WorkspaceChange>> {
    diff(checkpoint, workspace, None, &mut |file| digest_file(file))
}

/// Changes since `snapshot`, reading only entries its manifest cannot vouch for.
pub fn changes_since(snapshot: &super::SnapshotSlot, workspace: &Path) -> io::Result<Vec<WorkspaceChange>> {
    let manifest = snapshot.workspace_path.parent().and_then(WorkspaceManifest::load);
    diff(&snapshot.workspace_path, workspace, manifest.as_ref(), &mut |file| {
        digest_file(file)
    })
}

fn diff(
    checkpoint: &Path,
    workspace: &Path,
    manifest: Option<&WorkspaceManifest>,
    digest: &mut dyn FnMut(std::fs::File) -> io::Result<blake3::Hash>,
) -> io::Result<Vec<WorkspaceChange>> {
    let before_root = ContainedDir::open_root(checkpoint)?;
    let current_root = ContainedDir::open_root(workspace)?;
    let before = collect(&before_root)?;
    let current = collect(&current_root)?;
    let mut changes = Vec::new();
    for (path, entry) in &current {
        let kind = match (before.get(path), entry) {
            (None, _) => Some(ChangeKind::Created),
            (Some(Seen::Symlink(old)), Seen::Symlink(new)) => (old != new).then_some(ChangeKind::Modified),
            (Some(Seen::File(old)), Seen::File(new)) if old.size == new.size => {
                let same = manifest.is_some_and(|manifest| manifest.trusts(path, new))
                    || digest(open(&before_root, path)?)? == digest(open(&current_root, path)?)?;
                (!same).then_some(ChangeKind::Modified)
            }
            (Some(_), _) => Some(ChangeKind::Modified),
        };
        if let Some(kind) = kind {
            changes.push(WorkspaceChange {
                path: path.clone(),
                kind,
                size: Some(entry.size()),
                is_symlink: matches!(entry, Seen::Symlink(_)),
            });
        }
    }
    for (path, entry) in &before {
        if !current.contains_key(path) {
            changes.push(WorkspaceChange {
                path: path.clone(),
                kind: ChangeKind::Deleted,
                size: None,
                is_symlink: matches!(entry, Seen::Symlink(_)),
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

#[cfg(test)]
mod tests;
