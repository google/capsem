//! How a committed write op advances the session's counter snapshot.
//!
//! The writer is the only place that knows what a transaction committed: an op
//! refused by a CHECK constraint, rolled back with its batch, or retried alone
//! never reaches the table, and a completion with no start row updates
//! nothing. So the writer records each op's effect while it inserts, and
//! applies those effects here only after the transaction commits. Counting at
//! the producer, or on enqueue, would count rows that never landed.
//!
//! The snapshot is persisted beside the rows it counts, in the same flush
//! transaction, and read back whole. Nothing recomputes it by scanning.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use capsem_proto::forensic::SecurityForensicEvent;
use capsem_proto::ledger_counters::{bounded_entry, ModelUsage, MAX_OPEN_ASKS};
pub use capsem_proto::ledger_counters::{LedgerCounters, MAX_KEYS_PER_MAP, OVERFLOW_KEY};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use crate::events::{Decision, SecurityAskStatus};
use crate::writer::{format_timestamp, WriteOp};

/// The tool-call origins every surface counts as a tool call.
///
/// `mcp_proxy` is left out on purpose: it is a model naming an MCP tool in its
/// response, and the call it names is recorded again, as `mcp`, when the MCP
/// gateway carries it. Counting both would count one invocation twice.
pub const COUNTED_TOOL_ORIGINS: [&str; 4] = ["native", "mcp", "builtin", "local"];

/// Whether a `tool_calls.origin` counts as a tool call.
pub fn is_counted_tool_origin(origin: &str) -> bool {
    COUNTED_TOOL_ORIGINS.contains(&origin)
}

/// Security events whose payload was already counted. One event matching
/// several rules is recorded once per rule, each carrying the same payload,
/// and its plugin executions happened once. Rule matches for one event are
/// emitted together, so a short window is enough to recognise the repeats.
const RECENT_PAYLOAD_EVENTS: usize = 64;

/// What storing an op did that the op alone does not say.
#[derive(Debug, Default)]
pub(crate) struct StoredEffect {
    /// A completion set `exit_code` on a start row that had none.
    pub(crate) exec_completed: bool,
    /// The op changed a row that was already flushed, so the change reaches
    /// the disk when its batch commits rather than at the next flush, and the
    /// snapshot has to go with it.
    pub(crate) updated_disk_row: bool,
    /// The structured payload of a security rule event, parsed once for the
    /// archive and reused here.
    pub(crate) forensic: Option<SecurityForensicEvent>,
}

/// The writer's counters, and the little it must remember to advance them.
#[derive(Debug, Clone, Default)]
pub(crate) struct LedgerTally {
    counters: LedgerCounters,
    recent_payload_events: VecDeque<String>,
}

impl LedgerTally {
    pub(crate) fn restored(counters: LedgerCounters) -> Self {
        Self {
            counters,
            recent_payload_events: VecDeque::new(),
        }
    }

    pub(crate) fn counters(&self) -> &LedgerCounters {
        &self.counters
    }

