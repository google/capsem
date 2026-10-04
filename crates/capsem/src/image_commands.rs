//! `capsem images`: the image catalog the service offers, and pulls ahead
//! of a create. Names resolve in the service, so this command, `create` and
//! every other client see the same catalog and the same policy.

use std::path::PathBuf;

use anyhow::Result;
use capsem_api::{ImageListResponse, ImagePullRequest, ImagePullResponse};

use crate::client::{ApiResponse, UdsClient};
use crate::container_image::{check_image, registry_access};

#[derive(clap::Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct ImagesArgs {
    #[command(subcommand)]
    pub command: Option<ImageCommands>,
    /// Read the catalog from its registry now, not the service's copy
    #[arg(long)]
    pub refresh: bool,
    /// Output JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum ImageCommands {
    /// Pull an image into the host cache ahead of a create
    Pull {
        /// A catalog name (see `capsem images`), docker://IMAGE, or registry/repository:tag
        image: String,
        /// Additional PEM certificate trusted only for this registry pull
        #[arg(long)]
        registry_ca: Option<PathBuf>,
        /// Registry user; password/token comes from CAPSEM_REGISTRY_PASSWORD
        #[arg(long)]
        registry_user: Option<String>,
    },
}

pub(crate) async fn run(client: &UdsClient, args: &ImagesArgs) -> Result<()> {
    match &args.command {
        None => {
            let path = if args.refresh {
                "/images?refresh=true"
            } else {
                "/images"
            };
            let response: ApiResponse<ImageListResponse> = client.get(path).await?;
            let list = response.into_result()?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                print!("{}", render(&list));
            }
        }
        Some(ImageCommands::Pull {
            image,
            registry_ca,
            registry_user,
        }) => {
            check_image(image)?;
            let request = ImagePullRequest {
                image: image.clone(),
                registry: registry_access(registry_user.as_deref(), registry_ca.as_deref()).await?,
            };
            eprintln!("Pulling {image}");
            let response: ApiResponse<ImagePullResponse> = client.post("/images/pull", &request).await?;
            let pulled = response.into_result()?;
            println!("{}", pulled.resolved);
        }
    }
    Ok(())
}

/// The table `capsem images` prints. A digest is shortened to its first 12
/// hex digits, as `docker images` does; `--json` has every byte.
fn render(list: &ImageListResponse) -> String {
    let Some(catalog) = &list.catalog else {
        return "No image catalog: the image policy turns it off, or it could not be read.\n".into();
    };
    let short = |digest: &str| digest.chars().take("sha256:".len() + 12).collect::<String>();
    let mut out = format!(
        "Catalog {} ({}, {})\n",
        catalog.reference,
        catalog.channel,
        short(&catalog.digest)
    );
    if list.images.is_empty() {
        out.push_str("No images.\n");
        return out;
    }
    out.push_str(&format!(
        "{:<20} {:<14} {:<20} {}\n",
        "NAME", "ARCH", "DIGEST", "DESCRIPTION"
    ));
    for image in &list.images {
        let digest = image
            .image
            .as_deref()
            .and_then(|pinned| pinned.rsplit_once('@'))
            .map_or_else(|| "-".to_owned(), |(_, digest)| short(digest));
        out.push_str(&format!(
            "{:<20} {:<14} {:<20} {}\n",
            image.name,
            image.architectures.join(","),
            digest,
            image.description
        ));
    }
    out
}

#[cfg(test)]
mod tests;
