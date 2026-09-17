use std::path::Path;

/// Calculate total actual disk usage in bytes for all entries under the given base path.
///
/// Uses `symlink_metadata` to avoid following symlinks (prevents infinite recursion
/// from symlink loops). Reports actual allocated blocks (`blocks * 512`) instead of
/// logical file size, so sparse files (e.g. a 2GB rootfs.img overlay with 9MB of
/// actual changes) report their true disk footprint.
pub fn disk_usage_bytes(sessions_base: &Path) -> u64 {
    let entries = match std::fs::read_dir(sessions_base) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            total += dir_size(&path);
        } else {
            total += file_disk_usage(&meta);
        }
    }
    total
}

fn dir_size(path: &Path) -> u64 {
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let p = entry.path();
        let meta = match std::fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            total += dir_size(&p);
        } else {
            total += file_disk_usage(&meta);
        }
    }
    total
}

/// Actual disk usage for a file: allocated blocks * 512 bytes.
/// Sparse files report only the blocks actually written to disk.
#[cfg(unix)]
fn file_disk_usage(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn file_disk_usage(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

#[cfg(test)]
mod tests;
