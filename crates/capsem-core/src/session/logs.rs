//! Diagnostic streams belong to the VM owner's writable state, never its
//! session root or ledger. All accesses below that state use held directories.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::path::Path;

use capsem_foundation::telemetry::{CappedLogWriter, SharedLogWriter, SERIAL_LOG_MAX_BYTES};
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};

use super::OWNER_STATE_DIR;

const LOG_DIR: &str = "logs";
const LOG_NAMES: [&str; 4] = ["serial.log", "serial.1.log", "pty.log", "pty.log.1"];

pub struct SessionLogs(ContainedDir);

impl SessionLogs {
    pub fn open_serial(&self) -> io::Result<SharedLogWriter> {
        Ok(CappedLogWriter::open_in(self.0.try_clone()?, OsStr::new("serial.log"), SERIAL_LOG_MAX_BYTES)?.shared())
    }

    pub fn open_pty(&self) -> io::Result<crate::pty_log::PtyLog> {
        crate::pty_log::PtyLog::open_in(self.0.try_clone()?, OsStr::new("pty.log"))
    }
}

/// Called while the session's exclusive owner lock is held, before confinement
/// or any producer starts. Refuse conflicts before moving a single generation.
pub fn prepare_session_logs(session_dir: &Path) -> io::Result<SessionLogs> {
    let root = ContainedDir::open_root(session_dir)?;
    let owner = root.descend_or_create(OsStr::new(OWNER_STATE_DIR), 0o700)?;
    owner.validate_private()?;
    let directory = owner.descend_or_create(OsStr::new(LOG_DIR), 0o700)?;
    directory.validate_private()?;
    let mut retained = Vec::new();
    for name in LOG_NAMES {
        let name = OsStr::new(name);
        let legacy = open_optional(&root, name)?;
        let current = open_optional(&directory, name)?;
        if legacy.is_some() && current.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "both legacy and owner log generations exist",
            ));
        }
        retained.push((name, legacy, current));
    }
    for (name, legacy, current) in retained {
        if let Some(file) = legacy.as_ref().or(current.as_ref()) {
            capsem_foundation::unix::fs::set_mode(file.as_fd(), 0o600)?;
        }
        if legacy.is_some() {
            root.rename_to(name, &directory, name)?;
        }
    }
    for name in ["serial.log", "pty.log"] {
        let file = directory.open_single_link_file(OsStr::new(name), ContainedOpenOptions::append_create(0o600))?;
        capsem_foundation::unix::fs::set_mode(file.as_fd(), 0o600)?;
    }
    Ok(SessionLogs(directory))
}

fn open_optional(directory: &ContainedDir, name: &OsStr) -> io::Result<Option<File>> {
    match directory.open_single_link_file(name, ContainedOpenOptions::read_only()) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_directory(session_dir: &Path) -> io::Result<ContainedDir> {
    let root = ContainedDir::open_root(session_dir)?;
    match root
        .descend(OsStr::new(OWNER_STATE_DIR))
        .and_then(|owner| owner.descend(OsStr::new(LOG_DIR)))
    {
        Ok(directory) => Ok(directory),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(root),
        Err(error) => Err(error),
    }
}

pub fn read_session_serial_tail(session_dir: &Path, max_bytes: usize) -> io::Result<Option<Vec<u8>>> {
    capsem_foundation::telemetry::read_capped_log_tail_bytes_in(
        &read_directory(session_dir)?,
        OsStr::new("serial.log"),
        max_bytes,
    )
}

/// Oldest retained output first; keystrokes and partial frames are excluded by
/// the shared bounded transcript decoder. Missing files alone mean no output.
pub fn read_session_transcript(session_dir: &Path) -> io::Result<Option<Vec<u8>>> {
    let directory = read_directory(session_dir)?;
    let mut output = None;
    for name in ["pty.log.1", "pty.log"] {
        if let Some(file) = open_optional(&directory, OsStr::new(name))? {
            output
                .get_or_insert_with(Vec::new)
                .extend(crate::pty_log::read_output_file(file)?);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
