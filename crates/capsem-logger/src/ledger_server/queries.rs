//! Named query intent executed only by the ledger owner.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::ledger_protocol::{LedgerHistoryLayer, LedgerQuery, LedgerRows, LedgerTimelineLayer, LedgerValue};
use crate::DbHandle;

type Statement = (String, Vec<Value>);

pub(super) async fn execute(db: &DbHandle, query: LedgerQuery) -> Result<Vec<LedgerRows>, String> {
    let statements = statements(query)?;
    let raw = db.query_many(statements).await?;
    raw.into_iter().map(|rows| decode_rows(&rows)).collect()
}

fn statements(query: LedgerQuery) -> Result<Vec<Statement>, String> {
    match query {
        LedgerQuery::SecurityLatest { limit, detection_only } => {
            let detection = if detection_only {
                "WHERE event.detection_level != 'none'"
            } else {
                ""
            };
            Ok(vec![(
                format!(
                    "SELECT event.timestamp_unix_ms, event.event_id, event.event_type, event.rule_id, \
                     event.rule_action, event.detection_level, COALESCE(event.rule_json, run.rule_json) AS rule_json, \
                     event.trace_id, event.turn_id, event.credential_ref FROM security_rule_events AS event \
                     LEFT JOIN security_rule_runs AS run ON run.id = event.run_id {detection} \
                     ORDER BY event.timestamp_unix_ms DESC, event.id DESC LIMIT ?1"
                ),
                vec![json!(limit)],
            )])
        }
        LedgerQuery::Timeline {
            layers,
            cutoff,
            trace_id,
            limit,
        } => {
            let tool_cutoff = if cutoff.as_str() <= "1970-01-01T00:00:00Z" {
                String::new()
            } else {
                cutoff.clone()
            };
            Ok(vec![(
                timeline_sql(&layers),
                vec![
                    json!(limit),
                    json!(cutoff),
                    json!(tool_cutoff),
                    trace_id.map_or(Value::Null, Value::String),
                ],
            )])
        }
        LedgerQuery::History {
            layers,
            search,
            limit,
            offset,
        } => history_statements(&layers, search, limit, offset),
        LedgerQuery::StatsDetail => Ok(stats_detail_statements()),
        LedgerQuery::Triage { limit } => Ok(triage_statements(limit)),
        LedgerQuery::Commitments {
            after_global_sequence,
            limit,
        } => Ok(vec![(
            "SELECT global_sequence, lower(hex(generation)) AS generation, client_id, producer_role, \
             producer_sequence, event_kind, lower(hex(event_hash)) AS event_hash, \
             lower(hex(previous_hash)) AS previous_hash, lower(hex(commitment_hash)) AS commitment_hash \
             FROM ledger_commitments WHERE global_sequence > ?1 ORDER BY global_sequence ASC LIMIT ?2"
                .into(),
            vec![json!(after_global_sequence), json!(limit)],
        )]),
    }
}

const EXEC_WINDOW: &str = "SELECT timestamp, 'exec' AS layer, exec_id AS ref, command AS summary, \
    exit_code AS status, duration_ms, trace_id FROM exec_events WHERE timestamp >= ?2 \
    AND (?4 IS NULL OR trace_id = ?4 OR trace_id IS NULL) ORDER BY timestamp ASC LIMIT ?1";
const TOOL_WINDOW: &str = "SELECT COALESCE(NULLIF(tc.timestamp, ''), '1970-01-01T00:00:00Z') AS timestamp, \
    'tool' AS layer, tc.event_id AS ref, COALESCE(tc.server_name, tc.origin) || '/' || tc.tool_name || \
    COALESCE(' (call_id=' || tc.call_id || ')', '') AS summary, tc.decision AS status, tc.duration_ms, tc.trace_id \
    FROM tool_calls tc WHERE +tc.origin IN ('model', 'native', 'mcp', 'builtin', 'local', 'mcp_proxy') \
    AND tc.timestamp >= ?3 AND (?4 IS NULL OR tc.trace_id = ?4 OR tc.trace_id IS NULL) \
    ORDER BY tc.timestamp ASC LIMIT ?1";
