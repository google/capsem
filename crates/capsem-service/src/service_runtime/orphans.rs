//! Startup observations select candidates; only freshly verified pinned
//! processes can be signalled or counted as retired.
use anyhow::{ensure, Result};
use capsem_foundation::proctable;
use std::path::Path;

#[cfg(target_os = "linux")]
use anyhow::Context;
#[cfg(target_os = "linux")]
use capsem_foundation::proctable::Process;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use capsem_foundation::unix::process::{ProcessHandle, ProcessId, Signal};

#[cfg(target_os = "linux")]
struct Target {
    handle: ProcessHandle,
    observed: Process,
}

#[cfg(target_os = "linux")]
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
        verified.push(target);
    }
    Ok(verified)
}

#[cfg(target_os = "linux")]
pub(super) fn retire(root: &Path) -> Result<()> {
    let targets = pin_candidates(&proctable::processes()?, root)?;
    if targets.is_empty() {
        return Ok(());
    }
    tracing::warn!(count = targets.len(), "retiring verified original startup processes");
    for target in &targets {
        target.handle.signal(Signal::Terminate)?;
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

#[cfg(not(target_os = "linux"))]
pub(super) fn retire(root: &Path) -> Result<()> {
    ensure!(
        super::find_orphan_capsem_pids(&proctable::processes()?, root).is_empty(),
        "startup process retirement requires stable native identity"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
