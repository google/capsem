//! The detail a host ledger carries for a session event, as sparse named
//! MessagePack: what the session was when it was created, and what it had
//! done when it stopped.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::ledger_counters::LedgerCounters;
use crate::repeated::MAX_ENCODED_EVENT_BYTES;
use crate::sparse::is_default;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSessionDetail {
    #[serde(default, skip_serializing_if = "is_default")]
    pub persistent: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub ram_bytes: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub scratch_disk_size_gb: u32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub storage_mode: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub rootfs_hash: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub rootfs_version: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub forked_from: Option<String>,
    /// How the session ended: `stopped`, `crashed`, ...; set on a stop.
    #[serde(default, skip_serializing_if = "is_default")]
    pub status: Option<String>,
    /// The session ledger's counters when it stopped.
    #[serde(default, skip_serializing_if = "is_default")]
    pub counters: Option<LedgerCounters>,
}

impl HostSessionDetail {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let encoded = rmp_serde::to_vec_named(self).context("encode host session detail")?;
        anyhow::ensure!(
            encoded.len() <= MAX_ENCODED_EVENT_BYTES,
            "host session detail exceeds {MAX_ENCODED_EVENT_BYTES} bytes"
        );
        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self> {
        anyhow::ensure!(
            encoded.len() <= MAX_ENCODED_EVENT_BYTES,
            "host session detail exceeds {MAX_ENCODED_EVENT_BYTES} bytes"
        );
        rmp_serde::from_slice(encoded).context("decode host session detail")
    }
}

#[cfg(test)]
mod tests;
