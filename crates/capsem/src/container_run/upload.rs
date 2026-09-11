use super::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

#[derive(Serialize)]
struct Transfer {
    path: String,
    key: usize,
    parts: usize,
    sha256: String,
}

pub(super) async fn image(client: &UdsClient, vm: &str, image: &ImageLayout, args: &RunArgs) -> Result<()> {
    let mut transfer = Vec::new();
    // Every upload stays below the existing file API body limit. OCI extraction
    // happens inside the VM; host files here contain only verified image blobs.
    let mut buffer = vec![0; 1024 * 1024];
    for (key, path) in image.files().iter().enumerate() {
        let mut input = tokio::fs::File::open(image.path().join(path)).await?;
        let mut hash = Sha256::new();
        let mut parts = 0;
        loop {
            let length = input.read(&mut buffer).await?;
            if length == 0 {
                break;
            }
            hash.update(&buffer[..length]);
            file(client, vm, &format!("{key}-{parts}"), buffer[..length].to_vec()).await?;
            parts += 1;
        }
        transfer.push(Transfer {
            path: path.to_str().context("non-UTF8 image path")?.into(),
            key,
            parts,
            sha256: format!("{:x}", hash.finalize()),
        });
    }
    file(client, vm, "transfer.json", serde_json::to_vec(&transfer)?).await?;
    let options = serde_json::json!({"args": args.args, "env": client::parse_env_vars(&args.env)?.unwrap_or_default()});
    file(client, vm, "options.json", serde_json::to_vec(&options)?).await?;
    file(client, vm, "launch.py", container::LAUNCHER.to_vec()).await
}

async fn file(client: &UdsClient, vm: &str, name: &str, bytes: Vec<u8>) -> Result<()> {
    let path = format!("{}/{name}", container::STAGE);
    let route = format!("/vms/{vm}/files/content?path={}", urlencoding::encode(&path));
    let (response, _) = client
        .request_bytes("POST", &route, Some(bytes), Some("application/octet-stream"))
        .await?;
    #[derive(serde::Deserialize)]
    struct Uploaded {
        success: bool,
    }
    ensure!(
        serde_json::from_slice::<Uploaded>(&response)?.success,
        "image upload was refused"
    );
    Ok(())
}
