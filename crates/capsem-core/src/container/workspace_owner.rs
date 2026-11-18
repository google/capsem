//! Refresh trusted workload controllers, including Linux VirtioFS ownership.
//!
//! Controls are host-owned on the read-only image share. Descriptor-relative
//! reads refuse links. A changed controller requires a cold boot; callers use
//! the returned flag to reject an old VM checkpoint.

use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
use serde::Deserialize;
use std::io::Read;

const OWNER_POLICY: &str = "image_user";
const CONTROL_FILE_LIMIT: u64 = crate::session::LAUNCH_CONTROL_LIMIT;

/// Refresh an existing workload stage to the current launcher contract.
///
/// A missing share without writable legacy controls is a bare VM. Writable
/// controls are refused, never imported. A missing launcher is repaired only
/// when host-owned options identify a trusted staged workload.
pub fn refresh(session_dir: &std::path::Path) -> Result<bool> {
    let missing = |error: &std::io::Error| error.kind() == std::io::ErrorKind::NotFound;
    let stage = match ContainedDir::open_root(session_dir)
        .and_then(|session| session.descend(crate::session::IMAGE_SHARE_DIR.as_ref()))
    {
        Ok(stage) => stage,
        Err(error) if missing(&error) => return refuse_writable_controls(session_dir).map(|()| false),
        Err(error) => return Err(error).context("open trusted workload controls"),
    };
    let Some(options_bytes) = read_control_file(&stage, "options.json")? else {
        refuse_writable_controls(session_dir)?;
        return Ok(false);
    };
    let mut controllers_current = true;
    for (name, bytes) in super::LAUNCHER_FILES {
        controllers_current &= read_control_file(&stage, name)?.as_deref() == Some(*bytes);
    }
    let mut options = decode_options(&options_bytes)?;
    let object = options
        .as_object_mut()
        .context("staged workload options are not an object")?;
    let current_policy = object.get("workspace_owner").and_then(serde_json::Value::as_str) == Some(OWNER_POLICY);
    let upgrade_owner = cfg!(target_os = "linux") && !current_policy;
    if !upgrade_owner && controllers_current {
        return Ok(false);
    }

    let encoded = if upgrade_owner {
        object.insert("workspace_owner".into(), OWNER_POLICY.into());
        Some(encode_options(&options)?)
    } else {
        None
    };

    // Publish the controller first. A crash before the options replacement
    // leaves the legacy mapping selected; a crash after it leaves a complete
    // new controller contract.
    for (name, bytes) in super::LAUNCHER_FILES {
        crate::session::publish_contained_private(&stage, name, bytes)?;
    }
    if let Some(encoded) = encoded {
        crate::session::publish_contained_private(&stage, "options.json", &encoded)?;
    }
    Ok(true)
}

/// Rebind a trusted saved workload to its actual new owner, preserving its
/// approved variables and runtime configuration. A different revision requires
/// deliberate preparation, never a metadata-only retarget.
pub fn rebind_environment(
    session_dir: &std::path::Path,
    context: &super::stage::EnvironmentLaunchContext,
) -> Result<()> {
    let value = super::stage::environment_value(context)?;
    let stage = ContainedDir::open_root(session_dir)?.descend(crate::session::IMAGE_SHARE_DIR.as_ref())?;
    let bytes = read_control_file(&stage, "options.json")?.context("trusted workload options are missing")?;
    let mut options = decode_options(&bytes)?;
    let previous = super::stage::EnvironmentLaunchContext::deserialize(
        options
            .get("environment")
            .context("trusted workload has no environment context")?,
    )
    .context("invalid saved environment context")?;
    previous.validate()?;
    ensure!(
        previous.environment_id == context.environment_id && previous.revision_id == context.revision_id,
        "trusted workload belongs to another environment revision"
    );
    options["environment"] = value;
    crate::session::publish_launch_control(session_dir, "options.json", &encode_options(&options)?)
}

fn decode_options(bytes: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(bytes).context("parse staged workload options")
}

fn encode_options(options: &serde_json::Value) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(options).context("encode staged workload options")?;
    ensure!(
        bytes.len() as u64 <= CONTROL_FILE_LIMIT,
        "staged workload options exceed the control-file limit"
    );
    Ok(bytes)
}

fn refuse_writable_controls(session_dir: &std::path::Path) -> Result<()> {
    let absent = |error: &std::io::Error| error.kind() == std::io::ErrorKind::NotFound;
    let workspace = match crate::session::open_workspace(session_dir) {
        Ok(workspace) => workspace,
        Err(error) if absent(&error) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let stage = match workspace.descend(super::STAGE.as_ref()) {
        Ok(stage) => stage,
        Err(error) if absent(&error) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        stage.entry_kind("options.json".as_ref())?.is_none(),
        "guest-writable launch controls are unsupported; recreate the workload from its approved environment"
    );
    Ok(())
}

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

#[cfg(test)]
mod tests;
