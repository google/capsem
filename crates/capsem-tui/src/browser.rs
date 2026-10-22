//! Browser forwarding terminates on the TUI client's machine, including when
//! its gateway is remote. Never execute a shell or a guest-provided program.
use crate::gateway_provider::ActionOutcome;
use anyhow::{Context, Result};
use capsem_sdk::models::BrowserRequest;

fn command(request: &BrowserRequest) -> Result<std::process::Command> {
    let url = request.validated_url().map_err(anyhow::Error::msg)?;
    let mut command = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    Ok(command)
}

pub(crate) async fn open(request: &BrowserRequest) -> Result<ActionOutcome> {
    let status = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::from(command(request)?)
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("browser opener timed out")?
    .context("could not start browser opener")?;
    anyhow::ensure!(status.success(), "browser opener failed");
    Ok(ActionOutcome {
        message: "opened browser on this device".into(),
        focus_session: None,
    })
}

#[cfg(test)]
mod tests;
