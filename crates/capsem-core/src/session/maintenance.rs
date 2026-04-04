use std::io::Write;
use std::path::Path;

use rusqlite::Connection;

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
        return Err(anyhow::anyhow!("session.db not found in {}", session_dir.display()));
    }

    // Open, checkpoint, vacuum, close.
    {
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        conn.execute_batch("VACUUM")?;
    }

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

/// Calculate total disk usage in bytes for all session directories under the given base path.
///
/// Uses `symlink_metadata` to avoid following symlinks, which prevents infinite
/// recursion from symlink loops (e.g. `.venv/lib64 -> lib`).
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
            total += meta.len();
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
            total += meta.len();
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_usage_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let usage = disk_usage_bytes(tmp.path());
        assert_eq!(usage, 0);
    }

    #[test]
    fn disk_usage_with_files() {
        let tmp = tempfile::tempdir().unwrap();
        let f1 = tmp.path().join("file1.txt");
        std::fs::write(&f1, "hello").unwrap();
        let usage = disk_usage_bytes(tmp.path());
        assert!(usage >= 5);
    }

    #[test]
    fn disk_usage_nested_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.txt"), "data").unwrap();
        let usage = disk_usage_bytes(tmp.path());
        assert!(usage >= 4);
    }

    #[test]
    fn disk_usage_nonexistent_dir() {
        let usage = disk_usage_bytes(Path::new("/nonexistent/path/to/sessions"));
        assert_eq!(usage, 0);
    }

    #[test]
    fn vacuum_missing_db_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let result = vacuum_and_compress_session_db(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn vacuum_real_db() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("session.db");
        // Create a minimal SQLite DB
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE test (id INTEGER PRIMARY KEY)").unwrap();
        conn.execute_batch("INSERT INTO test VALUES (1)").unwrap();
        drop(conn);

        let result = vacuum_and_compress_session_db(tmp.path());
        assert!(result.is_ok());
        let gz_size = result.unwrap();
        assert!(gz_size > 0);
        // Original DB should be removed
        assert!(!db_path.exists());
        // Compressed file should exist
        assert!(tmp.path().join("session.db.gz").exists());
    }
}
