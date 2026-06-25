use std::io::Write;
use std::path::Path;

/// Checkpoint, vacuum, and gzip-compress a session database.
///
/// 1. Opens `session.db` in the given directory
/// 2. Checkpoints WAL (TRUNCATE mode)
/// 3. VACUUMs the database
/// 4. Closes the connection
/// 5. Gzip-compresses to `session.db.gz`
/// 6. Removes `session.db`, `session.db-wal`, `session.db-shm`
///
/// Returns the compressed file size in bytes.
pub fn vacuum_and_compress_session_db(session_dir: &Path) -> anyhow::Result<u64> {
    let db_path = session_dir.join("session.db");
    if !db_path.exists() {
        return Err(anyhow::anyhow!(
            "session.db not found in {}",
            session_dir.display()
        ));
    }

    capsem_logger::checkpoint_and_vacuum_session_db(&db_path)?;

    // Gzip compress session.db -> session.db.gz.
    let gz_path = session_dir.join("session.db.gz");
    let input = std::fs::read(&db_path)?;
    {
        let gz_file = std::fs::File::create(&gz_path)?;
        let mut encoder = flate2::write::GzEncoder::new(gz_file, flate2::Compression::default());
        encoder.write_all(&input)?;
        encoder.finish()?;
    }

    let compressed_size = std::fs::metadata(&gz_path)?.len();

    // Remove uncompressed files.
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(session_dir.join("session.db-wal"));
    let _ = std::fs::remove_file(session_dir.join("session.db-shm"));

    Ok(compressed_size)
}

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
