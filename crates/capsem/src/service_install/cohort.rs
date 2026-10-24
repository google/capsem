//! Retire only helpers whose pinned kernel identity names this installation.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use capsem_foundation::unix::process::{ProcessHandle, ProcessId, Signal};

const HELPER_NAMES: &[&str] = &[
    "capsem-service",
    "capsem-gateway",
    "capsem-tray",
    "capsem-process",
    "capsem-ledger",
    "capsem-proxy",
    "capsem-router",
    "capsem-port-router",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
];

pub(crate) async fn retire_installed_helpers(bin_dir: &Path) -> Result<()> {
    let bin_dir = std::fs::canonicalize(bin_dir)
        .with_context(|| format!("resolve installed binary directory {}", bin_dir.display()))?;
    let uid = capsem_foundation::unix::process::current_uid().to_string();
    // pgrep only supplies candidates. Neither its regular expression nor a
    // process command line authorizes a signal.
    let output = tokio::process::Command::new("pgrep")
        .args(["-u", &uid, "-f", "capsem-"])
        .output()
        .await
        .context("list possible Capsem helpers")?;
    if output.status.code() == Some(1) {
        return Ok(());
    }
    if !output.status.success() {
        bail!("process discovery failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let mut retired = Vec::new();
    for line in std::str::from_utf8(&output.stdout)
        .context("non-UTF-8 process id list")?
        .lines()
    {
        let Ok(pid) = line.trim().parse::<u32>() else { continue };
        let Ok(pid) = ProcessId::try_from(pid) else { continue };
        let Ok(handle) = ProcessHandle::open(pid) else { continue };
        let Ok(executable) = handle.executable_path() else {
            continue;
        };
        let Some(name) = executable.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !HELPER_NAMES.contains(&name) || executable != bin_dir.join(name) {
            continue;
        }
        handle
            .signal(Signal::Kill)
            .with_context(|| format!("stop installed {name} pid {}", pid.get()))?;
        retired.push(handle);
    }

    let stopped = tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        for handle in retired {
            if !handle.wait_until_exit(deadline)? {
                return Err(std::io::Error::other(format!(
                    "installed helper pid {} did not stop",
                    handle.pid().get()
                )));
            }
        }
        Ok::<_, std::io::Error>(())
    })
    .await
    .context("wait for retired helpers")?;
    stopped.context("wait for installed helpers to stop")
}
