//! Host-only spawn identity. Metadata identifies a generation; it does not
//! prove that any process is alive or authorize signalling a PID.

use crate::managed_sessions::VmBinding;
use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const SPAWN_IDENTITY_FILE: &str = "spawn-identity.json";
const RECORD_LIMIT: u64 = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    binding: VmBinding,
}

/// Bounded descriptor-only observation. A missing record is distinct from an
/// invalid record, and callers cannot treat either as live VM ownership.
pub fn read_spawn_identity(session: &ContainedDir) -> Result<Option<VmBinding>> {
    let file = match session.open_file(SPAWN_IDENTITY_FILE.as_ref(), ContainedOpenOptions::read_only()) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.len() <= RECORD_LIMIT,
        "spawn identity exceeds fixed-shape limit"
    );
    let mut bytes = Vec::new();
    file.take(RECORD_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= RECORD_LIMIT, "spawn identity grew during read");
    let record: Record = serde_json::from_slice(&bytes).context("invalid spawn identity")?;
    ensure!(record.schema_version == 1, "unsupported spawn identity schema");
    let binding = VmBinding::new(record.binding.id().to_owned(), record.binding.generation())?;
    Ok(Some(binding))
}

/// Publish before spawning, outside the guest's writable shares. Replacement
/// is atomic and does not modify another handle or hard link's old bytes.
pub fn write_spawn_identity(session: &ContainedDir, binding: &VmBinding) -> Result<()> {
    VmBinding::new(binding.id().to_owned(), binding.generation())?;
    // Do not silently overwrite corruption or a planted link.
    let _previous = read_spawn_identity(session)?;
    let bytes = serde_json::to_vec(&Record {
        schema_version: 1,
        binding: binding.clone(),
    })?;
    let temporary_name = format!(".pending-spawn-{}", uuid::Uuid::new_v4());
    let mut temporary = session.create_new_private_file(temporary_name.as_ref())?;
    let result = (|| -> Result<()> {
        temporary.write_all(&bytes)?;
        temporary.sync_all()?;
        session.rename_to(temporary_name.as_ref(), session, SPAWN_IDENTITY_FILE.as_ref())?;
        session.sync()?;
        Ok(())
    })();
    if result.is_err() {
        match session.remove_non_directory(temporary_name.as_ref()) {
            Ok(()) => session.sync()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    result
}

#[cfg(test)]
mod tests;
