use std::time::Duration;

use anyhow::{ensure, Context, Result};
use capsem_assets::oci::{ImageLayout, Puller, RegistryAuth};
use capsem_core::container;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use tokio::io::AsyncWriteExt;

use crate::client::{
    self, ApiResponse, ExecResponse, ListResponse, ProvisionRequest, ProvisionResponse, RunRequest, UdsClient,
};

mod upload;

#[derive(clap::Args)]
pub(super) struct RunArgs {
    /// Shell command, docker://IMAGE, or qualified registry/repository:tag
    pub command: String,
    #[arg(long, default_value = crate::DEFAULT_PROFILE_ID)]
    pub profile: String,
    /// Maximum workload duration in seconds
    #[arg(long)]
    pub timeout: Option<u64>,
    /// Environment variables (container-only when running an image)
    #[arg(short = 'e', long = "env")]
    pub env: Vec<String>,
    /// Container VM name; otherwise derived from the image
    #[arg(short = 'n', long)]
    pub name: Option<String>,
    /// Publish loopback HOST_PORT:GUEST_PORT over VSOCK (host 0 picks a port)
    #[arg(short = 'p', long = "publish")]
    pub publish: Vec<container::PortMapping>,
    /// Additional PEM certificate trusted only for this registry pull
    #[arg(long)]
    pub registry_ca: Option<std::path::PathBuf>,
    /// Registry user; password/token comes from CAPSEM_REGISTRY_PASSWORD
    #[arg(long)]
    pub registry_user: Option<String>,
    /// Replace the image's Cmd, preserving its Entrypoint
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

pub(super) async fn run(client: &UdsClient, args: &RunArgs) -> Result<i32> {
    client::validate_id(&args.profile)?;
    let Some(base) = container::image_name(&args.command)? else {
        ensure!(
            args.name.is_none()
                && args.args.is_empty()
                && args.registry_ca.is_none()
                && args.registry_user.is_none()
                && args.publish.is_empty(),
            "container options require an OCI image"
        );
        let request = RunRequest {
            command: args.command.clone(),
            profile_id: args.profile.clone(),
            timeout_secs: args.timeout,
            env: client::parse_env_vars(&args.env)?,
        };
        let response: ApiResponse<ExecResponse> = client.post("/run", request).await?;
        let response = response.into_result()?;
        tokio::io::stdout().write_all(response.stdout.as_bytes()).await?;
        tokio::io::stderr().write_all(response.stderr.as_bytes()).await?;
        if let Some(notice) = response.truncation_notice() {
            eprintln!("{notice}");
        }
        return Ok(response.exit_code);
    };
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let cancel = async {
        tokio::select! { _ = interrupt.recv() => 130, _ = terminate.recv() => 143 }
    };
    tokio::pin!(cancel);
    let architecture = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => anyhow::bail!("unsupported container architecture: {other}"),
    };
    let staging = tempfile::tempdir()?;
    let authentication = match &args.registry_user {
        Some(user) => RegistryAuth::Basic(
            user.clone(),
            std::env::var("CAPSEM_REGISTRY_PASSWORD").context("--registry-user requires CAPSEM_REGISTRY_PASSWORD")?,
        ),
        None => RegistryAuth::Anonymous,
    };
    let certificate = match &args.registry_ca {
        Some(path) => Some(tokio::fs::read(path).await.context("read registry CA")?),
        None => None,
    };
    let puller = Puller::new_with_root_certificate(architecture, authentication, certificate.as_deref())?;
    eprintln!("Pulling {}", args.command);
    let image = tokio::select! {
        result = puller.pull(&args.command, staging.path()) => result?,
        code = &mut cancel => return Ok(code),
    };
    eprintln!("Image {}", image.source_digest);
    let response: ApiResponse<ListResponse> = client.get("/vms/list").await?;
    let existing: Vec<_> = response
        .into_result()?
        .sessions
        .into_iter()
        .filter_map(|vm| vm.name)
        .collect();
    let name = match &args.name {
        Some(name) => {
            client::validate_id(name)?;
            name.clone()
        }
        None => container::available_name(&base, &existing)?,
    };
    let request = ProvisionRequest {
        name: Some(name.clone()),
        profile_id: args.profile.clone(),
        ram_mb: 4096,
        cpus: 4,
        persistent: false,
        env: None,
        from: None,
    };
    // Keep the create request alive until it returns the authoritative VM id.
    // Signals are already registered and remain queued while boot completes.
    let response: ApiResponse<ProvisionResponse> = client.post("/vms/create", request).await?;
    let vm = response.into_result()?;
    eprintln!("Running {name} ({})", vm.id);
    let work = async {
        // Create may return at the launch signal, before guest boot finishes.
        // The existing exec route owns readiness and its transport deadline.
        let ready: ApiResponse<ExecResponse> = client
            .post(
                &format!("/vms/{}/exec", vm.id),
                client::ExecRequest {
                    command: "true".into(),
                    timeout_secs: Some(30),
                },
            )
            .await?;
        let ready = ready.into_result()?;
        ensure!(ready.exit_code == 0, "guest readiness check failed: {}", ready.stderr);
        upload::image(client, &vm.id, &image, args).await?;
        stream(&vm, &args.publish).await
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
    let cleanup: Result<ApiResponse<serde_json::Value>> = client.delete(&format!("/vms/{}/delete", vm.id)).await;
    match (result, cleanup.and_then(ApiResponse::into_result)) {
        (Ok(code), Ok(_)) => Ok(code),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) => Err(error.context("container VM cleanup failed")),
        (Err(error), Err(cleanup)) => Err(error.context(format!("container VM cleanup also failed: {cleanup:#}"))),
    }
}

