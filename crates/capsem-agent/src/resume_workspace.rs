//! One bounded directory read verifies the resumed workspace transport.
use std::io;
use std::path::Path;

pub(crate) fn enumerate(path: &Path) -> io::Result<()> {
    first_entry(std::fs::read_dir(path)?)
}

fn first_entry<T>(mut entries: impl Iterator<Item = io::Result<T>>) -> io::Result<()> {
    // One READDIR result is the existing bounded transport probe. An empty
    // successful read is healthy; an iteration error is not an entry.
    entries.next().transpose().map(|_| ())
}

pub(crate) fn ready_after(
    thaw: impl FnOnce() -> io::Result<()>,
    rebind: impl FnOnce() -> io::Result<()>,
    ready: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    thaw()?;
    rebind()?;
    ready()
}

/// Reconnect the agent's own workspace mount after host owner replacement.
/// This does not repair existing workload-private mounts or held descriptors.
pub(crate) fn rebind() -> io::Result<()> {
    eprintln!("[capsem-agent] rebinding workspace after resume");
    rebind_steps(run, || {
        warm_with(
            || enumerate(Path::new("/mnt/shared/workspace")),
            || std::thread::sleep(std::time::Duration::from_millis(20)),
        )
    })
}

fn run(arguments: &[&str]) -> io::Result<()> {
    let status = std::process::Command::new(arguments[0])
        .args(&arguments[1..])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("workspace {} exited {status}", arguments[0])))
    }
}

fn warm_with(mut probe: impl FnMut() -> io::Result<()>, mut pause: impl FnMut()) -> io::Result<usize> {
    let mut last_error = None;
    for attempt in 1..=50 {
        match probe() {
            Ok(()) => return Ok(attempt),
            Err(error) => last_error = Some(error),
        }
        pause();
    }
    Err(last_error.expect("the fixed nonempty probe budget records its last failure"))
}

fn rebind_steps(
    mut run: impl FnMut(&[&str]) -> io::Result<()>,
    warm: impl FnOnce() -> io::Result<usize>,
) -> io::Result<()> {
    // These stale mounts may be absent on the first reconnect. Required
    // mount creation, enumeration and final bind follow the cleanup.
    let _ = run(&["umount", "-l", "/root"]);
    let _ = run(&["umount", "-l", "/mnt/shared"]);
    run(&["mkdir", "-p", "/mnt/shared"])?;
    run(&["mount", "-t", "virtiofs", "capsem", "/mnt/shared"])?;
    let attempts = warm()?;
    eprintln!("[capsem-agent] rebind: virtiofs warmed after {attempts} attempts");
    run(&["mkdir", "-p", "/root"])?;
    run(&["mount", "--bind", "/mnt/shared/workspace", "/root"])?;
    eprintln!("[capsem-agent] rebind: /root reconnected to host workspace");
    Ok(())
}

#[cfg(test)]
mod tests;
