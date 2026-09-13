//! `capsem create`: a VM from a profile, kept when named, and with `--image`
//! an OCI image's workload started detached in it.

use anyhow::Result;

use crate::client::{self, ProvisionRequest, ProvisionResponse, UdsClient};
use crate::container_image::{self, ImageArgs, Workload};
use crate::container_run::ram_mb;

#[derive(clap::Args)]
pub(super) struct CreateArgs {
    /// Name for the session (makes it persistent -- "if you name it, you keep it")
    #[arg(short = 'n', long)]
    pub name: Option<String>,
    /// Profile to use for this session
    #[arg(long, default_value = crate::DEFAULT_PROFILE_ID)]
    pub profile: String,
    /// RAM in GB (default: the profile's)
    #[arg(long)]
    pub ram: Option<u64>,
    /// CPU cores (default: the profile's)
    #[arg(long)]
    pub cpu: Option<u32>,
    /// Set environment variables (repeatable: -e KEY=VALUE; the container's, with --image)
    #[arg(short = 'e', long = "env")]
    pub env: Vec<String>,
    /// Clone state from an existing persistent session
    #[arg(long, conflicts_with = "image")]
    pub from: Option<String>,
    /// Named networks to join (repeatable: --network NAME)
    #[arg(long = "network")]
    pub network: Vec<String>,
    #[command(flatten)]
    pub image: ImageArgs,
    /// With --image, the command replacing the image's Cmd
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, requires = "image")]
    pub args: Vec<String>,
}

pub(super) async fn create(client: &UdsClient, args: &CreateArgs) -> Result<()> {
    client::validate_id(&args.profile)?;
    let persistent = args.name.is_some() || args.from.is_some();
    let request = ProvisionRequest {
        name: args.name.clone(),
        profile_id: args.profile.clone(),
        ram_mb: ram_mb(args.ram),
        cpus: args.cpu,
        persistent,
        // With an image, the environment is the container's.
        env: match args.image.image {
            Some(_) => None,
            None => client::parse_env_vars(&args.env)?,
        },
        from: args.from.clone(),
        networks: args.network.clone(),
    };
    let vm = match &args.image.image {
        None => container_image::provision(client, &request).await?,
        Some(reference) => {
            let workload = Workload {
                reference,
                image: &args.image,
                env: &args.env,
                args: &args.args,
            };
            start_image(client, &request, &workload).await?
        }
    };
    if persistent {
        println!("{} (persistent)", vm.id);
    } else {
        println!("{}", vm.id);
    }
    Ok(())
}

/// A VM this command could not start is not left behind half-configured.
async fn start_image(
    client: &UdsClient,
    request: &ProvisionRequest,
    workload: &Workload<'_>,
) -> Result<ProvisionResponse> {
    let pulled = container_image::pull(workload).await?;
    let vm = container_image::provision(client, request).await?;
    let started = async {
        container_image::stage(client, &vm, &pulled, workload).await?;
        container_image::launch_detached(client, &vm).await
    };
    if let Err(error) = started.await {
        return Err(match container_image::destroy(client, &vm.id).await {
            Ok(()) => error,
            Err(cleanup) => error.context(format!("container VM delete also failed: {cleanup:#}")),
        });
    }
    Ok(vm)
}

#[cfg(test)]
mod tests;