    /// Advance the counters for one op whose transaction committed.
    pub(crate) fn record(&mut self, op: &WriteOp, effect: &StoredEffect) {
        let counters = &mut self.counters;
        match op {
            WriteOp::NetEvent(event) => {
                let net = &mut counters.net;
                net.total += 1;
                match event.decision {
                    Decision::Allowed => net.allowed += 1,
                    Decision::Denied => net.denied += 1,
                    Decision::Error => net.error += 1,
                    Decision::Redirected => {}
                }
                net.bytes_sent = net.bytes_sent.saturating_add(event.bytes_sent);
                net.bytes_received = net.bytes_received.saturating_add(event.bytes_received);
            }
            WriteOp::ModelCall(call) => {
                let usage = ModelUsage {
                    calls: 1,
                    input_tokens: call.input_tokens.unwrap_or(0),
                    output_tokens: call.output_tokens.unwrap_or(0),
                    duration_ms: call.duration_ms,
                    cost_micro_usd: cost_micro_usd(call.estimated_cost_usd),
                };
                let model = &mut counters.model;
                add_usage(&mut model.total, &usage);
                let by_provider = bounded_entry(&mut model.by_model, &call.provider);
                add_usage(
                    bounded_entry(by_provider, call.model.as_deref().unwrap_or("unknown")),
                    &usage,
                );
                for (key, value) in &call.usage_details {
                    let total = bounded_entry(&mut model.usage_details, key);
                    *total = total.saturating_add(*value);
                }
                for tool_call in &call.tool_calls {
                    if !is_counted_tool_origin(&tool_call.origin) {
                        continue;
                    }
                    counters.tools.calls += 1;
                    let tool = bounded_entry(&mut counters.tools.by_tool, &tool_call.tool_name);
                    tool.calls += 1;
                    // The row carries its model call's duration; count what the row says.
                    tool.duration_ms = tool.duration_ms.saturating_add(call.duration_ms);
                }
            }
            WriteOp::McpCall(call) => {
                // Only `tools/call` is stored as a tool call; other methods are
                // protocol traffic and write no row.
                if call.method != "tools/call" {
                    return;
                }
                let tool_name = call.tool_name.as_deref().unwrap_or("");
                counters.tools.calls += 1;
                for tool in [
                    bounded_entry(&mut counters.tools.by_tool, tool_name),
                    bounded_entry(bounded_entry(&mut counters.tools.mcp, &call.server_name), tool_name),
                ] {
                    tool.calls += 1;
                    tool.duration_ms = tool.duration_ms.saturating_add(call.duration_ms);
                    tool.bytes_sent = tool.bytes_sent.saturating_add(call.bytes_sent);
                    tool.bytes_received = tool.bytes_received.saturating_add(call.bytes_received);
                }
            }
            WriteOp::FileEvent(event) => {
                counters.files.events += 1;
                *bounded_entry(&mut counters.files.by_action, event.action.as_str()) += 1;
            }
            WriteOp::ExecEvent(_) => counters.exec.started += 1,
            WriteOp::ExecEventComplete(_) => {
                if effect.exec_completed {
                    counters.exec.completed += 1;
                }
            }
            WriteOp::AuditEvent(event) => {
                counters.audit.events += 1;
                let seen = format_timestamp(event.timestamp);
                let process = bounded_entry(&mut counters.audit.by_exe, &event.exe);
                process.count += 1;
                if process.first_seen.is_empty() || seen < process.first_seen {
                    process.first_seen.clone_from(&seen);
                }
                if seen > process.last_seen {
                    process.last_seen = seen;
                }
            }
            WriteOp::SubstitutionEvent(event) => {
                if event.material_class != "credential" {
                    return;
                }
                let credential = bounded_entry(&mut counters.credentials, &event.substitution_ref);
                credential.substitutions += 1;
                if event.outcome == "injected" {
                    credential.injected_substitutions += 1;
                }
                if event.provider > credential.provider {
                    credential.provider.clone_from(&event.provider);
                }
                credential.last_seen_unix_ms = credential.last_seen_unix_ms.max(unix_ms(event.timestamp));
            }
            WriteOp::SecurityRuleEvent(event) => {
                let security = &mut counters.security;
                security.matches += 1;
                *bounded_entry(&mut security.by_action, event.rule_action.as_str()) += 1;
                *bounded_entry(&mut security.by_event_type, &event.event_type) += 1;
                *bounded_entry(&mut security.by_level, event.detection_level.as_str()) += 1;
                let by_action = bounded_entry(&mut security.by_rule, &event.rule_id);
                let rule = bounded_entry(
                    bounded_entry(by_action, event.rule_action.as_str()),
                    event.detection_level.as_str(),
                );
                rule.count += 1;
                // Ties go to the later row, as `ORDER BY timestamp DESC, id DESC` does.
                if rule.latest_event_id.is_empty() || event.timestamp_unix_ms >= rule.latest_timestamp_unix_ms {
                    rule.latest_event_id.clone_from(&event.event_id);
                    rule.latest_timestamp_unix_ms = event.timestamp_unix_ms;
                }
                if let Some(forensic) = &effect.forensic {
                    if self.recent_payload_events.contains(&event.event_id) {
                        return;
                    }
                    if self.recent_payload_events.len() == RECENT_PAYLOAD_EVENTS {
                        self.recent_payload_events.pop_front();
                    }
                    self.recent_payload_events.push_back(event.event_id.clone());
                    record_payload(&mut self.counters, forensic, event.timestamp_unix_ms);
                }
            }
            WriteOp::SecurityAskEvent(event) => {
                let security = &mut counters.security;
                match event.status {
                    SecurityAskStatus::Pending => {
                        if security.open_asks.contains(&event.ask_id) {
                            return;
                        }
                        if security.open_asks.len() < MAX_OPEN_ASKS {
                            security.open_asks.insert(event.ask_id.clone());
                        } else {
                            security.open_asks_overflow += 1;
                        }
                    }
                    SecurityAskStatus::Approved | SecurityAskStatus::Denied => {
                        if !security.open_asks.remove(&event.ask_id) && security.open_asks_overflow > 0 {
                            security.open_asks_overflow -= 1;
                        }
                    }
                }
            }
            WriteOp::TransportEvent(_)
            | WriteOp::DnsEvent(_)
            | WriteOp::SecurityDecisionEvent(_)
            | WriteOp::ProfileMutationEvent(_)
            | WriteOp::Network(_)
            | WriteOp::NetworkMembership(_) => {}
        }
    }
}

