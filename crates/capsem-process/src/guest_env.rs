//! Consume the service's one-use guest environment handoff before confinement.

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const MAX_GUEST_ENV_BYTES: u64 = 1024 * 1024;
pub(crate) const FILE_NAME: &str = "guest-env.json";

pub(crate) fn load(session_dir: &Path) -> Result<Vec<(String, String)>> {
    let path = session_dir.join(FILE_NAME);
    let file = capsem_foundation::unix::fs::open_regular_file_no_follow(&path)
        .with_context(|| format!("open private guest environment {}", path.display()))?;
    let metadata = file.metadata()?;
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!("guest environment file has group or other access");
    }
    if metadata.len() > MAX_GUEST_ENV_BYTES {
        bail!("guest environment file exceeds size limit");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_GUEST_ENV_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_GUEST_ENV_BYTES {
        bail!("guest environment file exceeds size limit");
    }
    let env: Vec<(String, String)> = serde_json::from_slice(&bytes).context("parse guest environment")?;
    std::fs::remove_file(&path).context("remove one-use guest environment")?;
    Ok(env)
}

#[cfg(test)]
mod tests;
