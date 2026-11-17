//! Preserve incompatible ledger cohorts before applying any writer pragmas.
//! The directory keeps SQLite's database/WAL/SHM names and archive resolution
//! intact. An interrupted move stays explicit and blocks a fresh writer.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use capsem_foundation::unix::fs::{durable_sync_directory, ensure_private_dir};
use rusqlite::{Connection, OpenFlags};

use super::bodies::archive_path_for_db;
use crate::schema::{self, ArchiveSchemaStatus};

fn failure(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

/// The caller holds the writer lock. Current, partial, malformed and future
/// formats are left for validation, never mistaken for a released old cohort.
pub(super) fn set_aside_pre_archive_ledger(path: &Path) -> rusqlite::Result<Option<PathBuf>> {
    let pending = with_suffix(path, ".set-aside-pending");
    if pending.exists() {
        return Err(failure(format!(
            "interrupted ledger preservation at {}; refusing to reset history",
            pending.display()
        )));
    }
    if !path.is_file() {
        return Ok(None);
    }
    let cohort = {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
        )?;
        match schema::archive_schema_status(&conn)? {
            ArchiveSchemaStatus::PreArchive => Some("pre-v4"),
            ArchiveSchemaStatus::Current(_) if schema::is_pre_environment_ledger(&conn)? => Some("pre-environment"),
            _ => None,
        }
    };
    let Some(cohort) = cohort else {
        return Ok(None);
    };
    if cohort == "pre-environment" && !archive_path_for_db(path).is_dir() {
        return Err(failure(
            "pre-environment ledger archive is missing; refusing to reset history",
        ));
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let mut attempt = 0usize;
    let aside = loop {
        let candidate = with_suffix(path, &format!(".{cohort}-{seconds}-{attempt}"));
        if !candidate.exists() {
            break candidate;
        }
        attempt = attempt
            .checked_add(1)
            .ok_or_else(|| failure("ledger preservation suffix exhausted"))?;
    };
    ensure_private_dir(&pending).map_err(|error| failure(format!("create ledger preservation directory: {error}")))?;
    let parent = path.parent().ok_or_else(|| failure("ledger path has no parent"))?;
    durable_sync_directory(parent).map_err(|error| failure(format!("sync ledger preservation directory: {error}")))?;
    // Database last: until every companion is kept, no fresh writer can mistake
    // this for absent state. The durable pending marker covers every crash cut.
    for companion in [
        with_suffix(path, "-wal"),
        with_suffix(path, "-shm"),
        archive_path_for_db(path),
        path.to_path_buf(),
    ] {
        if companion.exists() {
            let name = companion
                .file_name()
                .ok_or_else(|| failure("ledger companion has no file name"))?;
            std::fs::rename(&companion, pending.join(name)).map_err(|error| {
                failure(format!(
                    "preserve ledger {} in {}: {error}",
                    companion.display(),
                    pending.display()
                ))
            })?;
        }
    }
    durable_sync_directory(&pending).map_err(|error| failure(format!("sync preserved ledger: {error}")))?;
    std::fs::rename(&pending, &aside).map_err(|error| failure(format!("publish preserved ledger: {error}")))?;
    durable_sync_directory(parent).map_err(|error| failure(format!("sync preserved ledger publication: {error}")))?;
    let kept = aside.join(path.file_name().ok_or_else(|| failure("ledger has no file name"))?);
    tracing::warn!(ledger = %path.display(), kept_as = %kept.display(), cohort,
        "incompatible ledger cohort preserved; starting a fresh ledger");
    Ok(Some(kept))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests;
