//! Local gateway discovery shared by CLI SDK requests and the terminal UI.

use anyhow::{anyhow, Context, Result};
use std::{path::Path, time::Duration};

pub(crate) async fn gateway_url_from_runtime(run_dir: &Path) -> Option<Result<String>> {
    let port_path = run_dir.join("gateway.port");
    let raw = match tokio::fs::read_to_string(&port_path).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => return Some(Err(error).with_context(|| format!("read {}", port_path.display()))),
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let port = match raw.parse::<u16>() {
        Ok(0) => return Some(Err(anyhow!("{} contains port 0", port_path.display()))),
        Ok(port) => port,
        Err(error) => {
            return Some(Err(error).with_context(|| format!("parse gateway port from {}", port_path.display())))
        }
    };
    Some(Ok(format!("http://127.0.0.1:{port}")))
}

pub(crate) async fn wait_for_gateway_url(run_dir: &Path) -> Result<String> {
    match std::env::var("CAPSEM_GATEWAY_URL") {
        Ok(url) => {
            let url = url.trim_end_matches('/');
            anyhow::ensure!(!url.is_empty(), "CAPSEM_GATEWAY_URL must not be empty");
            return Ok(url.to_string());
        }
        Err(std::env::VarError::NotPresent) => {}
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(anyhow!("CAPSEM_GATEWAY_URL must be UTF-8"));
        }
    }

    let run_dir = run_dir.to_path_buf();
    match capsem_foundation::poll::poll_until(
        capsem_foundation::poll::PollOpts::new("gateway-runtime-ready", Duration::from_secs(5)),
        || {
            let run_dir = run_dir.clone();
            async move { gateway_url_from_runtime(&run_dir).await }
        },
    )
    .await
    {
        Ok(result) => result,
        Err(timeout) => Err(anyhow!(
            "Capsem gateway did not publish {} within {:?}",
            run_dir.join("gateway.port").display(),
            timeout.timeout
        )),
    }
}

#[cfg(test)]
mod tests;
