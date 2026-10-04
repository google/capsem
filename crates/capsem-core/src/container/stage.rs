//! What the guest launcher needs in the VM's stage directory, independent of
//! who writes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::LAUNCHER;

/// One small control file written into [`super::STAGE`]. The image itself is
/// never staged there: it reaches the guest through the read-only image share
/// (`crate::session::publish_image_share`), and the stage names it by digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// What of a pulled layout the image share holds: the manifest the layout's
/// index names, and every blob, by SHA-256 hex digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBlobs {
    /// `sha256:<hex>` of the manifest the launcher unpacks.
    pub manifest: String,
    pub blobs: Vec<String>,
}

/// The blobs of the pulled single-image layout at `root`, whose verified
/// files are `files`. The manifest the index names, its config and every
/// layer must be among them: a share missing one could not be unpacked, and
/// the launcher must never be pointed at a blob the host did not verify.
pub fn image_blobs(root: &Path, files: &[PathBuf]) -> Result<ImageBlobs> {
    let directory = Path::new("blobs/sha256");
    let blobs: Vec<String> = files
        .iter()
        .filter_map(|file| file.strip_prefix(directory).ok())
        .map(|name| {
            let name = name.to_str().context("blob name is not UTF-8")?;
            Ok(digest_hex(&format!("sha256:{name}"))?.to_owned())
        })
        .collect::<Result<_>>()?;
    let index = read_json(&root.join("index.json"))?;
    let manifest = index["manifests"][0]["digest"]
        .as_str()
        .context("the layout's index names no manifest")?
        .to_owned();
    let document = read_json(&root.join(directory).join(digest_hex(&manifest)?))?;
    let mut named = vec![manifest.as_str()];
    named.push(
        document["config"]["digest"]
            .as_str()
            .with_context(|| format!("manifest {manifest} names no config"))?,
    );
    for layer in document["layers"]
        .as_array()
        .with_context(|| format!("manifest {manifest} lists no layers"))?
    {
        named.push(layer["digest"].as_str().context("a layer names no digest")?);
    }
    for digest in named {
        let hex = digest_hex(digest)?;
        if !blobs.iter().any(|blob| blob == hex) {
            bail!("manifest {manifest} names {digest}, which the pull did not verify");
        }
    }
    Ok(ImageBlobs { manifest, blobs })
}

/// The stage, in write order: `options.json` (the image's manifest digest,
/// the command override, container environment, where the container sees
/// the workspace, and the capabilities, syscall filter, user-namespace map
/// and resources the workload gets), then the launcher. `surface` is what
/// [`image_surface`] read from the image; `manifest` is the digest the image
/// share holds the image under ([`image_blobs`]).
pub fn stage_plan(
    manifest: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    resources: super::WorkloadResources,
    surface: DeclaredSurface,
) -> Result<Vec<StagedFile>> {
    digest_hex(manifest)?;
    let surface = surface.seccomp();
    Ok(vec![
        StagedFile {
            name: "options.json".into(),
            bytes: serde_json::to_vec(&serde_json::json!({
                "manifest": manifest,
                "args": args,
                "env": env,
                "workspace": super::CONTAINER_WORKSPACE,
                "capabilities": super::seccomp::WORKLOAD_CAPABILITIES,
                "seccomp": super::seccomp::workload_seccomp(oci_architecture()?, surface)?,
                "id_map": super::WORKLOAD_ID_MAP,
                "resources": resources,
                "surface": match surface {
                    super::seccomp::Surface::Terminal => "terminal",
                    super::seccomp::Surface::Xpra => "xpra",
                },
            }))?,
        },
        StagedFile {
            name: "launch.py".into(),
            bytes: LAUNCHER.to_vec(),
        },
    ])
}

/// The hex of a `sha256:<hex>` digest, refused unless it is one.
fn digest_hex(digest: &str) -> Result<&str> {
    capsem_assets::oci::Digest::parse(digest)?;
    Ok(digest.trim_start_matches("sha256:"))
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}

/// The label an image declares its surface with (`terminal` or `xpra`).
pub const SURFACE_LABEL: &str = "org.capsem.surface";

