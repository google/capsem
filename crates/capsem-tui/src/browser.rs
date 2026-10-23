//! Browser forwarding terminates on the TUI client's machine, including when
//! its gateway is remote. Never execute a shell or a guest-provided program.
use crate::gateway_provider::ActionOutcome;
use anyhow::{Context, Result};
use capsem_sdk::models::BrowserRequest;

#[derive(Debug, Default)]
pub(crate) struct Callbacks(tokio::sync::Mutex<Option<capsem_sdk::browser::BrowserCallback>>);

impl Callbacks {
    pub(crate) async fn close(&self) -> Result<()> {
        let current = self.0.lock().await.take();
        if let Some(current) = current {
            current.close().await?;
        }
        Ok(())
    }
}

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

pub(crate) async fn open(
    request: &BrowserRequest,
    vm: &capsem_sdk::VM,
    callbacks: &Callbacks,
) -> Result<ActionOutcome> {
    let command = command(request)?;
    let mut current = callbacks.0.lock().await;
    if let Some(previous) = current.take() {
        previous.close().await.context("close previous browser callback")?;
    }
    let prepared = capsem_sdk::browser::BrowserCallback::prepare(vm, &request.url)
        .await
        .context("prepare browser callback")?;
    let status = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::from(command).kill_on_drop(true).status(),
    )
    .await
    .context("browser opener timed out")?
    .context("could not start browser opener")?;
    anyhow::ensure!(status.success(), "browser opener failed");
    *current = prepared;
    drop(current);
    Ok(ActionOutcome {
        message: "opened browser on this device".into(),
        focus_session: None,
        approval_requests: None,
    })
}

#[cfg(test)]
mod tests;
