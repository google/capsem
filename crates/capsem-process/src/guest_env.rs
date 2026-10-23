//! Consume the service's one-use guest environment handoff before confinement.

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(crate) use capsem_proto::guest_env::FILE_NAME;
use capsem_proto::guest_env::{GuestEnvironment, MAX_BYTES};

pub(crate) fn load(session_dir: &Path) -> Result<Vec<(String, String)>> {
    let path = session_dir.join(FILE_NAME);
    let file = capsem_foundation::unix::fs::open_regular_file_no_follow(&path)
        .with_context(|| format!("open private guest environment {}", path.display()))?;
    let metadata = file.metadata()?;
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!("guest environment file has group or other access");
    }
    if metadata.len() > MAX_BYTES as u64 {
        bail!("guest environment file exceeds size limit");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        bail!("guest environment file exceeds size limit");
    }
    let env: GuestEnvironment =
        capsem_foundation::ipc_channel::decode_record(&bytes, MAX_BYTES).context("parse guest environment")?;
    std::fs::remove_file(&path).context("remove one-use guest environment")?;
    Ok(env.values)
}

#[cfg(test)]
mod tests;