/// Persist the snapshot in the ledger's single `ledger_counters` row.
///
/// Called inside the transaction that commits the rows it counts.
pub(crate) fn store(conn: &Connection, counters: &LedgerCounters) -> rusqlite::Result<()> {
    let encoded = counters
        .encode()
        .map_err(|error| rusqlite::Error::InvalidParameterName(format!("{error:#}")))?;
    conn.prepare_cached("UPDATE main.ledger_counters SET counters = ?1 WHERE singleton = 1")?
        .execute([encoded])
        .and_then(|updated| match updated {
            1 => Ok(()),
            _ => Err(missing_counters_row()),
        })
}

/// The snapshot the ledger last committed.
///
/// Every ledger is created with its row, so a missing one is a broken
/// ledger, not a session that counted nothing.
pub(crate) fn load(conn: &Connection) -> rusqlite::Result<LedgerCounters> {
    let encoded: Vec<u8> = conn
        .query_row(
            "SELECT counters FROM main.ledger_counters WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(missing_counters_row)?;
    decode(&encoded)
}

/// Decode a stored snapshot, naming the ledger contract when it is not one.
pub fn decode(encoded: &[u8]) -> rusqlite::Result<LedgerCounters> {
    LedgerCounters::decode(encoded).map_err(|error| {
        rusqlite::Error::InvalidParameterName(format!("ledger_counters row is not a snapshot: {error:#}"))
    })
}

/// How a reader takes the snapshot: one primary-key lookup.
///
/// Hex, because the raw query rail renders a BLOB as a placeholder string.
/// Going down that rail rather than a request of its own is what lets a polled
/// route be answered from the handle's cache while the ledger has not moved.
pub(crate) const SNAPSHOT_SQL: &str = "SELECT hex(counters) FROM main.ledger_counters WHERE singleton = 1";

/// The snapshot out of a `SNAPSHOT_SQL` result
/// (`{"columns":[...],"rows":[["<hex>"]]}`).
pub(crate) fn from_snapshot_result(raw: &str) -> Result<LedgerCounters, String> {
    let parsed: Value =
        serde_json::from_str(raw).map_err(|error| format!("ledger counters returned invalid json: {error}"))?;
    let hex = parsed
        .get("rows")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| row.get(0))
        .and_then(Value::as_str)
        .ok_or_else(|| missing_counters_row().to_string())?;
    let bytes = decode_hex(hex).ok_or_else(|| "ledger_counters row is not hex".to_string())?;
    decode(&bytes).map_err(|error| error.to_string())
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect()
}

