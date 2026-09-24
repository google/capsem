//! The counters recomputed from the rows: what the writer's snapshot must equal.
//!
//! This is the aggregate SQL the routes used to run on every poll, kept as the
//! definition the counters are held to and nowhere else. It reads a flushed
//! ledger through the same external reader a route uses. It does not apply
//! the counters' key bounds; tests that compare against it stay below them.

use std::collections::BTreeSet;

use capsem_proto::forensic::SecurityForensicEvent;
use capsem_proto::ledger_counters::{CredentialCounters, LedgerCounters, ProcessUsage, RuleUsage, ToolUsage};
use serde_json::Value;

use crate::db::{BodyDirection, DbHandle};

async fn rows(db: &DbHandle, sql: &str) -> Vec<Vec<Value>> {
    let raw = db
        .query(sql, &[])
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let parsed: Value = serde_json::from_str(&raw).unwrap();
    parsed["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().clone())
        .collect()
}

fn u(value: &Value) -> u64 {
    value.as_u64().unwrap_or_else(|| panic!("not a count: {value}"))
}

fn s(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("not text: {value}"))
        .to_string()
}

pub(crate) async fn counters_from_rows(db: &DbHandle) -> LedgerCounters {
    let mut counters = LedgerCounters::default();

    let net = &rows(
        db,
        "SELECT COUNT(*),
                COALESCE(SUM(decision = 'allowed'), 0), COALESCE(SUM(decision = 'denied'), 0),
                COALESCE(SUM(decision = 'error'), 0),
                COALESCE(SUM(bytes_sent), 0), COALESCE(SUM(bytes_received), 0)
         FROM net_events",
    )
    .await[0];
    counters.net.total = u(&net[0]);
    counters.net.allowed = u(&net[1]);
    counters.net.denied = u(&net[2]);
    counters.net.error = u(&net[3]);
    counters.net.bytes_sent = u(&net[4]);
    counters.net.bytes_received = u(&net[5]);

    const MICROS: &str = "CASE WHEN estimated_cost_usd > 0
                              THEN CAST(ROUND(estimated_cost_usd * 1000000) AS INTEGER) ELSE 0 END";
    for row in rows(
        db,
        &format!(
            "SELECT provider, COALESCE(model, 'unknown'), COUNT(*),
                    COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(duration_ms), 0), COALESCE(SUM({MICROS}), 0)
             FROM model_calls GROUP BY provider, COALESCE(model, 'unknown')"
        ),
    )
    .await
    {
        let usage = counters
            .model
            .by_model
            .entry(s(&row[0]))
            .or_default()
            .entry(s(&row[1]))
            .or_default();
        usage.calls = u(&row[2]);
        usage.input_tokens = u(&row[3]);
        usage.output_tokens = u(&row[4]);
        usage.duration_ms = u(&row[5]);
        usage.cost_micro_usd = u(&row[6]);
        let total = &mut counters.model.total;
        total.calls += usage.calls;
        total.input_tokens += usage.input_tokens;
        total.output_tokens += usage.output_tokens;
        total.duration_ms += usage.duration_ms;
        total.cost_micro_usd += usage.cost_micro_usd;
    }
    for row in rows(
        db,
        "SELECT je.key, SUM(je.value) FROM model_calls mc, json_each(mc.usage_details) je
         WHERE mc.usage_details IS NOT NULL GROUP BY je.key",
    )
    .await
    {
        counters.model.usage_details.insert(s(&row[0]), u(&row[1]));
    }

    let origins = super::counted_tool_origins_sql();
    for row in rows(
        db,
        &format!(
            "SELECT tool_name, COUNT(*), COALESCE(SUM(duration_ms), 0),
                    COALESCE(SUM(bytes_sent), 0), COALESCE(SUM(bytes_received), 0)
             FROM tool_calls WHERE origin IN ({origins}) GROUP BY tool_name"
        ),
    )
    .await
    {
        let usage = tool_usage(&row[1..]);
        counters.tools.calls += usage.calls;
        counters.tools.by_tool.insert(s(&row[0]), usage);
    }
    for row in rows(
        db,
        "SELECT server_name, tool_name, COUNT(*), COALESCE(SUM(duration_ms), 0),
                COALESCE(SUM(bytes_sent), 0), COALESCE(SUM(bytes_received), 0)
         FROM tool_calls WHERE origin = 'mcp' GROUP BY server_name, tool_name",
    )
    .await
    {
        counters
            .tools
            .mcp
            .entry(s(&row[0]))
            .or_default()
            .insert(s(&row[1]), tool_usage(&row[2..]));
    }

    for row in rows(db, "SELECT action, COUNT(*) FROM fs_events GROUP BY action").await {
        counters.files.events += u(&row[1]);
        counters.files.by_action.insert(s(&row[0]), u(&row[1]));
    }

    let exec = &rows(db, "SELECT COUNT(*), COUNT(exit_code) FROM exec_events").await[0];
    counters.exec.started = u(&exec[0]);
    counters.exec.completed = u(&exec[1]);

    for row in rows(
        db,
        "SELECT exe, COUNT(*), MIN(timestamp), MAX(timestamp) FROM audit_events GROUP BY exe",
    )
    .await
    {
        counters.audit.events += u(&row[1]);
        counters.audit.by_exe.insert(
            s(&row[0]),
            ProcessUsage {
                count: u(&row[1]),
                first_seen: s(&row[2]),
                last_seen: s(&row[3]),
            },
        );
    }

    let security = &mut counters.security;
    for (column, map) in [
        ("rule_action", &mut security.by_action),
        ("event_type", &mut security.by_event_type),
        ("detection_level", &mut security.by_level),
    ] {
        for row in rows(
            db,
            &format!("SELECT {column}, SUM(count) FROM security_rule_runs GROUP BY {column}"),
        )
        .await
        {
            map.insert(s(&row[0]), u(&row[1]));
        }
    }
    security.matches = security.by_action.values().sum();
    for row in rows(
        db,
        "SELECT runs.rule_id, runs.rule_action, runs.detection_level, SUM(runs.count),
                (SELECT latest.event_id FROM security_rule_events latest
                 WHERE latest.rule_id = runs.rule_id AND latest.rule_action = runs.rule_action
                   AND latest.detection_level = runs.detection_level
                 ORDER BY latest.timestamp_unix_ms DESC, latest.id DESC LIMIT 1),
                MAX(runs.last_timestamp_unix_ms)
         FROM security_rule_runs runs GROUP BY runs.rule_id, runs.rule_action, runs.detection_level",
    )
    .await
    {
        security
            .by_rule
            .entry(s(&row[0]))
            .or_default()
            .entry(s(&row[1]))
            .or_default()
            .insert(
                s(&row[2]),
                RuleUsage {
                    count: u(&row[3]),
                    latest_event_id: s(&row[4]),
                    latest_timestamp_unix_ms: row[5].as_i64().unwrap(),
                },
            );
    }
    for row in rows(
        db,
        "SELECT ask_id FROM security_ask_events AS ask
         WHERE ask.id = (SELECT MAX(id) FROM security_ask_events WHERE ask_id = ask.ask_id)
           AND ask.status = 'pending'",
    )
    .await
    {
        security.open_asks.insert(s(&row[0]));
    }

    for row in rows(
        db,
        "SELECT substitution_ref, MAX(provider), COUNT(*), COALESCE(SUM(outcome = 'injected'), 0),
                MAX(CAST(strftime('%s', timestamp) AS INTEGER) * 1000
                    + CAST(substr(timestamp, 21, 3) AS INTEGER))
         FROM substitution_events WHERE material_class = 'credential' GROUP BY substitution_ref",
    )
    .await
    {
        counters.credentials.insert(
            s(&row[0]),
            CredentialCounters {
                provider: row[1].as_str().map(str::to_string),
                substitutions: u(&row[2]),
                injected_substitutions: u(&row[3]),
                last_seen_unix_ms: row[4].as_i64().unwrap(),
                ..CredentialCounters::default()
            },
        );
    }

    // Every rule event's payload, once per event, in the order it was written.
    let events = rows(
        db,
        "SELECT event_id, MIN(timestamp_unix_ms) FROM security_rule_events GROUP BY event_id ORDER BY MIN(id)",
    )
    .await;
    let ids: Vec<String> = events.iter().map(|row| s(&row[0])).collect();
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let archived = db
        .read_bodies_for_events(&id_refs, "security_rule_events", BodyDirection::Payload, usize::MAX)
        .await
        .unwrap();
    let mut counted = BTreeSet::new();
    for row in &events {
        let event_id = s(&row[0]);
        let Some(body) = archived.bodies.iter().find(|body| body.event_id == event_id) else {
            continue;
        };
        if !counted.insert(event_id) {
            continue;
        }
        if let Ok(forensic) = SecurityForensicEvent::decode(&body.bytes) {
            let payload: Value = serde_json::from_str(&forensic.to_json().unwrap()).unwrap();
            scan_payload(&mut counters, &payload, row[1].as_i64().unwrap());
        }
    }
    counters
}

