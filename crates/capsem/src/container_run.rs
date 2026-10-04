//! `capsem run`: one command in a VM that exists only for it -- a shell
//! command in a fresh VM, or with `--image` an OCI image's workload.
//! `capsem exec`: one command in a running session, its workload or its VM.

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

use crate::client::{self, ApiResponse, ExecRequest, ExecResponse, ProvisionRequest, RunRequest, UdsClient};
use crate::container_image::{self, ImageArgs, Workload};
use capsem_api::ContainerState;

#[derive(clap::Args)]
pub(super) struct RunArgs {
    /// Shell command (with --image, the command follows the image instead)
    #[arg(conflicts_with = "image")]
    pub command: Option<String>,
    /// Maximum workload duration in seconds
    #[arg(long)]
    pub timeout: Option<u64>,
    /// Environment variables (the container's, with --image)
    #[arg(short = 'e', long = "env")]
    pub env: Vec<String>,
    /// RAM in GB (default: 12)
    #[arg(long)]
    pub ram: Option<u64>,
    /// CPU cores (default: 4)
    #[arg(long)]
    pub cpu: Option<u32>,
    /// Named networks the VM joins at creation (repeatable; with --image)
    #[arg(long = "network")]
    pub network: Vec<String>,
    #[command(flatten)]
    pub image: ImageArgs,
}

#[derive(clap::Args)]
pub(super) struct ExecArgs {
    /// Name or ID of the session
    #[arg(value_name = "SESSION")]
    pub session: String,
    /// Command to execute
    pub command: String,
    /// Timeout in seconds
    #[arg(long)]
    pub timeout: Option<u64>,
    /// Where it runs: `workload` (the default for an image session) or `vm`
    #[arg(long, value_enum)]
    pub target: Option<ExecTarget>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExecTarget {
    Workload,
    Vm,
}

impl From<ExecTarget> for capsem_api::ExecTarget {
    fn from(target: ExecTarget) -> Self {
        match target {
            ExecTarget::Workload => Self::Workload,
            ExecTarget::Vm => Self::Vm,
        }
    }
}

pub(super) async fn exec(client: &UdsClient, args: &ExecArgs) -> Result<i32> {
    client::validate_id(&args.session)?;
    let session_id = crate::route_ids::resolve_session_route_id(client, &args.session).await?;
    let request = ExecRequest {
        command: args.command.clone(),
        timeout_secs: args.timeout,
        target: args.target.map(Into::into),
    };
    let response: ApiResponse<ExecResponse> = client.post(&format!("/vms/{session_id}/exec"), request).await?;
    let response = response.into_result()?;
    write_exec_output(&mut tokio::io::stdout(), &mut tokio::io::stderr(), &response).await?;
    Ok(response.exit_code)
}

pub(super) fn ram_mb(ram_gb: Option<u64>) -> Option<u64> {
    ram_gb.map(|gb| gb.saturating_mul(1024))
}

pub(super) async fn run(client: &UdsClient, args: &RunArgs) -> Result<i32> {
    match Workload::of(&args.image, &args.env)? {
        Some(workload) => run_image(client, args, &workload).await,
        None => {
            anyhow::ensure!(
                args.network.is_empty(),
                "--network needs --image: a shell run joins no network"
            );
            run_command(client, args).await
        }
    }
}

async fn run_command(client: &UdsClient, args: &RunArgs) -> Result<i32> {
    let request = RunRequest {
        command: args.command.clone().context("run needs a shell command, or --image")?,
        timeout_secs: args.timeout,
        ram_mb: ram_mb(args.ram),
        cpus: args.cpu,
        env: client::parse_env_vars(&args.env)?,
    };
    let response: ApiResponse<ExecResponse> = client.post("/run", request).await?;
    let response = response.into_result()?;
    write_exec_output(&mut tokio::io::stdout(), &mut tokio::io::stderr(), &response).await?;
    Ok(response.exit_code)
}

/// Print a command's output before its caller exits with the command's code.
/// Tokio's stdout hands a write to a blocking thread and returns; only the
/// flush waits for it, so without one `process::exit` could discard the output.
pub(super) async fn write_exec_output(
    stdout: &mut (impl tokio::io::AsyncWrite + Unpin),
    stderr: &mut (impl tokio::io::AsyncWrite + Unpin),
    response: &ExecResponse,
) -> Result<()> {
    stdout.write_all(&response.stdout.decode()?).await?;
    stdout.flush().await?;
    stderr.write_all(&response.stderr.decode()?).await?;
    if let Some(notice) = response.truncation_notice() {
        stderr.write_all(format!("{notice}\n").as_bytes()).await?;
    }
    stderr.flush().await?;
    Ok(())
}

/// The image's workload attached, in a VM destroyed however the run ends:
/// its exit, a timeout, a signal, or a failure.
async fn run_image(client: &UdsClient, args: &RunArgs, workload: &Workload<'_>) -> Result<i32> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let cancel = async {
        tokio::select! { _ = interrupt.recv() => 130, _ = terminate.recv() => 143 }
    };
    tokio::pin!(cancel);
    let request = ProvisionRequest {
        name: None,
        ram_mb: ram_mb(args.ram),
        cpus: args.cpu,
        persistent: false,
        env: None,
        from: None,
        networks: args.network.clone(),
        container: Some(workload.spec(true).await?),
    };
    // Signals stay queued while the create request returns the VM to destroy.
    let vm = container_image::provision(client, &request).await?;
    eprintln!("Running {} ({})", vm.name, vm.id);
    let work = async {
        container_image::follow(client, &vm.id, |state| state == ContainerState::Staged).await?;
        container_image::expose(client, &vm.id, &workload.image.publish).await?;
        container_image::attach(client, &vm.id).await
    };
    let deadline = async {
        match args.timeout {
            Some(seconds) => tokio::time::sleep(Duration::from_secs(seconds)).await,
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! {
        result = work => result,
        code = &mut cancel => Ok(code),
        _ = deadline => { eprintln!("Container timed out"); Ok(124) },
    };
    let destroyed = container_image::destroy(client, &vm.id).await;
    match (result, destroyed) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(error.context("container VM delete failed")),
        (Err(error), Err(cleanup)) => Err(error.context(format!("container VM delete also failed: {cleanup:#}"))),
    }
}

#[cfg(test)]
mod tests;
