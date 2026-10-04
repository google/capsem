//! What the guest launcher needs in the VM's stage directory, independent of
//! who writes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::LAUNCHER;

/// One file written into [`super::STAGE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
    pub name: String,
    pub content: StagedContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedContent {
    /// A verified image layout file, copied whole.
    File(PathBuf),
    /// Generated launcher input.
    Bytes(Vec<u8>),
}

/// The stage for a pulled layout at `root`, in write order: every layout file
/// with content as its single part, then `transfer.json`, `options.json` (the
/// command override, container environment, where the container sees the
/// workspace, and the capabilities, syscall filter, user-namespace map and
/// resources the workload gets) and
/// the launcher. `surface` is what [`image_surface`] read from the same layout.
pub fn stage_plan(
    root: &Path,
    files: &[PathBuf],
    args: &[String],
    env: &BTreeMap<String, String>,
    resources: super::WorkloadResources,
    surface: DeclaredSurface,
) -> Result<Vec<StagedFile>> {
    let surface = surface.seccomp();
    let transfer = capsem_assets::oci::transfer_manifest(root, files)?;
    let mut plan: Vec<StagedFile> = transfer
        .iter()
        .filter_map(|entry| {
            entry.part_name().map(|name| StagedFile {
                name,
                content: StagedContent::File(root.join(&entry.path)),
            })
        })
        .collect();
    plan.push(StagedFile {
        name: "transfer.json".into(),
        content: StagedContent::Bytes(serde_json::to_vec(&transfer)?),
    });
    plan.push(StagedFile {
        name: "options.json".into(),
        content: StagedContent::Bytes(serde_json::to_vec(&serde_json::json!({
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
        }))?),
    });
    plan.push(StagedFile {
        name: "launch.py".into(),
        content: StagedContent::Bytes(LAUNCHER.to_vec()),
    });
    Ok(plan)
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
    let read = |path: PathBuf| -> Result<serde_json::Value> {
        serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("read {}", path.display()))?)
            .with_context(|| format!("parse {}", path.display()))
    };
    let blob = |digest: &serde_json::Value| -> Result<PathBuf> {
        let digest = digest.as_str().context("digest")?;
        let hex = capsem_assets::oci::Digest::parse(digest)?
            .as_str()
            .trim_start_matches("sha256:")
            .to_owned();
        Ok(root.join("blobs/sha256").join(hex))
    };
    let index = read(root.join("index.json"))?;
    // A layout naming no manifest declares nothing: the terminal surface,
    // the narrowest filter, is what that gives.
    if index["manifests"][0].is_null() {
        return Ok(serde_json::Map::new());
    }
    let manifest = read(blob(&index["manifests"][0]["digest"])?)?;
    let config = read(blob(&manifest["config"]["digest"])?)?;
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
