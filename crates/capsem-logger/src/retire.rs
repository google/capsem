//! Ledgers older than the archive format are moved aside, never migrated.
//!
//! The format v4 archive refuses a ledger without `archive_state`. For the
//! service's own index, `main.db`, that refusal stopped every service start
//! after an upgrade. The old file is renamed with its WAL and shared-memory
//! sidecars, so nothing is lost and the next open creates a fresh ledger.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};

use crate::schema;

/// Move `path` aside when it predates the archive format, returning where it
/// went. A missing or current ledger is left alone; a ledger in the current
/// format that fails validation is an error, never moved.
pub fn retire_predating_ledger(path: &Path) -> rusqlite::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let predates = schema::predates_archive_format(&conn)?;
    if !predates {
        schema::archive_schema_status(&conn)?;
        return Ok(None);
    }
    drop(conn);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let retired = path.with_file_name(format!("{name}.retired-{stamp}"));
    for sidecar in ["-wal", "-shm"] {
        let from = path.with_file_name(format!("{name}{sidecar}"));
        if from.exists() {
            let to = retired.with_file_name(format!("{name}.retired-{stamp}{sidecar}"));
            std::fs::rename(&from, &to).map_err(|error| io_error(&from, error))?;
        }
    }
    std::fs::rename(path, &retired).map_err(|error| io_error(path, error))?;
    Ok(Some(retired))
}

fn io_error(path: &Path, error: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::InvalidPath(PathBuf::from(format!("{}: {error}", path.display())))
}

#[cfg(test)]
mod tests;