const NET_WINDOW: &str = "SELECT timestamp, 'net' AS layer, id AS ref, COALESCE(method, 'GET') || ' ' || \
    domain || COALESCE(path, '') AS summary, status_code AS status, duration_ms, trace_id FROM net_events \
    WHERE timestamp >= ?2 AND (?4 IS NULL OR trace_id = ?4 OR trace_id IS NULL) \
    ORDER BY timestamp ASC LIMIT ?1";
const FILE_WINDOW: &str = "SELECT timestamp, 'fs' AS layer, id AS ref, action || ' ' || path AS summary, \
    NULL AS status, NULL AS duration_ms, trace_id FROM fs_events WHERE timestamp >= ?2 \
    AND (?4 IS NULL OR trace_id = ?4 OR trace_id IS NULL) ORDER BY timestamp ASC LIMIT ?1";
const MODEL_WINDOW: &str = "SELECT timestamp, 'model' AS layer, id AS ref, provider || '/' || COALESCE(model, '?') \
    AS summary, status_code AS status, duration_ms, trace_id FROM model_calls WHERE timestamp >= ?2 \
    AND (?4 IS NULL OR trace_id = ?4 OR trace_id IS NULL) ORDER BY timestamp ASC LIMIT ?1";

fn timeline_sql(layers: &[LedgerTimelineLayer]) -> String {
    let windows = layers
        .iter()
        .map(|layer| match layer {
            LedgerTimelineLayer::Exec => EXEC_WINDOW,
            LedgerTimelineLayer::Tool => TOOL_WINDOW,
            LedgerTimelineLayer::Net => NET_WINDOW,
            LedgerTimelineLayer::File => FILE_WINDOW,
            LedgerTimelineLayer::Model => MODEL_WINDOW,
        })
        .map(|window| format!("SELECT * FROM ({window})"))
        .collect::<Vec<_>>();
    format!("{} ORDER BY timestamp ASC LIMIT ?1", windows.join(" UNION ALL "))
}

const EXEC_HISTORY: &str = "SELECT timestamp, 'exec' AS layer, command, exit_code, duration_ms, stdout_preview, \
    stderr_preview, json_object('source', source, 'trace_id', trace_id, 'process_name', process_name, \
    'exec_id', exec_id) AS details FROM exec_events";
const AUDIT_HISTORY: &str = "SELECT timestamp, 'audit' AS layer, argv AS command, exit_code, NULL AS duration_ms, \
    NULL AS stdout_preview, NULL AS stderr_preview, json_object('pid', pid, 'ppid', ppid, 'uid', uid, 'exe', exe, \
    'comm', comm, 'cwd', cwd, 'tty', tty, 'session_id', session_id, 'audit_id', audit_id, 'parent_exe', parent_exe) \
    AS details FROM audit_events";
const HISTORY_SEARCH: &str = " WHERE instr(command, ?1) > 0 OR instr(stdout_preview, ?1) > 0 \
    OR instr(stderr_preview, ?1) > 0 OR instr(details, ?1) > 0";

fn history_statements(
    layers: &[LedgerHistoryLayer],
    search: Option<String>,
    limit: u16,
    offset: u64,
) -> Result<Vec<Statement>, String> {
    let has_search = search.is_some();
    let arms = layers
        .iter()
        .map(|layer| match layer {
            LedgerHistoryLayer::Exec => EXEC_HISTORY,
            LedgerHistoryLayer::Audit => AUDIT_HISTORY,
        })
        .collect::<Vec<_>>();
    let filtered = |rows: &str| format!("SELECT * FROM ({rows}){}", if has_search { HISTORY_SEARCH } else { "" });
    let page = arms
        .iter()
        .map(|rows| format!("SELECT * FROM ({} ORDER BY timestamp DESC LIMIT ?2)", filtered(rows)))
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    let params = vec![
        search.clone().map_or(Value::Null, Value::String),
        json!(i64::try_from(offset.saturating_add(u64::from(limit))).unwrap_or(i64::MAX)),
        json!(limit),
        json!(i64::try_from(offset).unwrap_or(i64::MAX)),
    ];
    let mut statements = vec![(format!("{page} ORDER BY timestamp DESC LIMIT ?3 OFFSET ?4"), params)];
    if let Some(search) = search {
        let total = arms
            .iter()
            .map(|rows| filtered(rows))
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        statements.push((format!("SELECT COUNT(*) AS total FROM ({total})"), vec![json!(search)]));
    }
    Ok(statements)
}

