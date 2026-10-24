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
/// Held descriptors reject other-account path authority; SQLite still opens
/// the validated host path. Atomic containment against deliberately racing
/// replacement by this same account is not a guarantee of this API.
pub fn admit_preinstall_host_schema(home: &Path, timeout: Duration) -> anyhow::Result<HostSchemaAdmission> {
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
    let mut held_files = Vec::new();
    for name in ["host.db-wal", "host.db-shm", "host.db-journal", "main.db"] {
        if let Some(handle) = file(&sessions, name, uid)? {
            ensure!(
                main.is_some() && name != "main.db",
                "partial or retired host ledger state"
            );
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
    let uri = readonly_uri(&sessions.path().join("host.db"));
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
    remaining(deadline)?;
    transaction.commit()?;
    remaining(deadline)?;
    // Keep validated descriptors alive until SQLite closes, including failures.
    drop(conn);
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

#[cfg(test)]
mod tests;
