//! The counter snapshot a session ledger keeps beside its rows.
//!
//! The ledger's writer owns one of these, advances it for every op it
//! commits, and persists it in the same transaction as the rows it counts.
//! Readers take it whole with a primary-key lookup instead of aggregating the
//! tables they would otherwise scan on every poll. The wire type lives here,
//! with the other Capsem-owned MessagePack records; the rules for advancing it
//! belong to the ledger writer.
//!
//! Every map is bounded. Its keys come from traffic the guest controls -- a
//! model name, a tool name, an executable path -- so an unbounded map would
//! let a guest spend the host's memory and the snapshot's bytes by varying
//! them. Past `MAX_KEYS_PER_MAP` distinct keys, further keys are counted
//! under `OVERFLOW_KEY`: totals stay exact, attribution stops.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::repeated::MAX_ENCODED_EVENT_BYTES;
use crate::sparse::is_default;

/// Distinct keys a single counter map attributes before folding the rest.
pub const MAX_KEYS_PER_MAP: usize = 256;

/// Where keys past `MAX_KEYS_PER_MAP` are counted. Parenthesised so it cannot
/// be mistaken for, or collide with, a real tool, model or rule identifier.
pub const OVERFLOW_KEY: &str = "(other)";

/// Open asks tracked by id. An ask resolves quickly or not at all, so more
/// than this many open at once means a producer never resolves them; the
/// count past the bound is kept in `SecurityCounters::open_asks_overflow`.
pub const MAX_OPEN_ASKS: usize = 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerCounters {
    #[serde(default, skip_serializing_if = "is_default")]
    pub net: NetCounters,
    #[serde(default, skip_serializing_if = "is_default")]
    pub model: ModelCounters,
    #[serde(default, skip_serializing_if = "is_default")]
    pub tools: ToolCounters,
    #[serde(default, skip_serializing_if = "is_default")]
    pub files: FileCounters,
    #[serde(default, skip_serializing_if = "is_default")]
    pub exec: ExecCounters,
    #[serde(default, skip_serializing_if = "is_default")]
    pub audit: AuditCounters,
    #[serde(default, skip_serializing_if = "is_default")]
    pub security: SecurityCounters,
    /// Plugin runtime by plugin id.
    #[serde(default, skip_serializing_if = "is_default")]
    pub plugins: BTreeMap<String, PluginCounters>,
    /// Brokered credential activity by credential reference.
    #[serde(default, skip_serializing_if = "is_default")]
    pub credentials: BTreeMap<String, CredentialCounters>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetCounters {
    #[serde(default, skip_serializing_if = "is_default")]
    pub total: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub allowed: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub denied: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub error: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub bytes_sent: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub bytes_received: u64,
}

/// Model usage totals. Cost is integer micro-USD so a sum over millions of
/// calls is exact and identical wherever it is computed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsage {
    #[serde(default, skip_serializing_if = "is_default")]
    pub calls: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub input_tokens: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub cost_micro_usd: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCounters {
    #[serde(default, skip_serializing_if = "is_default")]
    pub total: ModelUsage,
    /// Provider-reported usage details (cache reads, thinking, ...) by key.
    #[serde(default, skip_serializing_if = "is_default")]
    pub usage_details: BTreeMap<String, u64>,
    /// Usage by provider, then by model (`unknown` when the call named none).
    #[serde(default, skip_serializing_if = "is_default")]
    pub by_model: BTreeMap<String, BTreeMap<String, ModelUsage>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUsage {
    #[serde(default, skip_serializing_if = "is_default")]
    pub calls: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub bytes_sent: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub bytes_received: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCounters {
    /// Tool calls in the counted origins, the one definition every surface
    /// reports.
    #[serde(default, skip_serializing_if = "is_default")]
    pub calls: u64,
    /// Counted tool calls by tool name.
    #[serde(default, skip_serializing_if = "is_default")]
    pub by_tool: BTreeMap<String, ToolUsage>,
    /// MCP tool calls by server, then by tool.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mcp: BTreeMap<String, BTreeMap<String, ToolUsage>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCounters {
    /// Every `fs_events` row, overflow markers included.
    #[serde(default, skip_serializing_if = "is_default")]
    pub events: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub by_action: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecCounters {
    #[serde(default, skip_serializing_if = "is_default")]
    pub started: u64,
    /// Starts whose completion landed: counted once, when `exit_code` is set.
    #[serde(default, skip_serializing_if = "is_default")]
    pub completed: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessUsage {
    #[serde(default, skip_serializing_if = "is_default")]
    pub count: u64,
    /// Ledger timestamps (RFC 3339, fixed width), as the rows spell them.
    #[serde(default, skip_serializing_if = "is_default")]
    pub first_seen: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub last_seen: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditCounters {
    #[serde(default, skip_serializing_if = "is_default")]
    pub events: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub by_exe: BTreeMap<String, ProcessUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleUsage {
    #[serde(default, skip_serializing_if = "is_default")]
    pub count: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub latest_event_id: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub latest_timestamp_unix_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityCounters {
    /// Rule matches.
    #[serde(default, skip_serializing_if = "is_default")]
    pub matches: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub by_action: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub by_event_type: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub by_level: BTreeMap<String, u64>,
    /// Matches by rule id, then action, then detection level.
    #[serde(default, skip_serializing_if = "is_default")]
    pub by_rule: BTreeMap<String, BTreeMap<String, BTreeMap<String, RuleUsage>>>,
    /// Asks recorded pending and not yet resolved, by ask id.
    #[serde(default, skip_serializing_if = "is_default")]
    pub open_asks: BTreeSet<String>,
    /// Open asks beyond `MAX_OPEN_ASKS`, counted but not named.
    #[serde(default, skip_serializing_if = "is_default")]
    pub open_asks_overflow: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCounters {
    #[serde(default, skip_serializing_if = "is_default")]
    pub executions: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub applied: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub skipped: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub total_duration_us: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub max_duration_us: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub detections: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialCounters {
    /// The greatest provider name seen for this reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Credential substitutions recorded for this reference.
    #[serde(default, skip_serializing_if = "is_default")]
    pub substitutions: u64,
    /// Of those, the ones whose outcome was an injection.
    #[serde(default, skip_serializing_if = "is_default")]
    pub injected_substitutions: u64,
    /// Observations carried by security payloads.
    #[serde(default, skip_serializing_if = "is_default")]
    pub observations: u64,
    /// Injections carried by security payloads.
    #[serde(default, skip_serializing_if = "is_default")]
    pub injections: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub last_seen_unix_ms: i64,
}

impl LedgerCounters {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let encoded = rmp_serde::to_vec_named(self).context("encode ledger counters")?;
        anyhow::ensure!(
            encoded.len() <= MAX_ENCODED_EVENT_BYTES,
            "ledger counters exceed {MAX_ENCODED_EVENT_BYTES} bytes"
        );
        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self> {
        anyhow::ensure!(
            encoded.len() <= MAX_ENCODED_EVENT_BYTES,
            "ledger counters exceed {MAX_ENCODED_EVENT_BYTES} bytes"
        );
        rmp_serde::from_slice(encoded).context("decode ledger counters")
    }
}

/// The entry for `key` in a bounded map, folding a new key into
/// `OVERFLOW_KEY` once the map already attributes `MAX_KEYS_PER_MAP` keys.
pub fn bounded_entry<'a, V: Default>(map: &'a mut BTreeMap<String, V>, key: &str) -> &'a mut V {
    let key = if map.contains_key(key) || map.len() < MAX_KEYS_PER_MAP {
        key
    } else {
        OVERFLOW_KEY
    };
    map.entry(key.to_string()).or_default()
}

#[cfg(test)]
mod tests;
