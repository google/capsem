//! Descriptor-owned durable terminal state; used only during owner quiescence.

use super::{publish_contained_private, OWNER_STATE_DIR};
use crate::terminal::{SessionTerminal, MAX_CHECKPOINT_BYTES};
use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::contained::ContainedDir;
use std::ffi::OsStr;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::sync::Arc;

const DIRECTORY: &str = "terminal";
const FILE: &str = "checkpoint.msgpack";

/// Prepared under the exclusive session-owner lock before confinement. All
/// later I/O stays anchored to this host-only directory, never its pathname.
/// Call save/load/clear on a blocking worker while terminal I/O is quiescent.
pub struct TerminalCheckpointStore(ContainedDir);

impl TerminalCheckpointStore {
    pub fn prepare(session_dir: &Path) -> Result<Self> {
        let session = ContainedDir::open_root(session_dir)?;
        let owner = session.descend_or_create(OsStr::new(OWNER_STATE_DIR), 0o700)?;
        owner.validate_private()?;
        let directory = owner.descend_or_create(OsStr::new(DIRECTORY), 0o700)?;
        directory.validate_private()?;
        directory.sync()?;
        owner.sync()?;
        session.sync()?;
        Ok(Self(directory))
    }

    /// Publish the complete continuation before the VM's completion marker.
    /// Refuse planted/shared files; replacement never mutates an old inode.
    pub fn save(&self, terminal: &SessionTerminal) -> Result<()> {
        match self.0.open_existing_private_append(OsStr::new(FILE)) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("validate previous terminal checkpoint"),
        }
        let bytes = terminal.checkpoint().context("capture terminal continuation")?;
        publish_contained_private(&self.0, FILE, &bytes).context("publish terminal checkpoint")
    }

    /// Missing or invalid state is a failed restore, never an empty terminal.
    /// Construction and validation finish before the caller admits viewers.
    pub fn load(&self, capacity: usize) -> Result<Arc<SessionTerminal>> {
        let file = self
            .0
            .open_existing_private_append(OsStr::new(FILE))
            .context("open terminal checkpoint")?;
        ensure!(
            file.metadata()?.len() <= MAX_CHECKPOINT_BYTES as u64,
            "terminal checkpoint exceeds byte limit"
        );
        let mut bytes = Vec::new();
        file.take(MAX_CHECKPOINT_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        SessionTerminal::from_checkpoint(&bytes, capacity).context("restore terminal continuation")
    }

    pub fn clear(&self) -> Result<()> {
        match self.0.remove_private_file(OsStr::new(FILE)) {
            Ok(()) => self.0.sync().context("sync removed terminal checkpoint"),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("remove terminal checkpoint"),
        }
    }
}

#[cfg(test)]
mod tests;
