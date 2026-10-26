//! The local diagnostic upstream and its shared listener lease.

use anyhow::{anyhow, Context, Result};
use std::{
    io::BufRead,
    path::{Path, PathBuf},
    process::{Child, Command as StdCommand, Stdio},
    time::{Duration, Instant},
};

pub(crate) const DOCTOR_MOCK_SERVER_ADDR: &str = "127.0.0.1:3713";
const DOCTOR_MOCK_SERVER_LOCK_TIMEOUT: Duration = Duration::from_secs(600);

struct DoctorMockServerLock {
    _lock: capsem_foundation::unix::lock::FileLock,
}

impl DoctorMockServerLock {
    fn path_for_addr(addr: &str) -> PathBuf {
        let safe_addr = addr.replace([':', '.'], "-");
        // Listening ports remain shared when diagnostics have private TMPDIRs.
        // The harness supplies its cache-owned lock directory to both launchers.
        let directory = std::env::var_os("CAPSEM_MOCK_SERVER_LOCK_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        directory.join(format!("capsem-mock-server-{safe_addr}.lock"))
    }

    fn acquire(addr: &str, timeout: Duration) -> Result<Self> {
        let path = Self::path_for_addr(addr);
        let deadline = Instant::now() + timeout;
        loop {
            match capsem_foundation::unix::lock::try_acquire(&path, capsem_foundation::unix::lock::LockMode::Exclusive)
                .with_context(|| format!("acquire mock-server lock {}", path.display()))?
            {
                capsem_foundation::unix::lock::LockAttempt::Acquired(lock) => return Ok(Self { _lock: lock }),
                capsem_foundation::unix::lock::LockAttempt::Contended => {
                    if Instant::now() >= deadline {
                        anyhow::bail!("timed out waiting for mock-server lock {}", path.display());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

pub(crate) struct DoctorMockServer {
    _lock: DoctorMockServerLock,
    child: Child,
    base_url: String,
}

impl DoctorMockServer {
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DoctorMockServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn find_mock_server_binary(executable: &Path, current_dir: &Path, manifest_dir: &Path) -> Option<PathBuf> {
    if let Some(bin_dir) = executable.parent() {
        let installed_candidate = bin_dir.join("capsem-mock-server");
        if installed_candidate.exists() {
            return Some(installed_candidate);
        }
    }

    let cwd_candidate = current_dir.join("cache/target/cargo/debug/capsem-mock-server");
    if cwd_candidate.exists() {
        return Some(cwd_candidate);
    }

    let manifest_candidate = manifest_dir.join("../../cache/target/cargo/debug/capsem-mock-server");
    if manifest_candidate.exists() {
        return Some(manifest_candidate);
    }

    None
}

fn mock_server_binary_path() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("resolve current Capsem executable")?;
    let current_dir = std::env::current_dir().context("read current directory")?;
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    find_mock_server_binary(&executable, &current_dir, &manifest_dir).ok_or_else(|| {
        anyhow!(
            "capsem-mock-server not found beside the Capsem executable or under cache/target/cargo/debug; \
         reinstall Capsem or run cargo build -p capsem-mock-server"
        )
    })
}

fn doctor_mock_server_command(binary: &Path) -> StdCommand {
    let mut command = StdCommand::new(binary);
    command
        .arg("--addr")
        .arg(DOCTOR_MOCK_SERVER_ADDR)
        .arg("--parent-pid")
        .arg(std::process::id().to_string());
    command
}

pub(crate) fn spawn_doctor_mock_server() -> Result<DoctorMockServer> {
    let lock = DoctorMockServerLock::acquire(DOCTOR_MOCK_SERVER_ADDR, DOCTOR_MOCK_SERVER_LOCK_TIMEOUT)?;
    let binary = mock_server_binary_path()?;
    let mut child = doctor_mock_server_command(&binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("start {}", binary.display()))?;

    let stdout = child.stdout.take().context("mock server stdout must be piped")?;
    let mut reader = std::io::BufReader::new(stdout);
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).context("read mock server ready JSON")?;
    if bytes == 0 {
        let status = child.try_wait().context("read mock server status")?;
        return Err(anyhow!("mock server exited before ready JSON; status={status:?}"));
    }

    let ready: serde_json::Value = serde_json::from_str(&line).context("parse mock server ready JSON")?;
    if ready.get("service").and_then(serde_json::Value::as_str) != Some("capsem-mock-server") {
        child.kill().ok();
        return Err(anyhow!("unexpected mock server ready payload: {line}"));
    }
    let base_url = ready
        .get("base_url")
        .and_then(serde_json::Value::as_str)
        .context("mock server ready JSON missing base_url")?
        .to_string();

    Ok(DoctorMockServer {
        _lock: lock,
        child,
        base_url,
    })
}

#[cfg(test)]
mod tests;
