//! Upgrade a staged Linux workload to stable VirtioFS ownership.
//!
//! The workspace is guest-writable, so every read and replacement stays
//! descriptor-relative and refuses links. A changed controller requires a
//! cold boot; callers use the returned flag to reject an old VM checkpoint.

use anyhow::Result;

#[cfg(target_os = "linux")]
use anyhow::{ensure, Context};
#[cfg(target_os = "linux")]
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
#[cfg(target_os = "linux")]
use std::io::Read;

#[cfg(target_os = "linux")]
const OWNER_POLICY: &str = "image_user";
#[cfg(target_os = "linux")]
const CONTROL_FILE_LIMIT: u64 = 16 * 1024 * 1024;

/// Refresh an existing workload stage to the stable-owner launcher contract.
///
/// Missing workspaces and stages are bare VMs. Missing options are incomplete
/// stages and remain the launcher's responsibility. A missing launcher is
/// repaired when valid options identify a staged workload.
pub fn refresh(session_dir: &std::path::Path) -> Result<bool> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = session_dir;
        Ok(false)
    }

    #[cfg(target_os = "linux")]
    refresh_linux(session_dir)
}

#[cfg(target_os = "linux")]
fn refresh_linux(session_dir: &std::path::Path) -> Result<bool> {
    let missing = |error: &std::io::Error| error.kind() == std::io::ErrorKind::NotFound;
    let workspace = match crate::session::open_workspace(session_dir) {
        Ok(workspace) => workspace,
        Err(error) if missing(&error) => return Ok(false),
        Err(error) => return Err(error).context("open workload workspace"),
    };
    let stage = match workspace.descend(super::STAGE.as_ref()) {
        Ok(stage) => stage,
        Err(error) if missing(&error) => return Ok(false),
        Err(error) => return Err(error).context("open workload stage"),
    };
    let Some(options_bytes) = read_control_file(&stage, "options.json")? else {
        return Ok(false);
    };
    let launcher = read_control_file(&stage, "launch.py")?;
    let mut options: serde_json::Value =
        serde_json::from_slice(&options_bytes).context("parse staged workload options")?;
    let object = options
        .as_object_mut()
        .context("staged workload options are not an object")?;
    let current_policy = object.get("workspace_owner").and_then(serde_json::Value::as_str) == Some(OWNER_POLICY);
    if current_policy && launcher.as_deref() == Some(super::LAUNCHER) {
        return Ok(false);
    }

    object.insert("workspace_owner".into(), OWNER_POLICY.into());
    let encoded = serde_json::to_vec(&options).context("encode staged workload options")?;
    ensure!(
        encoded.len() as u64 <= CONTROL_FILE_LIMIT,
        "staged workload options exceed the control-file limit"
    );

    // Publish the controller first. A crash before the options replacement
    // leaves the legacy mapping selected; a crash after it leaves a complete
    // new controller contract.
    crate::session::publish_contained_private(&stage, "launch.py", super::LAUNCHER)?;
    crate::session::publish_contained_private(&stage, "options.json", &encoded)?;
    Ok(true)
}

#[cfg(target_os = "linux")]
fn read_control_file(stage: &ContainedDir, name: &str) -> Result<Option<Vec<u8>>> {
    let file = match stage.open_file(name.as_ref(), ContainedOpenOptions::read_only()) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("open staged {name}")),
    };
    ensure!(
        file.metadata()?.len() <= CONTROL_FILE_LIMIT,
        "staged {name} exceeds the control-file limit"
    );
    let mut bytes = Vec::new();
    file.take(CONTROL_FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= CONTROL_FILE_LIMIT,
        "staged {name} grew during read"
    );
    Ok(Some(bytes))
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
