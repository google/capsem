//! A ledger written before format v4 is set aside, not opened.
//!
//! Format v4 added `archive_state` and the persisted counter snapshot, and
//! nothing rebuilds a snapshot by scanning rows, so a pre-v4 ledger cannot be
//! opened as one. Refusing it outright stopped every Capsem 0.6.3 install that
//! updated to 0.6.4: the service opens `main.db` at start and crash-looped on
//! the ledger 0.6.3 had written. The old file is kept beside the new one under
//! a `.pre-v4-<seconds>` suffix and a fresh ledger takes its place.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};

use super::bodies::archive_path_for_db;
use crate::schema::{self, ArchiveSchemaStatus};

/// Move `path` and its companions aside when it predates `archive_state`.
/// The caller holds the writer lock. Anything else -- fresh, current, or a
/// malformed current ledger -- is left for the normal open to judge.
pub(super) fn set_aside_pre_archive_ledger(path: &Path) -> rusqlite::Result<Option<PathBuf>> {
    if !path.is_file() {
        return Ok(None);
    }
    let status = {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
        )?;
        schema::archive_schema_status(&conn)?
    };
    if status != ArchiveSchemaStatus::PreArchive {
        return Ok(None);
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let suffix = format!("pre-v4-{seconds}");
    let aside = with_suffix(path, &format!(".{suffix}"));
    for companion in [
        path.to_path_buf(),
        with_suffix(path, "-wal"),
        with_suffix(path, "-shm"),
        archive_path_for_db(path),
    ] {
        if companion.exists() {
            let target = with_suffix(&companion, &format!(".{suffix}"));
            std::fs::rename(&companion, &target).map_err(|error| {
                rusqlite::Error::InvalidParameterName(format!(
                    "set aside pre-v4 ledger {} as {}: {error}",
                    companion.display(),
                    target.display()
                ))
            })?;
        }
    }
    tracing::warn!(
        ledger = %path.display(),
        kept_as = %aside.display(),
        "ledger predates format v4; kept the old file and started a fresh ledger"
    );
    Ok(Some(aside))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests;
