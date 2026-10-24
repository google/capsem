use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct HostConfig {
    signing: Signing,
    functional: Functional,
}
#[derive(Deserialize)]
struct Signing {
    built: Vec<String>,
}
#[derive(Deserialize)]
struct Functional {
    native_file_content: Artifact,
}
#[derive(Deserialize)]
struct Artifact {
    artifact_name: String,
}

fn configured() -> HostConfig {
    // This is the artifact's compiled source configuration, never an ambient
    // file beside the test or a second hand-maintained host-role inventory.
    toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/gate.toml"
    )))
    .unwrap()
}

pub(super) fn artifact_name() -> String {
    configured().functional.native_file_content.artifact_name
}

fn roles() -> Vec<String> {
    let config = configured();
    let mut roles = config.signing.built;
    roles.push(config.functional.native_file_content.artifact_name);
    roles
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    source_commit: String,
    source_tree: String,
    source_digest: String,
    cohort_package: Option<String>,
    binaries: BTreeMap<String, String>,
}

pub(super) fn verify(directory: &StdPath, receipt: &[u8]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let receipt: Receipt = serde_json::from_slice(receipt)?;
    anyhow::ensure!(
        receipt.schema == "capsem.native-file-content.v1",
        "foreign receipt schema"
    );
    for identity in [&receipt.source_commit, &receipt.source_tree] {
        anyhow::ensure!(
            identity.len() == 40 && identity.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "missing source identity"
        );
    }
    for identity in std::iter::once(&receipt.source_digest).chain(receipt.cohort_package.as_ref()) {
        anyhow::ensure!(
            identity.len() == 64 && identity.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "missing component identity"
        );
    }
    let roles = roles();
    anyhow::ensure!(receipt.binaries.len() == roles.len(), "incomplete host generation");
    for role in roles {
        let expected = receipt.binaries.get(&role).context("missing host role")?;
        let path = directory.join(&role);
        let metadata = std::fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
            "invalid host role {role}"
        );
        let observed = capsem_crypto::blake3::hash(&std::fs::read(&path)?).to_hex().to_string();
        anyhow::ensure!(&observed == expected, "foreign host generation: {role}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