/// The container loopback port an xpra surface listens on.
pub const SURFACE_PORT_LABEL: &str = "org.capsem.surface.port";

/// What an image's labels ask the runtime to present.
///
/// An xpra surface names exactly one container loopback port, which the
/// service grants as a browser-preview exposure; nothing else in the image can
/// ask for more. A terminal image names none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclaredSurface {
    Terminal,
    Xpra { port: u16 },
}

impl DeclaredSurface {
    /// Read and validate the surface labels. A missing, malformed or zero
    /// xpra port, or a port on a terminal image, refuses the image instead of
    /// guessing: the runtime never exposes a port an image did not declare
    /// exactly.
    pub fn from_labels(labels: &serde_json::Map<String, serde_json::Value>) -> Result<Self> {
        let label = |name: &str| -> Result<Option<&str>> {
            labels
                .get(name)
                .map(|value| value.as_str().with_context(|| format!("{name} must be a string")))
                .transpose()
        };
        let port = label(SURFACE_PORT_LABEL)?;
        match (super::seccomp::Surface::from_label(label(SURFACE_LABEL)?)?, port) {
            (super::seccomp::Surface::Terminal, None) => Ok(Self::Terminal),
            (super::seccomp::Surface::Terminal, Some(_)) => {
                bail!("{SURFACE_PORT_LABEL} needs {SURFACE_LABEL}=xpra")
            }
            (super::seccomp::Surface::Xpra, None) => bail!("an xpra surface needs {SURFACE_PORT_LABEL}"),
            (super::seccomp::Surface::Xpra, Some(port)) => {
                // Digits only: `u16::from_str` also takes "+14500".
                let digits = (1..=5).contains(&port.len()) && port.bytes().all(|byte| byte.is_ascii_digit());
                match port.parse::<u16>() {
                    Ok(port) if digits && port != 0 => Ok(Self::Xpra { port }),
                    _ => bail!("{SURFACE_PORT_LABEL} {port:?} is not a port between 1 and 65535"),
                }
            }
        }
    }

    /// The syscall-filter exception this surface needs.
    pub fn seccomp(self) -> super::seccomp::Surface {
        match self {
            Self::Terminal => super::seccomp::Surface::Terminal,
            Self::Xpra { .. } => super::seccomp::Surface::Xpra,
        }
    }
}

/// The surface the pulled layout at `root` declares.
pub fn image_surface(root: &Path) -> Result<DeclaredSurface> {
    DeclaredSurface::from_labels(&image_labels(root)?)
}

/// The image config's labels from a pulled single-image layout: index.json
/// names the manifest, the manifest names the config. The layout is the
/// puller's verified output, so its blobs are read as written.
pub fn image_labels(root: &Path) -> Result<serde_json::Map<String, serde_json::Value>> {
    let blob = |digest: &serde_json::Value| -> Result<serde_json::Value> {
        read_json(
            &root
                .join("blobs/sha256")
                .join(digest_hex(digest.as_str().context("digest")?)?),
        )
    };
    let index = read_json(&root.join("index.json"))?;
    // A layout naming no manifest declares nothing: the terminal surface,
    // the narrowest filter, is what that gives.
    if index["manifests"][0].is_null() {
        return Ok(serde_json::Map::new());
    }
    let manifest = blob(&index["manifests"][0]["digest"])?;
    let config = blob(&manifest["config"]["digest"])?;
    Ok(config["config"]["Labels"].as_object().cloned().unwrap_or_default())
}

/// The OCI platform architecture of this host.
pub fn oci_architecture() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Ok("arm64"),
        "x86_64" => Ok("amd64"),
        other => bail!("unsupported container architecture: {other}"),
    }
}

/// This host's architecture as the image catalog names it, which picks the
/// catalog version a name resolves to.
pub fn catalog_architecture() -> Result<capsem_assets::asset_manager::PackageArchitecture> {
    use capsem_assets::asset_manager::PackageArchitecture;
    match oci_architecture()? {
        "arm64" => Ok(PackageArchitecture::Arm64),
        "amd64" => Ok(PackageArchitecture::Amd64),
        other => bail!("unsupported container architecture: {other}"),
    }
}

#[cfg(test)]
mod tests;
