//! Fresh approved controls over an admitted image retained on the private share.
use super::{read_control_file, CONTROL_FILE_LIMIT};
use crate::container::{stage, WorkloadResources};
use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
use std::{collections::BTreeMap, path::Path};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Rootfs {
    digest: String,
    integrity: String,
    size: u64,
}

/// The caller has admitted this immutable image and selected this historical
/// revision for the original new owner. Only image metadata is carried; args,
/// variables, startup, repositories and resource controls use fresh inputs.
/// Nothing is published here: the service applies its ordinary import policy.
pub fn plan_environment(
    session: &Path,
    manifest: &str,
    context: &stage::EnvironmentLaunchContext,
    variables: &BTreeMap<String, String>,
    startup: Option<&capsem_config::environment::EnvironmentStartup>,
    repositories: &[capsem_config::environment::EnvironmentRepository],
    resources: WorkloadResources,
) -> Result<Vec<stage::StagedFile>> {
    context.validate()?;
    capsem_assets::oci::Digest::parse(manifest)?;
    let share = ContainedDir::open_root(session)?.descend(crate::session::IMAGE_SHARE_DIR.as_ref())?;
    let bytes = read_control_file(&share, "options.json")?.context("trusted workload options are missing")?;
    let options: serde_json::Value = serde_json::from_slice(&bytes).context("parse trusted workload options")?;
    ensure!(
        options["manifest"].as_str() == Some(manifest),
        "carried workload manifest changed"
    );
    let previous: stage::EnvironmentLaunchContext = serde_json::from_value(
        options
            .get("environment")
            .context("trusted workload has no environment context")?
            .clone(),
    )
    .context("invalid saved environment context")?;
    previous.validate()?;
    // A reconnect to this same live owner cannot rewrite its running controls.
    if previous == *context {
        return Ok(Vec::new());
    }
    let image = verified_json(&share, manifest)?;
    let config = verified_json(
        &share,
        image["config"]["digest"]
            .as_str()
            .context("carried manifest names no config")?,
    )?;
    let labels = config["config"]["Labels"].as_object().cloned().unwrap_or_default();
    let surface = stage::DeclaredSurface::from_labels(&labels)?;
    let rootfs: Option<Rootfs> = options
        .get("rootfs")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()?;
    let plan = stage::stage_plan(
        stage::StageImage {
            manifest,
            rootfs: rootfs.as_ref().map(|root| stage::RootfsFile {
                digest: &root.digest,
                integrity: &root.integrity,
                size: root.size,
            }),
            environment: Some(context),
            startup,
            repositories,
        },
        &[],
        variables,
        resources,
        surface,
    )?;
    if let Some(root) = rootfs {
        let name = root
            .digest
            .strip_prefix("sha256:")
            .context("invalid carried rootfs digest")?;
        let file = share.open_file(name.as_ref(), ContainedOpenOptions::read_only())?;
        capsem_crypto::verify::blake3(file, &root.integrity, root.size).context("verify carried rootfs")?;
    }
    ensure!(
        plan.iter().all(|file| file.bytes.len() as u64 <= CONTROL_FILE_LIMIT),
        "reprepared controls exceed byte limit"
    );
    Ok(plan)
}

fn verified_json(share: &ContainedDir, digest: &str) -> Result<serde_json::Value> {
    capsem_assets::oci::Digest::parse(digest)?;
    let name = digest.strip_prefix("sha256:").context("invalid carried image digest")?;
    let bytes = read_control_file(share, name)?.context("carried image metadata is missing")?;
    let mut verifier = capsem_crypto::verify::Verifier::new(digest)?;
    verifier.update(&bytes);
    ensure!(verifier.finish(), "carried image metadata integrity mismatch");
    serde_json::from_slice(&bytes).context("parse carried image metadata")
}

#[cfg(test)]
mod tests;
