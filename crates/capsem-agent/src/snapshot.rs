//! Guest-owned ext4 freeze state survives warm checkpoints and transport reconnects.

use std::io;
use std::sync::Mutex;

pub(crate) const SYSTEM_FS_MOUNT: &str = "/dev/.capsem-system";
// The host captures guest memory only after SnapshotReady, so restored state
// remembers the successful freeze. Ordinary reconnects start and remain false.
static SYSTEM_FS_FROZEN: Mutex<bool> = Mutex::new(false);

pub(crate) fn fsfreeze_command(mode: &'static str) -> std::process::Command {
    let mut command = std::process::Command::new("fsfreeze");
    command.args([mode, SYSTEM_FS_MOUNT]);
    command
}

fn set_system_filesystem_frozen(frozen: bool) -> io::Result<()> {
    transition(&mut SYSTEM_FS_FROZEN.lock().unwrap(), frozen, run_fsfreeze)
}

fn transition(state: &mut bool, frozen: bool, apply: impl FnOnce(bool) -> io::Result<()>) -> io::Result<()> {
    if *state != frozen {
        apply(frozen)?;
        *state = frozen;
    }
    Ok(())
}

fn run_fsfreeze(frozen: bool) -> io::Result<()> {
    let mode = if frozen { "-f" } else { "-u" };
    let status = fsfreeze_command(mode).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "fsfreeze {mode} {SYSTEM_FS_MOUNT} exited with {status}"
        )))
    }
}

pub(crate) fn freeze_system_filesystem() -> io::Result<()> {
    set_system_filesystem_frozen(true)
}

pub(crate) fn thaw_system_filesystem() -> io::Result<()> {
    set_system_filesystem_frozen(false)
}

#[cfg(test)]
mod tests;
