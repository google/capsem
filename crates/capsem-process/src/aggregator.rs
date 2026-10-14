use anyhow::Result;
use capsem_proto::mcp_aggregator::{write_frame, AggregatorClient};
use std::path::{Path, PathBuf};
use tracing::{error, info};

/// Path to the aggregator's dedicated stderr log within a VM's session
/// directory. Kept out of `process.log` so the parent's JSON tracing
/// stream is not polluted by child text tracing.
pub(crate) fn aggregator_log_path(session_dir: &Path) -> PathBuf {
    session_dir.join("mcp-aggregator.stderr.log")
}

/// Spawn the isolated MCP aggregator subprocess and return its client.
pub(crate) async fn spawn_mcp_aggregator(
    servers: &[capsem_proto::mcp_contracts::McpServerDef],
    session_dir: &Path,
    vm_id: &str,
    trace_id: &str,
    aggregator_bin: PathBuf,
    stderr_file: std::fs::File,
) -> Result<AggregatorClient> {
    let (client, rx) = AggregatorClient::channel(64);
    let log_path = aggregator_log_path(session_dir);
    info!(
        bin = %aggregator_bin.display(),
        servers = servers.len(),
        log = %log_path.display(),
        "spawning MCP aggregator"
    );

    let mut cmd = tokio::process::Command::new(&aggregator_bin);
    // Preserve the service trace context while keeping the root span's
    // explicit trace id authoritative for this child.
    for (key, value) in capsem_foundation::telemetry::child_trace_env(vm_id) {
        cmd.env(key, value);
    }
    cmd.env("CAPSEM_TRACE_ID", trace_id);
    let mut child = cmd
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--lock-path")
        .arg(session_dir.join("mcp-aggregator.lock"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::from(stderr_file))
        .spawn()?;

    let mut child_stdin = child.stdin.take().expect("piped aggregator stdin");
    let child_stdout = child.stdout.take().expect("piped aggregator stdout");
    write_frame(&mut child_stdin, &servers.to_vec()).await?;
    let inflight = capsem_core::mcp::aggregator_driver::spawn(rx, child_stdin, child_stdout);

    tokio::spawn(async move {
        info!("aggregator monitor task started");
        match child.wait().await {
            Ok(status) => info!(status = %status, "aggregator subprocess exited"),
            Err(error) => error!(%error, "failed to wait on aggregator"),
        }
        inflight.close();
    });
    Ok(client)
}

pub(crate) fn resolve_mcp_aggregator_binary(exe_path: &Path) -> Result<PathBuf> {
    let bin_dir = exe_path.parent().unwrap_or(Path::new("."));
    let mut candidates = vec![bin_dir.join("capsem-mcp-aggregator")];
    if bin_dir.file_name().and_then(|name| name.to_str()) == Some("deps") {
        if let Some(target_debug) = bin_dir.parent() {
            candidates.push(target_debug.join("capsem-mcp-aggregator"));
        }
    }
    if let Some(candidate) = candidates.iter().find(|candidate| candidate.exists()) {
        return Ok(candidate.clone());
    }

    let searched = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::bail!("required MCP aggregator binary capsem-mcp-aggregator is missing; searched: {searched}")
}
