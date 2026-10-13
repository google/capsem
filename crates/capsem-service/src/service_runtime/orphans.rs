//! Startup observations select candidates; only freshly verified pinned
//! processes can be signalled or counted as retired.
use anyhow::{ensure, Result};
use capsem_foundation::proctable;
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use anyhow::Context;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use capsem_foundation::proctable::Process;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::{Duration, Instant};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use capsem_foundation::unix::process::{ProcessHandle, ProcessId, Signal};

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct Target {
    handle: ProcessHandle,
    observed: Process,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn pin_candidates(observations: &[Process], root: &Path) -> Result<Vec<Target>> {
    let selected = super::find_orphan_capsem_pids(observations, root);
    let mut pinned = Vec::new();
    for observed in observations {
        if observed.parent_pid == std::process::id()
            || !selected
                .iter()
                .any(|pid| u32::try_from(*pid).ok() == Some(observed.pid))
        {
            continue;
        }
        match ProcessHandle::open(ProcessId::try_from(observed.pid)?) {
            Ok(handle) => pinned.push(Target {
                handle,
                observed: observed.clone(),
            }),
            Err(error) => return Err(error).context("pin original startup process"),
        }
    }
    if pinned.is_empty() {
        return Ok(pinned);
    }
    // Pin every candidate before this one fresh sweep. A process that exited
    // between sweeps cannot redirect any handle to its recycled numeric PID.
    let fresh = proctable::processes().context("verify pinned startup arguments")?;
    let mut verified = Vec::new();
    for target in pinned {
        if target.handle.has_exited()? {
            continue;
        }
        let current = fresh
            .iter()
            .find(|process| process.pid == target.observed.pid)
            .context("pinned startup process identity unavailable")?;
        ensure!(
            current.argv == target.observed.argv && current.parent_pid != std::process::id(),
            "pinned startup process identity changed"
        );
        ensure!(
            original_parent_lost(current)?,
            "original service parent has not retired"
        );
        verified.push(target);
    }
    Ok(verified)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn retire(root: &Path) -> Result<()> {
    let targets = pin_candidates(&proctable::processes()?, root)?;
    if targets.is_empty() {
        return Ok(());
    }
    tracing::warn!(count = targets.len(), "retiring verified original startup processes");
    for target in &targets {
        match target.handle.signal(Signal::Terminate) {
            Ok(_) => {}
            // macOS 14 lacks the guarded native signal API. Its service-owned
            // process has a mandatory parent-loss terminator; wait for actual
            // exit instead of falling back to a recyclable numeric PID.
            Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {}
            Err(error) => return Err(error.into()),
        }
    }
    let grace = Instant::now() + Duration::from_secs(2);
    let mut forced = Vec::new();
    for target in targets {
        if !target.handle.wait_until_exit(grace)? {
            target.handle.signal(Signal::Kill)?;
            forced.push(target);
        }
    }
    let forced_exit = Instant::now() + Duration::from_secs(2);
    for target in forced {
        ensure!(
            target.handle.wait_until_exit(forced_exit)?,
            "original startup process exit unconfirmed"
        );
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn original_parent_lost(process: &Process) -> Result<bool> {
    let mut args = process.argv.iter().skip(1);
    let mut declared = None;
    while let Some(arg) = args.next() {
        if arg == "--parent-pid" {
            ensure!(declared.is_none(), "duplicate original parent argument");
            let value = args.next().context("missing original parent argument")?;
            let parent: u32 = value.to_str().context("invalid original parent argument")?.parse()?;
            ProcessId::try_from(parent)?;
            ensure!(parent != 1, "invalid original service parent");
            declared = Some(parent);
        }
    }
    // New owners carry the original parent through reparenting, including
    // subreapers. Legacy owners without that fact need kernel init ancestry.
    Ok(declared.map_or(process.parent_pid == 1, |parent| parent != process.parent_pid))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn retire(root: &Path) -> Result<()> {
    ensure!(
        super::find_orphan_capsem_pids(&proctable::processes()?, root).is_empty(),
        "startup process retirement requires stable native identity"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