const BODY_BLOBS: &str = "SELECT event_id, source_table, direction, content_type, original_bytes, stored_bytes, \
    truncated, body_hash FROM event_body_blobs WHERE source_table NOT IN ('security_decision_events', \
    'security_ask_events') AND (event_id IN (SELECT event_id FROM net_events WHERE event_id IS NOT NULL ORDER BY id \
    DESC LIMIT 200) OR event_id IN (SELECT event_id FROM model_calls WHERE event_id IS NOT NULL ORDER BY id DESC \
    LIMIT 200) OR event_id IN (SELECT event_id FROM tool_calls WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200) \
    OR event_id IN (SELECT event_id FROM tool_responses WHERE call_id IN (SELECT call_id FROM tool_calls WHERE event_id \
    IS NOT NULL ORDER BY id DESC LIMIT 200)) OR event_id IN (SELECT event_id FROM exec_events ORDER BY id DESC LIMIT \
    ?1) OR event_id IN (SELECT event_id FROM security_rule_events ORDER BY id DESC LIMIT 200)) ORDER BY event_id, direction";
const MODEL_ITEMS: &str = "SELECT mi.event_id, mi.timestamp, mi.model_call_id, mc.event_id AS model_event_id, \
    mi.trace_id, mi.turn_id, mi.item_index, mi.kind, mi.call_id, substr(mi.content, 1, ?1) AS content, \
    COALESCE(length(mi.content) > ?1, 0) AS content_truncated, (SELECT CASE WHEN MAX(tr.is_error NOT IN (0, 1)) = 1 \
    THEN 2 WHEN COUNT(DISTINCT tr.is_error) = 1 THEN MIN(tr.is_error) ELSE NULL END FROM tool_responses tr WHERE \
    tr.model_call_id = mi.model_call_id AND tr.call_id = mi.call_id) AS is_error FROM model_items mi LEFT JOIN \
    model_calls mc ON mc.id = mi.model_call_id WHERE mi.kind != 'tool_call' ORDER BY mi.id DESC LIMIT 200";
const TOOL_ITEMS: &str = "SELECT tc.event_id, COALESCE(NULLIF(tc.timestamp, ''), mc.timestamp, '') AS timestamp, \
    tc.model_call_id, mc.event_id AS model_event_id, tc.trace_id, tc.turn_id, tc.call_id, tc.tool_name, tc.server_name, \
    tc.origin, tc.decision, tc.method, substr(tc.arguments, 1, ?1) AS arguments, COALESCE(length(tc.arguments) > ?1, 0) \
    AS arguments_truncated, tc.response_preview, tc.error_message FROM tool_calls tc LEFT JOIN model_calls mc ON mc.id \
    = tc.model_call_id ORDER BY tc.id DESC LIMIT 200";

