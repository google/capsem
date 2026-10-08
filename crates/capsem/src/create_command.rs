//! `capsem create`: a VM, kept when named, and with `--image` an OCI image's
//! workload started detached in it.

use anyhow::Result;

use crate::client::{self, ApiResponse, ForkRequest, ForkResponse, ProvisionRequest, ProvisionResponse, UdsClient};
use crate::container_image::{self, ImageArgs, Workload};
use crate::container_run::ram_mb;
use capsem_api::ContainerState;

#[derive(clap::Args)]
pub(super) struct CreateArgs {
    /// Name for the session (makes it persistent -- "if you name it, you keep it")
    #[arg(short = 'n', long)]
    pub name: Option<String>,
    /// RAM in GB (default: 12)
    #[arg(long)]
    pub ram: Option<u64>,
    /// CPU cores (default: 4)
    #[arg(long)]
    pub cpu: Option<u32>,
    /// Set environment variables (repeatable: -e KEY=VALUE; the container's, with --image)
    #[arg(short = 'e', long = "env")]
    pub env: Vec<String>,
    /// Attach advisory metadata labels at creation (repeatable: -l KEY=VALUE)
    #[arg(short = 'l', long = "label")]
    pub label: Vec<String>,
    /// Clone state from an existing persistent session: its workspace and,
    /// with --image, its image volumes under the new image; without, its image
    #[arg(long)]
    pub from: Option<String>,
    /// Named networks to join (repeatable: --network NAME)
    #[arg(long = "network")]
    pub network: Vec<String>,
    #[command(flatten)]
    pub image: ImageArgs,
}

pub(super) async fn create(client: &UdsClient, args: &CreateArgs) -> Result<()> {
    let persistent = args.name.is_some() || args.from.is_some();
    let workload = Workload::of(&args.image, &args.env)?;
    let labels = client::parse_env_vars(&args.label)?;
    let request = ProvisionRequest {
        name: args.name.clone(),
        ram_mb: ram_mb(args.ram),
        cpus: args.cpu,
        persistent,
        // With an image, the environment is the container's.
        env: match workload {
            None => client::parse_env_vars(&args.env)?,
            Some(_) => None,
        },
        labels,
        from: args.from.clone(),
        networks: args.network.clone(),
        container: match &workload {
            Some(workload) => Some(workload.spec(false).await?),
            None => None,
        },
    };
    let vm = container_image::provision(client, &request).await?;
    if let Some(workload) = &workload {
        start_image(client, &vm, workload).await?;
    }
    if persistent {
        println!("{} (persistent)", vm.id);
    } else {
        println!("{}", vm.id);
    }
    Ok(())
}

pub(super) async fn fork(
    client: &UdsClient,
    session: &str,
    name: &str,
    description: Option<&str>,
    label: &[String],
) -> Result<()> {
    client::validate_id(session)?;
    let session_id = crate::route_ids::resolve_session_route_id(client, session).await?;
    let req = ForkRequest {
        name: name.to_owned(),
        description: description.map(ToOwned::to_owned),
        labels: client::parse_env_vars(label)?,
    };
    let resp: ApiResponse<ForkResponse> = client.post(&format!("/vms/{session_id}/fork"), &req).await?;
    let info = resp.into_result()?;
    let size_mb = info.size_bytes as f64 / 1024.0 / 1024.0;
    println!("Forked session '{}' from '{session}' ({size_mb:.1} MB)", info.name);
    Ok(())
}

/// Follow the service until the workload is launched, then publish its
/// ports. A VM this command could not start is not left behind.
async fn start_image(client: &UdsClient, vm: &ProvisionResponse, workload: &Workload<'_>) -> Result<()> {
    let started = async {
        container_image::follow(client, &vm.id, |state| {
            matches!(state, ContainerState::Starting | ContainerState::Running)
        })
        .await?;
        container_image::expose(client, &vm.id, &workload.image.publish).await
    };
    if let Err(error) = started.await {
        return Err(match container_image::destroy(client, &vm.id).await {
            Ok(()) => error,
            Err(cleanup) => error.context(format!("container VM delete also failed: {cleanup:#}")),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
