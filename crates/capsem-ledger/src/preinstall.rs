//! Fresh, private candidate admission; never enters the ordinary ledger worker.
use std::ffi::OsString;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};
use capsem_foundation::unix::{account, process};
use capsem_logger::host_admission::{admit_preinstall_host_state, PREINSTALL_ADMISSION_TIMEOUT};
use clap::Parser;

#[derive(Parser)]
struct Args {
    #[arg(long, required = true, hide = true)]
    preinstall_ledger_admission: bool,
    #[arg(long, hide = true)]
    target_user: String,
}

pub(super) fn requested() -> bool {
    is_requested(std::env::args_os().skip(1))
}

fn is_requested(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter().any(|arg| {
        arg.to_str().is_some_and(|arg| {
            arg == "--preinstall-ledger-admission" || arg.starts_with("--preinstall-ledger-admission=")
        })
    })
}

pub(super) fn run() -> Result<()> {
    let args = Args::parse();
    ensure!(args.preinstall_ledger_admission, "private admission flag required");
    bounded(PREINSTALL_ADMISSION_TIMEOUT, move |deadline| {
        let target = account::by_name(&args.target_user)?.context("preinstall target account does not exist")?;
        validate_identity(&target, process::current_uid(), process::current_effective_uid())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("preinstall admission deadline exceeded")?;
        admit_preinstall_host_state(&target.home, remaining)?;
        Ok(())
    })
}

fn validate_identity(target: &account::Account, current_uid: u32, effective_uid: u32) -> Result<()> {
    ensure!(target.uid != 0, "preinstall target must be non-root");
    ensure!(
        target.uid == current_uid && target.uid == effective_uid,
        "preinstall target must match real and effective current UID"
    );
    ensure!(
        target.home.is_absolute(),
        "preinstall target has no absolute account home"
    );
    Ok(())
}

fn bounded(timeout: Duration, operation: impl FnOnce(Instant) -> Result<()> + Send + 'static) -> Result<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("preinstall deadline overflow")?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    // This is a fresh single-use helper. Never join an NSS/DirectoryServices
    // lookup that may block: returning from main exits the entire process on
    // timeout, releasing its descriptors/locks and terminating this thread.
    // No writer, runtime, telemetry or ordinary worker has been initialized.
    let _operation = std::thread::Builder::new()
        .name("preinstall-admission".into())
        .spawn(move || {
            let _ = sender.send(operation(deadline));
        })
        .context("start bounded preinstall admission")?;
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => bail!("preinstall admission deadline exceeded"),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            bail!("preinstall admission operation stopped before completion")
        }
    }
}

#[cfg(test)]
mod tests;
