//! An OCI image as a VM's workload, set up by the service: the VM is created
//! with the image, the service pulls, verifies and stages it, and the CLI
//! follows its status, publishes ports through the exposure API, and for
//! `run --image` attaches through the VM stream. The CLI never reaches the
//! VM owner directly.

use std::time::Duration;

use anyhow::{ensure, Context, Result};
use capsem_api::stream::{StreamControl, StreamKind};
use capsem_api::{
    ContainerSpec, ContainerState, ContainerStatusResponse, ExposureAccess, ExposureInfo, ExposureRequest,
    ExposureTarget, RegistryAccess,
};
use capsem_core::container;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::client::{self, ApiResponse, ProvisionRequest, ProvisionResponse, StreamEvent, UdsClient};

/// How often the CLI reads the setup status while the service works.
const STATUS_POLL: Duration = Duration::from_millis(250);

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

    /// The service's container spec. A reference that cannot name an image,
    /// or a registry user without a password, is refused before any VM exists.
    pub(super) async fn spec(&self, attach: bool) -> Result<ContainerSpec> {
        capsem_assets::oci::image_reference(self.reference)
            .context("--image expects docker://IMAGE or registry/repository:tag")?;
        let username = self.image.registry_user.clone();
        let password = match &username {
            Some(_) => Some(
                std::env::var("CAPSEM_REGISTRY_PASSWORD")
                    .context("--registry-user requires CAPSEM_REGISTRY_PASSWORD")?,
            ),
            None => None,
        };
        let ca_pem = match &self.image.registry_ca {
            Some(path) => Some(tokio::fs::read_to_string(path).await.context("read registry CA")?),
            None => None,
        };
        let registry = (username.is_some() || ca_pem.is_some()).then_some(RegistryAccess {
            username,
            password,
            ca_pem,
        });
        Ok(ContainerSpec {
            image: self.reference.to_string(),
            args: self.args.to_vec(),
            env: client::parse_env_vars(self.env)?
                .unwrap_or_default()
                .into_iter()
                .collect(),
            registry,
            attach,
        })
    }
}

pub(super) async fn provision(client: &UdsClient, request: &ProvisionRequest) -> Result<ProvisionResponse> {
    let response: ApiResponse<ProvisionResponse> = client.post("/vms/create", request).await?;
    response.into_result()
}

pub(super) async fn destroy(client: &UdsClient, id: &str) -> Result<()> {
    let response: ApiResponse<serde_json::Value> = client.delete(&format!("/vms/{id}/delete")).await?;
    response.into_result().map(drop)
}

/// Follow the service's setup of VM `id` until its state is one `done`
/// accepts, reporting the pull and the verified digest as they happen. A
/// failed setup is an error carrying the service's reason.
pub(super) async fn follow(
    client: &UdsClient,
    id: &str,
    reference: &str,
    done: impl Fn(ContainerState) -> bool,
) -> Result<ContainerStatusResponse> {
    eprintln!("Pulling {reference}");
    let mut reported_digest = false;
    loop {
        let status: ApiResponse<ContainerStatusResponse> = client.get(&format!("/vms/{id}/container")).await?;
        let status = status.into_result()?;
        if let (false, Some(digest)) = (reported_digest, &status.digest) {
            eprintln!("Image {digest}");
            reported_digest = true;
        }
        match status.state {
            ContainerState::Failed => {
                anyhow::bail!(
                    "container setup failed: {}",
                    status.error.as_deref().unwrap_or("no reason given")
                )
            }
            state if done(state) => return Ok(status),
            _ => tokio::time::sleep(STATUS_POLL).await,
        }
    }
}

/// Publish every `-p` mapping on the host loopback. Exposures belong to the
/// VM owner and outlive this command.
pub(super) async fn expose(client: &UdsClient, id: &str, mappings: &[container::PortMapping]) -> Result<()> {
    for mapping in mappings {
        let request = ExposureRequest {
            guest_port: mapping.guest,
            host_port: mapping.host,
            target: ExposureTarget::Container,
            access: ExposureAccess::LoopbackTcp,
        };
        let exposed: ApiResponse<ExposureInfo> = client.post(&format!("/vms/{id}/exposures"), request).await?;
        let exposed = exposed.into_result().context("publish port")?;
        eprintln!(
            "Published 127.0.0.1:{} -> {}/tcp",
            exposed
                .host_port
                .context("loopback exposure did not return a host port")?,
            exposed.guest_port
        );
    }
    Ok(())
}

/// Start the staged workload attached: its output streams here and its exit
/// code is the container's.
pub(super) async fn attach(client: &UdsClient, id: &str) -> Result<i32> {
    let start = StreamControl::Start {
        kind: StreamKind::Container,
        command: None,
    };
    let mut attached = client.open_stream(id, start).await.context("attach container")?;
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut stderr = tokio::io::stderr();
    let mut input = [0_u8; 8192];
    let mut stdin_closed = false;
    loop {
        tokio::select! {
            read = stdin.read(&mut input), if !stdin_closed => {
                let read = read.context("read container stdin")?;
                if read == 0 {
                    stdin_closed = true;
                    attached.close_stdin().await?;
                } else {
                    attached.send_stdin(&input[..read]).await?;
                }
            }
            event = attached.next() => match event? {
                StreamEvent::Output(data) => {
                    stdout.write_all(&data).await?;
                    stdout.flush().await?;
                }
                StreamEvent::ErrorOutput(data) => {
                    stderr.write_all(&data).await?;
                    stderr.flush().await?;
                }
                StreamEvent::Exit { code, truncated } => {
                    ensure!(!truncated && code >= 0, "container exec transport failed");
                    return Ok(code);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
