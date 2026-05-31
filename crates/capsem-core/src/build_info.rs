use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const BUILD_INFO_JSON_ARG: &str = "--build-info-json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfo {
    pub binary: String,
    pub version: String,
    pub protocol_version: u16,
    pub schema_hash: String,
    pub build_ts: String,
}

impl BuildInfo {
    pub fn current(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: capsem_proto::PROTOCOL_VERSION,
            schema_hash: schema_hash_hex(),
            build_ts: option_env!("CAPSEM_BUILD_TS").unwrap_or("dev").to_string(),
        }
    }

    pub fn protocol_compatible_with_current(&self) -> bool {
        self.protocol_version == capsem_proto::PROTOCOL_VERSION
            && self.schema_hash == schema_hash_hex()
    }
}

pub fn schema_hash_hex() -> String {
    format!("{:016x}", capsem_proto::SCHEMA_HASH)
}

pub fn maybe_print_json_and_exit(binary: &str) -> Result<bool> {
    if !std::env::args().any(|arg| arg == BUILD_INFO_JSON_ARG) {
        return Ok(false);
    }
    println!("{}", serde_json::to_string(&BuildInfo::current(binary))?);
    Ok(true)
}

pub async fn query_binary(path: &Path, timeout: std::time::Duration) -> Option<BuildInfo> {
    let output = tokio::time::timeout(
        timeout,
        tokio::process::Command::new(path)
            .arg(BUILD_INFO_JSON_ARG)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}