fn stats_detail_statements() -> Vec<Statement> {
    let field = || vec![json!(16 * 1024)];
    vec![
        (BODY_BLOBS.into(), vec![json!(100)]),
        (MODEL_ITEMS.into(), field()),
        (TOOL_ITEMS.into(), field()),
        ("SELECT event_id, timestamp, provider, model, method, path, status_code, input_tokens, output_tokens, duration_ms, response_bytes, stop_reason, trace_id, credential_ref FROM model_calls ORDER BY id DESC LIMIT 200".into(), vec![]),
        ("SELECT tc.event_id, COALESCE(NULLIF(tc.timestamp, ''), mc.timestamp) AS timestamp, tc.process_name, COALESCE(tc.server_name, 'model') AS server_name, tc.tool_name, tc.method, tc.call_id, tc.model_call_id, CASE WHEN tc.model_call_id IS NOT NULL AND mc.id IS NULL THEN 1 ELSE 0 END AS model_parent_missing, tc.decision, COALESCE(tc.duration_ms, mc.duration_ms, 0) AS duration_ms, COALESCE(LENGTH(tc.arguments), 0) + COALESCE(LENGTH(COALESCE(tc.response_preview, tr.content_preview)), 0) AS bytes, substr(tc.arguments, 1, ?1) AS arguments, COALESCE(tc.response_preview, tr.content_preview) AS response_preview, tr.event_id AS response_event_id, tc.error_message, tc.origin AS source, COALESCE(tc.credential_ref, tr.credential_ref) AS credential_ref FROM tool_calls AS tc NOT INDEXED LEFT JOIN model_calls mc ON tc.model_call_id = mc.id LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id WHERE tc.origin IN ('model', 'native', 'mcp', 'builtin', 'local', 'mcp_proxy') ORDER BY tc.id DESC LIMIT 200".into(), field()),
        ("SELECT event_id, timestamp, domain, port, method, path, query, status_code, decision, duration_ms, bytes_sent, bytes_received, matched_rule, policy_rule, trace_id, credential_ref, request_headers, response_headers FROM net_events ORDER BY id DESC LIMIT 200".into(), vec![]),
        ("SELECT event_id, timestamp, qname, qtype, qclass, rcode, decision, matched_rule, policy_rule, source_proto, process_name, upstream_resolver_ms, trace_id, credential_ref FROM dns_events ORDER BY id DESC LIMIT 200".into(), vec![]),
        ("SELECT event_id, timestamp, action, path, size, trace_id, credential_ref FROM fs_events ORDER BY id DESC LIMIT 200".into(), vec![]),
        ("SELECT event_id, timestamp, exec_id, command, exit_code, duration_ms, stdout_bytes, stderr_bytes, source, process_name, pid, trace_id, credential_ref FROM exec_events ORDER BY id DESC LIMIT 100".into(), vec![]),
        ("SELECT event_id, timestamp, pid, ppid, uid, exe, comm, argv, cwd, exit_code, session_id, tty, audit_id, exec_event_id, parent_exe, trace_id, credential_ref FROM audit_events ORDER BY id DESC LIMIT 100".into(), vec![]),
        ("SELECT event_id, timestamp, material_class, source, event_type, event_type AS origin, outcome AS verb, provider, trace_id, context_json FROM substitution_events ORDER BY id DESC LIMIT 100".into(), vec![]),
    ]
}

fn triage_statements(limit: u16) -> Vec<Statement> {
    let limit = u64::from(limit);
    vec![
        (format!("SELECT timestamp, domain, decision, status_code, duration_ms FROM net_events WHERE decision = 'denied' OR status_code >= 500 ORDER BY timestamp DESC LIMIT {limit}"), vec![]),
        (format!("SELECT timestamp, server_name, method, decision, policy_mode, policy_action, policy_rule, policy_reason, error_message, duration_ms FROM tool_calls NOT INDEXED WHERE origin IN ('native','mcp','builtin','local') AND (decision IN ('denied','error') OR error_message IS NOT NULL) ORDER BY id DESC LIMIT {limit}"), vec![]),
        (format!("SELECT timestamp, exec_id, command, exit_code, duration_ms FROM exec_events WHERE exit_code IS NOT NULL AND exit_code != 0 ORDER BY timestamp DESC LIMIT {limit}"), vec![]),
    ]
}

#[derive(Deserialize)]
struct JsonRows {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

fn decode_rows(raw: &str) -> Result<LedgerRows, String> {
    let JsonRows { columns, rows } = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    let rows = rows
        .into_iter()
        .map(|row| row.into_iter().map(value).collect())
        .collect::<Result<Vec<_>, _>>()?;
    let rows = LedgerRows { columns, rows };
    rows.validate().map_err(|error| error.to_string())?;
    Ok(rows)
}

fn value(value: Value) -> Result<LedgerValue, String> {
    match value {
        Value::Null => Ok(LedgerValue::Null),
        Value::Bool(value) => Ok(LedgerValue::Integer(i64::from(value))),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(LedgerValue::Integer(value))
            } else if let Some(value) = value.as_f64() {
                Ok(LedgerValue::Real(value))
            } else {
                Err(format!("ledger query returned unsupported JSON number {value}"))
            }
        }
        Value::String(value) => Ok(LedgerValue::Text(value)),
        other => Err(format!("ledger query returned unsupported JSON value {other}")),
    }
}
