//! An OCI image as a VM's workload: pulled and verified on the host, the VM
//! provisioned from a profile, the blobs uploaded, and the guest launcher
//! started. `create --image` starts it detached; `run --image` attaches to it
//! and destroys the VM when it ends.

use anyhow::{ensure, Context, Result};
use capsem_assets::oci::{ImageLayout, Puller, RegistryAuth};
use capsem_core::container;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use tokio::io::AsyncWriteExt;

use crate::client::{self, ApiResponse, ExecResponse, ProvisionRequest, ProvisionResponse, UdsClient};

mod upload;

/// The flags that make a VM an image's: shared by `create` and `run`.
#[derive(clap::Args, Debug, Default)]
pub(super) struct ImageArgs {
    /// OCI image (docker://IMAGE or registry/repository:tag) and the command
    /// replacing its Cmd. Everything after the image is that command, as with
    /// `docker run`, so options go before --image.
    #[arg(long, num_args = 1.., allow_hyphen_values = true, value_names = ["IMAGE", "CMD"])]
    pub image: Vec<String>,
    /// Publish loopback HOST_PORT:GUEST_PORT over VSOCK (host 0 picks a port)
    #[arg(short = 'p', long = "publish")]
    pub publish: Vec<container::PortMapping>,
    /// Additional PEM certificate trusted only for this registry pull
    #[arg(long)]
    pub registry_ca: Option<std::path::PathBuf>,
    /// Registry user; password/token comes from CAPSEM_REGISTRY_PASSWORD
    #[arg(long)]
    pub registry_user: Option<String>,
}

/// An image's workload: the reference, the command replacing its Cmd, and
/// the container's environment.
pub(super) struct Workload<'a> {
    pub reference: &'a str,
    pub image: &'a ImageArgs,
    pub env: &'a [String],
    pub args: &'a [String],
}

impl<'a> Workload<'a> {
    /// The workload `--image` names, if it names one. Image-only flags
    /// without an image are refused rather than ignored.
    pub(super) fn of(image: &'a ImageArgs, env: &'a [String]) -> Result<Option<Self>> {
        let Some((reference, args)) = image.image.split_first() else {
            ensure!(
                image.publish.is_empty() && image.registry_ca.is_none() && image.registry_user.is_none(),
                "--publish, --registry-ca and --registry-user need --image"
            );
            return Ok(None);
        };
        Ok(Some(Self {
            reference,
            image,
            env,
            args,
        }))
    }
}

/// A pulled image, its staging directory held for as long as it is uploaded.
pub(super) struct Pulled {
    layout: ImageLayout,
    _staging: tempfile::TempDir,
}

pub(super) async fn pull(workload: &Workload<'_>) -> Result<Pulled> {
    capsem_assets::oci::image_reference(workload.reference)
        .context("--image expects docker://IMAGE or registry/repository:tag")?;
    let architecture = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => anyhow::bail!("unsupported container architecture: {other}"),
    };
    let authentication = match &workload.image.registry_user {
        Some(user) => RegistryAuth::Basic(
            user.clone(),
            std::env::var("CAPSEM_REGISTRY_PASSWORD").context("--registry-user requires CAPSEM_REGISTRY_PASSWORD")?,
        ),
        None => RegistryAuth::Anonymous,
    };
    let certificate = match &workload.image.registry_ca {
        Some(path) => Some(tokio::fs::read(path).await.context("read registry CA")?),
        None => None,
    };
    let puller = Puller::new_with_root_certificate(architecture, authentication, certificate.as_deref())?;
    let staging = tempfile::tempdir()?;
    eprintln!("Pulling {}", workload.reference);
    let layout = puller.pull(workload.reference, staging.path()).await?;
    eprintln!("Image {}", layout.source_digest);
    Ok(Pulled {
        layout,
        _staging: staging,
    })
}

pub(super) async fn provision(client: &UdsClient, request: &ProvisionRequest) -> Result<ProvisionResponse> {
    let response: ApiResponse<ProvisionResponse> = client.post("/vms/create", request).await?;
    response.into_result()
}

/// Wait for the guest, upload the image and its options, and publish its
/// ports. The IPC channel that published them comes back for an attach.
pub(super) async fn stage(
    client: &UdsClient,
    vm: &ProvisionResponse,
    pulled: &Pulled,
    workload: &Workload<'_>,
) -> Result<Channel> {
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
    upload::image(client, &vm.id, &pulled.layout, workload).await?;
    let channel = Channel::open(vm).await?;
    for mapping in &workload.image.publish {
        channel.publish(*mapping).await?;
    }
    Ok(channel)
}

pub(super) async fn destroy(client: &UdsClient, id: &str) -> Result<()> {
    let response: ApiResponse<serde_json::Value> = client.delete(&format!("/vms/{id}/delete")).await?;
    response.into_result().map(drop)
}

/// Start the launcher detached, as a boot of this VM would: its output goes
/// to the console `capsem logs` reads.
pub(super) async fn launch_detached(client: &UdsClient, vm: &ProvisionResponse) -> Result<()> {
    let launched: ApiResponse<ExecResponse> = client
        .post(
            &format!("/vms/{}/exec", vm.id),
            client::ExecRequest {
                command: container::detached_launch_command(),
                timeout_secs: Some(30),
            },
        )
        .await?;
    let launched = launched.into_result()?;
    ensure!(launched.exit_code == 0, "container launch failed: {}", launched.stderr);
    Ok(())
}

/// An IPC connection to the VM's owner.
pub(super) struct Channel {
    sender: capsem_foundation::ipc_channel::Sender<ServiceToProcess>,
    receiver: capsem_foundation::ipc_channel::Receiver<ProcessToService>,
}

impl Channel {
    async fn open(vm: &ProvisionResponse) -> Result<Self> {
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
        let (sender, receiver) =
            capsem_foundation::ipc_channel::channel_from_std::<ServiceToProcess, ProcessToService>(socket)?;
        Ok(Self { sender, receiver })
    }

    /// Publications belong to the VM, not to this connection: they outlive it.
    async fn publish(&self, mapping: container::PortMapping) -> Result<()> {
        self.sender
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
            } = self.receiver.recv().await?
            {
                if let Some(error) = error {
                    anyhow::bail!("publish port: {error}");
                }
                eprintln!(
                    "Published 127.0.0.1:{host_port} -> {}/tcp (router {router_pid})",
                    mapping.guest
                );
                return Ok(());
            }
        }
    }

    /// Run the launcher attached: its output streams here and its exit code
    /// is the container's.
    pub(super) async fn attach(self) -> Result<i32> {
        // Service job ids count upward; this private attached command has its own
        // connection and uses the reserved high end, with duplicate refusal in process.
        let id = i64::MAX as u64;
        self.sender
            .send(ServiceToProcess::ExecStream {
                id,
                command: container::LAUNCH_COMMAND.into(),
            })
            .await?;
        let mut stdout = tokio::io::stdout();
        loop {
            match self.receiver.recv().await? {
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
                    let mut output = tokio::io::stderr();
                    output.write_all(&stderr).await?;
                    output.flush().await?;
                    ensure!(!truncated && exit_code >= 0, "container exec transport failed");
                    return Ok(exit_code);
                }
                _ => {}
            }
        }
    }
}
