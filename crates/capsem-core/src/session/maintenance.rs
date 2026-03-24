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
pub fn disk_usage_bytes(sessions_base: &Path) -> u64 {
    let entries = match std::fs::read_dir(sessions_base) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += dir_size(&path);
        } else if path.is_file() {
            total += path.metadata().map(|m| m.len()).unwrap_or(0);
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
        if p.is_dir() {
            total += dir_size(&p);
        } else if p.is_file() {
            total += p.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}
