//! Pair identity survives the normal sidecar store; old functional receipts
//! cannot substitute for a fresh collector run or its machine-fitness checks.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::schema::Record;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Phase {
    source_commit: String,
    version: String,
    package_sha256: String,
    sbom_sha256: String,
    manifest_sha256: String,
    runtime_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    schema: String,
    scope: String,
    source_commit: String,
    owner: String,
    transport: Transport,
    before: Phase,
    after: Phase,
    receipt_sha256: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Transport {
    ServiceUds,
    GatewayHttp,
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn stable_version(value: &str) -> Option<[u64; 3]> {
    let mut parts = value.split('.');
    let part = |value: Option<&str>| {
        let value = value?;
        if value.is_empty() || (value.len() > 1 && value.starts_with('0')) || !value.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        value.parse().ok()
    };
    let version = [part(parts.next())?, part(parts.next())?, part(parts.next())?];
    parts.next().is_none().then_some(version)
}

fn read(record: &Record) -> Result<Proof> {
    let proof: Proof = serde_json::from_str(record.sidecar.as_deref().context("Install lacks pair provenance")?)
        .context("invalid Install pair provenance")?;
    if proof.schema != "capsem.bench.install.v1"
        || proof.scope != "binary_apply_only"
        || !matches!(proof.owner.as_str(), "linux-docker-systemd" | "macos-tart")
        || proof.source_commit != record.release.commit
        || proof.after.source_commit != record.release.commit
        || !hex(&proof.receipt_sha256, 64)
        || (record.host.os == "linux" && proof.owner != "linux-docker-systemd")
        || (record.host.os == "macos" && proof.owner != "macos-tart")
        || !matches!(record.host.os.as_str(), "linux" | "macos")
    {
        bail!("Install proof has a foreign source, owner or scope");
    }
    for phase in [&proof.before, &proof.after] {
        if !hex(&phase.source_commit, 40)
            || phase.version.is_empty()
            || [
                &phase.package_sha256,
                &phase.sbom_sha256,
                &phase.manifest_sha256,
                &phase.runtime_sha256,
            ]
            .iter()
            .any(|digest| !hex(digest, 64))
        {
            bail!("Install proof has incomplete package/source/SBOM/runtime identity");
        }
    }
    if proof.before.runtime_sha256 != proof.after.runtime_sha256
        || proof.before.source_commit == proof.after.source_commit
        || proof.before.package_sha256 == proof.after.package_sha256
    {
        bail!("Install binary Apply proof lacks a distinct package/source with identical runtime");
    }
    match (
        stable_version(&proof.before.version),
        stable_version(&proof.after.version),
    ) {
        (Some(before), Some(after)) if before < after => {}
        _ => bail!("Install proof requires a genuinely newer stable package version"),
    }
    Ok(proof)
}

pub(crate) fn eligible(record: &Record) -> Result<()> {
    read(record).map(|_| ())
}

pub(crate) fn compatible(a: &Record, b: &Record) -> Result<()> {
    let (a, b) = (read(a)?, read(b)?);
    if a.owner != b.owner
        || a.transport != b.transport
        || a.before != b.before
        || a.after.runtime_sha256 != b.after.runtime_sha256
        || (a.source_commit == b.source_commit && a.after != b.after)
    {
        bail!("Install transport/predecessor/runtime or same-source candidate inputs changed");
    }
    Ok(())
}
