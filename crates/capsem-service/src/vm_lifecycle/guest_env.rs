//! Private, one-use guest environment handoff to the VM owner.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

const FILE_NAME: &str = "guest-env.json";

pub(super) fn stage(
    session_dir: &Path,
    vm_id: &str,
    vm_name: &str,
    env: Option<&HashMap<String, String>>,
) -> Result<()> {
    let mut pairs = vec![
        ("CAPSEM_VM_ID".to_string(), vm_id.to_string()),
        ("CAPSEM_VM_NAME".to_string(), vm_name.to_string()),
    ];
    if let Some(env) = env {
        pairs.extend(env.iter().map(|(key, value)| (key.clone(), value.clone())));
    }
    let bytes = serde_json::to_vec(&pairs).context("serialize guest environment")?;
    capsem_foundation::unix::fs::atomic_write_private(&session_dir.join(FILE_NAME), &bytes)
        .context("stage private guest environment")?;
    Ok(())
}

#[cfg(test)]
mod tests;