async fn stream(vm: &ProvisionResponse, ports: &[container::PortMapping]) -> Result<i32> {
    let path = vm
        .uds_path
        .as_ref()
        .context("service did not return the VM IPC socket")?;
    let socket = tokio::net::UnixStream::connect(path).await?.into_std()?;
    let (socket, _) = capsem_foundation::ipc_handshake::negotiate_initiator_off_worker(
        socket,
        "capsem-cli",
        capsem_foundation::telemetry::current_parent_traceparent(),
    )
    .await?;
    let (sender, receiver) = tokio_unix_ipc::channel_from_std::<ServiceToProcess, ProcessToService>(socket)?;
    for mapping in ports {
        sender
            .send(ServiceToProcess::PublishPort {
                id: 0,
                host_port: mapping.host,
                guest_port: mapping.guest,
            })
            .await?;
        loop {
            if let ProcessToService::PortPublished {
                id: 0,
                host_port,
                router_pid,
                error,
            } = receiver.recv().await?
            {
                if let Some(error) = error {
                    anyhow::bail!("publish port: {error}");
                }
                eprintln!(
                    "Published 127.0.0.1:{host_port} -> {}/tcp (router {router_pid})",
                    mapping.guest
                );
                break;
            }
        }
    }
    // Service job ids count upward; this private attached command has its own
    // connection and uses the reserved high end, with duplicate refusal in process.
    let id = i64::MAX as u64;
    sender
        .send(ServiceToProcess::ExecStream {
            id,
            command: container::LAUNCH_COMMAND.into(),
        })
        .await?;
    let mut stdout = tokio::io::stdout();
    loop {
        match receiver.recv().await? {
            ProcessToService::ExecOutput { id: job, data } if job == id => {
                stdout.write_all(&data).await?;
                stdout.flush().await?;
            }
            ProcessToService::ExecResult {
                id: job,
                exit_code,
                stderr,
                truncated,
                ..
            } if job == id => {
                tokio::io::stderr().write_all(&stderr).await?;
                ensure!(!truncated && exit_code >= 0, "container exec transport failed");
                return Ok(exit_code);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