fn missing_counters_row() -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName("ledger_counters has no snapshot row: the ledger is broken".to_string())
}

/// Plugin executions, plugin detections and credential activity carried by
/// one security event's payload.
fn record_payload(counters: &mut LedgerCounters, forensic: &SecurityForensicEvent, timestamp_unix_ms: i64) {
    let mut executed = std::collections::BTreeSet::new();
    for execution in &forensic.plugin_executions {
        let Some(plugin_id) = str_field(execution, "plugin_id") else {
            continue;
        };
        let stage = str_field(execution, "stage").unwrap_or("unknown");
        if !executed.insert((plugin_id, stage)) {
            continue;
        }
        let plugin = bounded_entry(&mut counters.plugins, plugin_id);
        plugin.executions += 1;
        if execution.get("applied").and_then(Value::as_bool).unwrap_or(false) {
            plugin.applied += 1;
        } else {
            plugin.skipped += 1;
        }
        let duration_us = execution.get("duration_us").and_then(Value::as_u64).unwrap_or(0);
        plugin.total_duration_us = plugin.total_duration_us.saturating_add(duration_us);
        plugin.max_duration_us = plugin.max_duration_us.max(duration_us);
    }
    let mut detected = std::collections::BTreeSet::new();
    for detection in &forensic.detections {
        if str_field(detection, "source") != Some("plugin") {
            continue;
        }
        let Some(plugin_id) = str_field(detection, "plugin_id") else {
            continue;
        };
        if detected.insert(plugin_id) {
            bounded_entry(&mut counters.plugins, plugin_id).detections += 1;
        }
    }
    let mut credited = std::collections::BTreeSet::new();
    for (items, injection) in [
        (&forensic.credential_observations, false),
        (&forensic.credential_injections, true),
    ] {
        for item in items {
            let Some(reference) = credential_ref(item) else {
                continue;
            };
            let source = str_field(item, "source").unwrap_or("");
            if !credited.insert((injection, reference, source)) {
                continue;
            }
            let credential = bounded_entry(&mut counters.credentials, reference);
            if injection {
                credential.injections += 1;
            } else {
                credential.observations += 1;
            }
            let provider = str_field(item, "provider");
            if provider > credential.provider.as_deref() {
                credential.provider = provider.map(str::to_string);
            }
            credential.last_seen_unix_ms = credential.last_seen_unix_ms.max(timestamp_unix_ms);
        }
    }
}

/// The reference a payload's credential item names, under any of the
/// spellings producers have used for it.
fn credential_ref(item: &Value) -> Option<&str> {
    ["credential_ref", "substitution_ref", "reference", "ref"]
        .iter()
        .find_map(|field| item.get(field))
        .and_then(Value::as_str)
}

fn str_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn add_usage(total: &mut ModelUsage, usage: &ModelUsage) {
    total.calls = total.calls.saturating_add(usage.calls);
    total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
    total.duration_ms = total.duration_ms.saturating_add(usage.duration_ms);
    total.cost_micro_usd = total.cost_micro_usd.saturating_add(usage.cost_micro_usd);
}

/// One call's estimated cost in whole micro-USD. Rounded per call, so the
/// total is the same wherever and in whatever order it is summed. A cost that
/// is negative or not a number is a pricing bug, not a refund.
pub fn cost_micro_usd(cost_usd: f64) -> u64 {
    if cost_usd.is_finite() && cost_usd > 0.0 {
        (cost_usd * 1_000_000.0).round() as u64
    } else {
        0
    }
}

/// Whole micro-USD back to the dollars the API reports.
pub fn usd_from_micro(micro_usd: u64) -> f64 {
    micro_usd as f64 / 1_000_000.0
}

fn unix_ms(timestamp: SystemTime) -> i64 {
    timestamp
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod equivalence;
#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(test)]
pub(crate) mod oracle;
#[cfg(test)]
mod tests;
