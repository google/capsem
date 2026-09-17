//! Ledger maintenance the logger crate owns.
//!
//! Core and session code decide when a ledger is compacted or snapshotted; the
//! SQLite work itself stays behind the DB boundary.

use std::path::Path;

/// Clone a whole session ledger -- `session.db` and `session.bodies` -- from
/// one session directory into another.
///
/// The database is written first, with `VACUUM INTO`, and the archive copied
/// after. That order is what makes the copy coherent: the archive is
/// append-only, so every block the snapshotted index references is already in
/// the file when the copy starts, and blocks appended afterwards land past
/// everything the index names. The reverse order could copy an archive that
/// stops short of a block the database went on to reference.
pub fn snapshot_session_ledger(src_dir: &Path, dst_dir: &Path) -> anyhow::Result<()> {
    let src = src_dir.join(SESSION_DB_FILE);
    let dst = dst_dir.join(SESSION_DB_FILE);
    snapshot_session_db(&src, &dst)?;

    let src_archive = crate::writer::archive_path_for_db(&src);
    if !src_archive.exists() {
        return Ok(());
    }
    let dst_archive = crate::writer::archive_path_for_db(&dst);
    std::fs::copy(&src_archive, &dst_archive).map_err(|error| {
        tracing::error!(
            src_archive_path = %src_archive.display(),
            dst_archive_path = %dst_archive.display(),
            operation = "snapshot_copy_archive",
            error = %error,
            "session ledger snapshot failed"
        );
        error
    })?;
    Ok(())
}

const SESSION_DB_FILE: &str = "session.db";

fn snapshot_session_db(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let src_conn = rusqlite::Connection::open_with_flags(
        src,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        tracing::error!(
            src_db_path = %src.display(),
            dst_db_path = %dst.display(),
            operation = "snapshot_open_source",
            error = %error,
            "session db snapshot failed"
        );
        error
    })?;

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            tracing::error!(
                src_db_path = %src.display(),
                dst_db_path = %dst.display(),
                parent_path = %parent.display(),
                operation = "snapshot_create_parent",
                error = %error,
                "session db snapshot failed"
            );
            error
        })?;
    }
    let _ = std::fs::remove_file(dst);
    let escaped = dst.to_string_lossy().replace('\'', "''");
    src_conn
        .execute_batch(&format!("VACUUM INTO '{escaped}';"))
        .map_err(|error| {
            tracing::error!(
                src_db_path = %src.display(),
                dst_db_path = %dst.display(),
                operation = "snapshot_vacuum_into",
                error = %error,
                "session db snapshot failed"
            );
            error
        })?;
    drop(src_conn);

    let dst_conn = rusqlite::Connection::open_with_flags(
        dst,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        tracing::error!(
            src_db_path = %src.display(),
            dst_db_path = %dst.display(),
            operation = "snapshot_open_destination",
            error = %error,
            "session db snapshot failed"
        );
        error
    })?;
    let quick_check: String = dst_conn
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(|error| {
            tracing::error!(
                src_db_path = %src.display(),
                dst_db_path = %dst.display(),
                operation = "snapshot_quick_check",
                error = %error,
                "session db snapshot failed"
            );
            error
        })?;
    if quick_check.eq_ignore_ascii_case("ok") {
        tracing::debug!(
            src_db_path = %src.display(),
            dst_db_path = %dst.display(),
            operation = "snapshot",
            "session db snapshot completed"
        );
        Ok(())
    } else {
        tracing::error!(
            src_db_path = %src.display(),
            dst_db_path = %dst.display(),
            operation = "snapshot_quick_check",
            quick_check,
            "session db snapshot failed"
        );
        anyhow::bail!("cloned session db failed quick_check: {quick_check}")
    }
}
