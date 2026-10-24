//! Read-only admission of this account's host schema; never a migration.
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context};
use capsem_foundation::unix::{
    contained::{ContainedDir, ContainedOpenOptions},
    fs, process,
};
use rusqlite::{Connection, OpenFlags};

/// Existing reader budget also bounds the complete fresh preinstall invocation.
pub const PREINSTALL_ADMISSION_TIMEOUT: Duration = crate::schema::READER_BUSY_TIMEOUT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSchemaAdmission {
    Absent,
    Current,
}

/// Admit only host schema and archive-state metadata, not physical archives or
/// retained session schemas. The caller supplies an authoritative resolved
/// account home, not ambient HOME. This never creates or repairs state.
///
/// SQLite may create an empty WAL and create/update its SHM coordination index;
/// it cannot migrate schema or write ledger records through this connection.
/// Retired `main.db` files are inert protected files, never opened as SQLite;
/// their existence says nothing about retained session compatibility.
/// Held descriptors reject other-account path authority; SQLite still opens
/// the validated host path. Atomic containment against deliberately racing
/// replacement by this same account is not a guarantee of this API.
pub fn admit_preinstall_host_schema(home: &Path, timeout: Duration) -> anyhow::Result<HostSchemaAdmission> {
    admit(home, timeout, false)
}

fn admit(home: &Path, timeout: Duration, physical_archive: bool) -> anyhow::Result<HostSchemaAdmission> {
    ensure!(
        !timeout.is_zero() && timeout <= crate::schema::READER_BUSY_TIMEOUT,
        "invalid host admission budget"
    );
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("host admission deadline overflow")?;
    let uid = process::current_uid();
    let home = ContainedDir::open_root_no_follow(home)?;
    home.validate_owner_no_external_write(uid)?;
    let Some(capsem) = descend(&home, ".capsem", uid)? else {
        remaining(deadline)?;
        return Ok(HostSchemaAdmission::Absent);
    };
    let Some(sessions) = descend(&capsem, "sessions", uid)? else {
        remaining(deadline)?;
        return Ok(HostSchemaAdmission::Absent);
    };
    let main = file(&sessions, "host.db", uid)?;
    let retired_main = file(&sessions, "main.db", uid)?;
    let retired_present = retired_main.is_some();
    let mut held_files: Vec<File> = retired_main.into_iter().collect();
    for name in ["main.db-wal", "main.db-shm", "main.db-journal"] {
        if let Some(handle) = file(&sessions, name, uid)? {
            ensure!(retired_present, "partial retired main ledger state");
            held_files.push(handle);
        }
    }
    for name in ["host.db-wal", "host.db-shm", "host.db-journal"] {
        if let Some(handle) = file(&sessions, name, uid)? {
            ensure!(main.is_some(), "partial host ledger state");
            held_files.push(handle);
        }
    }
    if physical_archive {
        if let Some(handle) = file(&sessions, "host.db-archive.lock", uid)? {
            ensure!(main.is_some(), "partial host archive lock state");
            held_files.push(handle);
        }
    }
    if main.is_none() {
        // The archive owner names this directory with db_path.with_extension("bodies").
        ensure!(
            descend(&sessions, "host.bodies", uid)?.is_none(),
            "partial host archive state"
        );
        remaining(deadline)?;
        return Ok(HostSchemaAdmission::Absent);
    }
    remaining(deadline)?;
    let db_path = sessions.path().join("host.db");
    // Rotation's acquisition lock precedes the SQLite snapshot, like ordinary
    // captured archive reads. Never create or repair a missing lock.
    let archive_lock = if physical_archive {
        Some(capsem_foundation::unix::lock::acquire_existing_until(
            &crate::writer::archive_lock_path_for_db(&db_path),
            capsem_foundation::unix::lock::LockMode::Shared,
            deadline,
        )?)
    } else {
        None
    };
    let uri = readonly_uri(&db_path);
    let conn = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(remaining(deadline)?)?;
    conn.progress_handler(1_000, Some(move || Instant::now() >= deadline));
    let transaction = conn.unchecked_transaction()?;
    crate::schema::validate_existing_host_schema(&transaction)?;
    let archive_reader = if physical_archive {
        let state = crate::schema::archive_state(&transaction)?;
        let directory = sessions.descend(OsStr::new("host.bodies"))?;
        directory.validate_private()?;
        let name = state.header.generation_id.file_name();
        let descriptor = file(&directory, &name, uid)?.context("selected host archive generation is missing")?;
        // Existing archive owner checks only its bounded header and physical
        // extent here; no body decompression, scanning or tail recovery.
        Some(capsem_archive::BodyLogReader::from_descriptor(
            descriptor,
            state.header,
            state.committed_end,
        )?)
    } else {
        None
    };
    remaining(deadline)?;
    transaction.commit()?;
    remaining(deadline)?;
    // Keep validated descriptors alive until SQLite closes, including failures.
    drop(conn);
    drop(archive_reader);
    drop(archive_lock);
    drop(held_files);
    drop(main);
    Ok(HostSchemaAdmission::Current)
}

fn remaining(deadline: Instant) -> anyhow::Result<Duration> {
    match deadline.checked_duration_since(Instant::now()) {
        Some(duration) if !duration.is_zero() => Ok(duration),
        _ => bail!("host admission deadline exceeded"),
    }
}

fn descend(parent: &ContainedDir, name: &str, uid: u32) -> io::Result<Option<ContainedDir>> {
    match parent.descend(OsStr::new(name)) {
        Ok(dir) => {
            dir.validate_owner_no_external_write(uid)?;
            Ok(Some(dir))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn file(parent: &ContainedDir, name: &str, uid: u32) -> io::Result<Option<File>> {
    match parent.open_single_link_file(OsStr::new(name), ContainedOpenOptions::read_only()) {
        Ok(file) => {
            fs::validate_owner_no_external_write(&file, uid)?;
            Ok(Some(file))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn readonly_uri(path: &Path) -> String {
    use std::fmt::Write;
    let mut uri = String::from("file:");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            write!(uri, "%{byte:02X}").expect("formatting into String cannot fail");
        }
    }
    uri.push_str("?mode=ro");
    uri
}

/// Admit host schema plus its selected physical archive header/generation and
/// committed extent in the same bounded snapshot, under the shared acquisition
/// lock. This does not scan body blocks or admit retained session ledgers.
/// The account/path/SQLite-coordination contract is the schema API's contract.
pub fn admit_preinstall_host_state(home: &Path, timeout: Duration) -> anyhow::Result<HostSchemaAdmission> {
    admit(home, timeout, true)
}

#[cfg(test)]
mod tests;