/// The plugin and credential scan the service ran over each payload, written
/// out again independently of the writer's version so the two can disagree.
fn scan_payload(counters: &mut LedgerCounters, payload: &Value, timestamp_unix_ms: i64) {
    let empty = Vec::new();
    let list = |field: &str| payload.get(field).and_then(Value::as_array).unwrap_or(&empty).clone();
    let mut seen_stage = BTreeSet::new();
    for execution in list("plugin_executions") {
        let Some(plugin_id) = execution.get("plugin_id").and_then(Value::as_str) else {
            continue;
        };
        let stage = execution.get("stage").and_then(Value::as_str).unwrap_or("unknown");
        if !seen_stage.insert(format!("{plugin_id}:{stage}")) {
            continue;
        }
        let plugin = counters.plugins.entry(plugin_id.to_string()).or_default();
        plugin.executions += 1;
        if execution.get("applied").and_then(Value::as_bool) == Some(true) {
            plugin.applied += 1;
        } else {
            plugin.skipped += 1;
        }
        let duration = execution.get("duration_us").and_then(Value::as_u64).unwrap_or(0);
        plugin.total_duration_us += duration;
        plugin.max_duration_us = plugin.max_duration_us.max(duration);
    }
    let mut seen_detection = BTreeSet::new();
    for detection in list("detections") {
        if detection.get("source").and_then(Value::as_str) != Some("plugin") {
            continue;
        }
        if let Some(plugin_id) = detection.get("plugin_id").and_then(Value::as_str) {
            if seen_detection.insert(plugin_id.to_string()) {
                counters.plugins.entry(plugin_id.to_string()).or_default().detections += 1;
            }
        }
    }
    let mut seen_credential = BTreeSet::new();
    for (field, injected) in [("credential_observations", false), ("credential_injections", true)] {
        for item in list(field) {
            let Some(reference) = ["credential_ref", "substitution_ref", "reference", "ref"]
                .iter()
                .find_map(|key| item.get(key))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let source = item.get("source").and_then(Value::as_str).unwrap_or("");
            if !seen_credential.insert((field, reference.to_string(), source.to_string())) {
                continue;
            }
            let credential = counters.credentials.entry(reference.to_string()).or_default();
            if injected {
                credential.injections += 1;
            } else {
                credential.observations += 1;
            }
            let provider = item.get("provider").and_then(Value::as_str).map(str::to_string);
            credential.provider = credential.provider.clone().max(provider);
            credential.last_seen_unix_ms = credential.last_seen_unix_ms.max(timestamp_unix_ms);
        }
    }
}

fn tool_usage(row: &[Value]) -> ToolUsage {
    ToolUsage {
        calls: u(&row[0]),
        duration_ms: u(&row[1]),
        bytes_sent: u(&row[2]),
        bytes_received: u(&row[3]),
    }
}
