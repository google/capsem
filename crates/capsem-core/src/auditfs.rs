//! The one place a hardlink is made, because a hardlink is not a copy.
//!
//! `fs::hard_link` is the only operation that makes two paths *the same file*.
//! Everything else -- a write, a rename, a chmod -- affects one name. That
//! distinction stopped being academic when `capsem-admin` staged profile
//! payloads with it: 48 checked-in `config/` files ended up inside the
//! published release channel sharing an inode, so a `chmod` on an artifact
//! rewrote tracked source, and no content digest could notice because the
//! bytes never changed.
//!
//! Linking build output to build output is still right, and still fast: asset
//! staging moves multi-gigabyte images and copying them to satisfy a rule
//! aimed at small checked-in seeds would trade a defect for an hour of I/O.
//! So the rule is about *what* is being linked, not about linking.
//!
//! Rust cannot be monkeypatched the way the gate proxies Python's primitives,
//! so this module plus `tests/test_rust_filesystem_chokepoint.py` is the
//! equivalent: one audited call site, and a test that fails when a crate
//! reaches around it.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::{Context, Result};

/// Place `source` at `destination`, linking only when that cannot couple
/// build output to the checked-in tree.
///
/// `root` is the checkout. A source outside it, or under a build-output
/// directory inside it, is disposable and gets a hardlink. A tracked file gets
/// a copy: the artifact must be able to have its own permissions, its own
/// lifetime, and its own inode.
pub fn stage(source: &Path, destination: &Path, root: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create parent for {}", destination.display()))?;
    }
    if destination.exists() {
        fs::remove_file(destination).with_context(|| format!("replace {}", destination.display()))?;
    }

    if is_build_output(source, root) {
        match fs::hard_link(source, destination) {
            Ok(()) => return Ok(()),
            // `EXDEV` and friends: staging onto another filesystem is normal,
            // and is the reason this fell back to copying in the first place.
            Err(_) => return copy(source, destination),
        }
    }
    copy(source, destination)
}

fn copy(source: &Path, destination: &Path) -> Result<()> {
    fs::copy(source, destination).with_context(|| format!("copy {} -> {}", source.display(), destination.display()))?;
    Ok(())
}

/// Whether this path is something the build produced rather than something a
/// human checked in.
///
/// **Only a confident yes permits a link.** The first version answered "yes"
/// for anything it could not place, and a relative path -- which is what the
/// release scripts pass -- could not be placed, so 192 checked-in files were
/// still linked into the published channel after the fix. Failing open on a
/// question about publication integrity is the same mistake as not asking it.
///
/// A needless copy costs I/O. A needless link costs a published artifact that
/// shares an inode with tracked source, which is what this exists to prevent.
///
/// Path-based rather than asking git: this runs per staged file during a
/// release, and a subprocess each time would cost more than the copy it is
/// deciding about.
fn is_build_output(source: &Path, root: &Path) -> bool {
    // Resolve both sides before comparing: a relative source, a symlinked
    // checkout, and `/var` vs `/private/var` on macOS all defeat a plain
    // `strip_prefix`, and each of them defeating it means "link the source
    // tree into the release".
    let (Ok(source), Ok(root)) = (absolute(source), absolute(root)) else {
        return false;
    };
    let Ok(relative) = source.strip_prefix(&root) else {
        return false;
    };
    matches!(
        relative.components().next().and_then(|c| c.as_os_str().to_str()),
        Some("cache")
    )
}

fn absolute(path: &Path) -> std::io::Result<std::path::PathBuf> {
    // `canonicalize` needs the path to exist, which a destination's parent may
    // not; every caller here passes an existing source, and the fallback keeps
    // the answer conservative rather than wrong.
    path.canonicalize().or_else(|error| {
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            std::env::current_dir().map(|cwd| cwd.join(path)).map_err(|_| error)
        }
    })
}

/// How many names this file has. Exposed so callers and tests can assert the
/// property rather than infer it.
pub fn links(path: &Path) -> Result<u64> {
    Ok(fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .nlink())
}

#[cfg(test)]
mod tests;
