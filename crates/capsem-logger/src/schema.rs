use std::{collections::BTreeSet, path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension};

const MEMORY_SCHEMA: &str = "mem";
const DISK_ONLY_TABLES: &[&str] = &["event_body_blobs"];
static MEMORY_SCHEMA_LOCK: Mutex<()> = Mutex::new(());

const CREDENTIAL_REF_CHECK: &str =
    "CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))";
const SUBSTITUTION_REF_CHECK: &str =
    "CHECK (substitution_ref IS NULL OR (length(substitution_ref) = 82 AND substitution_ref GLOB 'credential:blake3:[0-9a-f]*'))";
const SUBSTITUTION_OUTCOME_CHECK: &str =
    "CHECK (outcome IN ('captured', 'brokered', 'injected', 'error'))";
const RULE_ACTION_CHECK: &str =
    "CHECK (rule_action IN ('allow', 'ask', 'block', 'preprocess', 'rewrite', 'postprocess'))";
const DETECTION_LEVEL_CHECK: &str =
    "CHECK (detection_level IN ('none', 'informational', 'low', 'medium', 'high', 'critical'))";
const ASK_STATUS_CHECK: &str = "CHECK (status IN ('pending', 'approved', 'denied'))";
const PROFILE_MUTATION_STATUS_CHECK: &str = "CHECK (status IN ('applied', 'failed'))";
const BLAKE3_REF_CHECK: &str =
    "CHECK (length(old_hash) = 71 AND old_hash GLOB 'blake3:[0-9a-f]*' AND length(new_hash) = 71 AND new_hash GLOB 'blake3:[0-9a-f]*')";
const SECURITY_DECISION_CHECK: &str = "CHECK (previous_decision IN ('allow', 'ask', 'block') AND requested_decision IN ('allow', 'ask', 'block') AND effective_decision IN ('allow', 'ask', 'block'))";
const SECURITY_DECISION_STAGE_CHECK: &str =
    "CHECK (stage IN ('preprocess', 'rule', 'rewrite', 'postprocess', 'ask_resolution'))";
const SECURITY_EVENT_TYPE_CHECK: &str =
    "CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask'))";
const SECURITY_EVENT_ID_CHECK: &str =
    "CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]')";
const MODEL_PROTOCOL_CHECK: &str =
    "CHECK (protocol IS NULL OR protocol IN ('anthropic', 'openai', 'google', 'ollama'))";

pub const CREATE_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS net_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        timestamp TEXT NOT NULL,
        domain TEXT NOT NULL,
        port INTEGER DEFAULT 443,
        decision TEXT NOT NULL,
        process_name TEXT,
        pid INTEGER,
        method TEXT,
        path TEXT,
        query TEXT,
        status_code INTEGER,
        bytes_sent INTEGER DEFAULT 0,
        bytes_received INTEGER DEFAULT 0,
        duration_ms INTEGER DEFAULT 0,
        matched_rule TEXT,
        request_headers TEXT,
        response_headers TEXT,
        request_body_preview TEXT,
        response_body_preview TEXT,
        conn_type TEXT DEFAULT 'https',
        policy_mode TEXT,
        policy_action TEXT,
        policy_rule TEXT,
        policy_reason TEXT,
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );

    CREATE TABLE IF NOT EXISTS model_calls (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        timestamp TEXT NOT NULL,
        provider TEXT NOT NULL,
        protocol TEXT CHECK (protocol IS NULL OR protocol IN ('anthropic', 'openai', 'google', 'ollama')),
        model TEXT,
        process_name TEXT,
        pid INTEGER,
        method TEXT NOT NULL,
        path TEXT NOT NULL,
        stream INTEGER DEFAULT 0,
        system_prompt_preview TEXT,
        messages_count INTEGER DEFAULT 0,
        tools_count INTEGER DEFAULT 0,
        request_bytes INTEGER DEFAULT 0,
        request_body_preview TEXT,
        message_id TEXT,
        status_code INTEGER,
        text_content TEXT,
        thinking_content TEXT,
        stop_reason TEXT,
        input_tokens INTEGER,
        output_tokens INTEGER,
        duration_ms INTEGER DEFAULT 0,
        response_bytes INTEGER DEFAULT 0,
        estimated_cost_usd REAL DEFAULT 0,
        trace_id TEXT,
        turn_id TEXT,
        usage_details TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );

    CREATE TABLE IF NOT EXISTS event_body_blobs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask')),
        source_table TEXT NOT NULL CHECK (source_table IN ('net_events', 'model_calls', 'tool_calls')),
        direction TEXT NOT NULL CHECK (direction IN ('request', 'response')),
        content_type TEXT,
        original_bytes INTEGER NOT NULL CHECK (original_bytes >= 0),
        stored_bytes INTEGER NOT NULL CHECK (stored_bytes >= 0 AND stored_bytes <= original_bytes),
        truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)),
        body_hash TEXT NOT NULL CHECK (length(body_hash) = 71 AND body_hash GLOB 'blake3:[0-9a-f]*'),
        body BLOB NOT NULL,
        trace_id TEXT,
        turn_id TEXT,
        created_at TEXT NOT NULL,
        UNIQUE(event_id, source_table, direction)
    );
    CREATE INDEX IF NOT EXISTS idx_event_body_blobs_event_id
        ON event_body_blobs(event_id);
    CREATE INDEX IF NOT EXISTS idx_event_body_blobs_trace_id
        ON event_body_blobs(trace_id);
    CREATE INDEX IF NOT EXISTS idx_event_body_blobs_hash
        ON event_body_blobs(body_hash);

    CREATE TABLE IF NOT EXISTS tool_calls (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        timestamp TEXT NOT NULL DEFAULT '',
        model_call_id INTEGER,
        provider TEXT NOT NULL DEFAULT '',
        status TEXT NOT NULL DEFAULT 'observed' CHECK (status IN ('requested', 'observed', 'responded', 'error')),
        call_index INTEGER NOT NULL,
        call_id TEXT NOT NULL,
        tool_name TEXT NOT NULL,
        arguments TEXT,
        response_preview TEXT,
        origin TEXT NOT NULL DEFAULT 'native',
        server_name TEXT,
        method TEXT,
        request_id TEXT,
        decision TEXT NOT NULL DEFAULT 'allowed',
        duration_ms INTEGER DEFAULT 0,
        error_message TEXT,
        process_name TEXT,
        bytes_sent INTEGER DEFAULT 0,
        bytes_received INTEGER DEFAULT 0,
        policy_mode TEXT,
        policy_action TEXT,
        policy_rule TEXT,
        policy_reason TEXT,
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );

    CREATE TABLE IF NOT EXISTS tool_responses (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        model_call_id INTEGER NOT NULL,
        call_id TEXT NOT NULL,
        content_preview TEXT,
        is_error INTEGER DEFAULT 0,
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );

    CREATE INDEX IF NOT EXISTS idx_net_events_domain
        ON net_events(domain);
    CREATE INDEX IF NOT EXISTS idx_net_events_timestamp
        ON net_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_model_calls_provider_ts
        ON model_calls(provider, timestamp);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_model_call
        ON tool_calls(model_call_id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_model_call
        ON tool_responses(model_call_id);
    CREATE INDEX IF NOT EXISTS idx_model_calls_trace_id
        ON model_calls(trace_id);

    CREATE TABLE IF NOT EXISTS model_items (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        model_call_id INTEGER NOT NULL,
        timestamp TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT,
        path TEXT NOT NULL,
        trace_id TEXT,
        turn_id TEXT,
        kind TEXT NOT NULL CHECK (kind IN ('request', 'reasoning', 'response', 'tool_call', 'tool_response')),
        item_index INTEGER NOT NULL,
        call_id TEXT NOT NULL DEFAULT '',
        tool_name TEXT,
        arguments TEXT,
        content TEXT,
        content_hash TEXT NOT NULL CHECK (length(content_hash) = 71 AND content_hash GLOB 'blake3:[0-9a-f]*'),
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*')),
        UNIQUE(trace_id, kind, content_hash, call_id)
    );
    CREATE INDEX IF NOT EXISTS idx_model_items_trace_id
        ON model_items(trace_id);
    CREATE INDEX IF NOT EXISTS idx_model_items_call_id
        ON model_items(call_id);
    CREATE INDEX IF NOT EXISTS idx_model_items_provider_path_model
        ON model_items(provider, path, model);

    CREATE INDEX IF NOT EXISTS idx_tool_calls_call_id
        ON tool_calls(call_id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_call_id
        ON tool_responses(call_id);

    CREATE TABLE IF NOT EXISTS fs_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        timestamp TEXT NOT NULL,
        action TEXT NOT NULL,
        path TEXT NOT NULL,
        directory TEXT,
        name TEXT,
        size INTEGER,
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );

    CREATE INDEX IF NOT EXISTS idx_fs_events_timestamp
        ON fs_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_fs_events_path
        ON fs_events(path);

    CREATE TABLE IF NOT EXISTS exec_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        timestamp TEXT NOT NULL,
        exec_id INTEGER NOT NULL,
        command TEXT NOT NULL,
        exit_code INTEGER,
        duration_ms INTEGER,
        stdout_preview TEXT,
        stderr_preview TEXT,
        stdout_bytes INTEGER DEFAULT 0,
        stderr_bytes INTEGER DEFAULT 0,
        source TEXT NOT NULL DEFAULT 'api',
        trace_id TEXT,
        turn_id TEXT,
        process_name TEXT,
        pid INTEGER,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );
    CREATE INDEX IF NOT EXISTS idx_exec_events_timestamp
        ON exec_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_exec_events_exec_id
        ON exec_events(exec_id);
    CREATE INDEX IF NOT EXISTS idx_exec_events_trace_id
        ON exec_events(trace_id);
    CREATE INDEX IF NOT EXISTS idx_exec_events_source
        ON exec_events(source);

    CREATE TABLE IF NOT EXISTS dns_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        timestamp TEXT NOT NULL,
        qname TEXT NOT NULL,
        qtype INTEGER NOT NULL,
        qclass INTEGER NOT NULL,
        rcode INTEGER NOT NULL,
        answer_ip TEXT,
        decision TEXT NOT NULL,
        matched_rule TEXT,
        source_proto TEXT,
        process_name TEXT,
        upstream_resolver_ms INTEGER DEFAULT 0,
        trace_id TEXT,
        turn_id TEXT,
        policy_mode TEXT,
        policy_action TEXT,
        policy_rule TEXT,
        policy_reason TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );
    CREATE INDEX IF NOT EXISTS idx_dns_events_timestamp
        ON dns_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_dns_events_qname
        ON dns_events(qname);
    CREATE INDEX IF NOT EXISTS idx_dns_events_trace_id
        ON dns_events(trace_id);
    CREATE INDEX IF NOT EXISTS idx_dns_events_decision
        ON dns_events(decision);
    CREATE INDEX IF NOT EXISTS idx_dns_events_policy_rule
        ON dns_events(policy_rule);

    CREATE TABLE IF NOT EXISTS audit_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        timestamp TEXT NOT NULL,
        pid INTEGER NOT NULL,
        ppid INTEGER NOT NULL,
        uid INTEGER NOT NULL,
        exe TEXT NOT NULL,
        comm TEXT,
        argv TEXT NOT NULL,
        cwd TEXT,
        exit_code INTEGER,
        session_id INTEGER,
        tty TEXT,
        audit_id TEXT,
        exec_event_id INTEGER,
        parent_exe TEXT,
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );
    CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp
        ON audit_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_audit_events_exe
        ON audit_events(exe);
    CREATE INDEX IF NOT EXISTS idx_audit_events_pid
        ON audit_events(pid);
    CREATE INDEX IF NOT EXISTS idx_audit_events_ppid
        ON audit_events(ppid);

    CREATE TABLE IF NOT EXISTS substitution_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        timestamp TEXT NOT NULL,
        material_class TEXT NOT NULL,
        source TEXT NOT NULL,
        event_type TEXT,
        algorithm TEXT NOT NULL,
        substitution_ref TEXT NOT NULL CHECK (length(substitution_ref) = 82 AND substitution_ref GLOB 'credential:blake3:[0-9a-f]*'),
        outcome TEXT NOT NULL CHECK (outcome IN ('captured', 'brokered', 'injected', 'error')),
        provider TEXT,
        confidence REAL,
        trace_id TEXT,
        turn_id TEXT,
        context_json TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_substitution_events_timestamp
        ON substitution_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_substitution_events_ref
        ON substitution_events(substitution_ref);
    CREATE INDEX IF NOT EXISTS idx_substitution_events_material
        ON substitution_events(material_class);

    CREATE TABLE IF NOT EXISTS security_rule_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp_unix_ms INTEGER NOT NULL,
        event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask')),
        rule_id TEXT NOT NULL,
        rule_action TEXT NOT NULL CHECK (rule_action IN ('allow', 'ask', 'block', 'preprocess', 'rewrite', 'postprocess')),
        detection_level TEXT NOT NULL DEFAULT 'none' CHECK (detection_level IN ('none', 'informational', 'low', 'medium', 'high', 'critical')),
        rule_json TEXT NOT NULL CHECK (json_valid(rule_json)),
        event_json TEXT NOT NULL CHECK (json_valid(event_json)),
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_timestamp
        ON security_rule_events(timestamp_unix_ms);
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_event_id
        ON security_rule_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_rule_id
        ON security_rule_events(rule_id);
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_event_type
        ON security_rule_events(event_type);

    CREATE TABLE IF NOT EXISTS security_decision_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp_unix_ms INTEGER NOT NULL,
        event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask')),
        stage TEXT NOT NULL CHECK (stage IN ('preprocess', 'rule', 'rewrite', 'postprocess', 'ask_resolution')),
        actor TEXT NOT NULL,
        rule_id TEXT,
        plugin_id TEXT,
        previous_decision TEXT NOT NULL CHECK (previous_decision IN ('allow', 'ask', 'block')),
        requested_decision TEXT NOT NULL CHECK (requested_decision IN ('allow', 'ask', 'block')),
        effective_decision TEXT NOT NULL CHECK (effective_decision IN ('allow', 'ask', 'block')),
        reason TEXT,
        event_json TEXT NOT NULL CHECK (json_valid(event_json)),
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
    );
    CREATE INDEX IF NOT EXISTS idx_security_decision_events_timestamp
        ON security_decision_events(timestamp_unix_ms);
    CREATE INDEX IF NOT EXISTS idx_security_decision_events_event_id
        ON security_decision_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_security_decision_events_actor
        ON security_decision_events(actor);

    CREATE TABLE IF NOT EXISTS security_ask_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp_unix_ms INTEGER NOT NULL,
        ask_id TEXT NOT NULL CHECK (length(ask_id) = 12 AND ask_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask')),
        rule_id TEXT NOT NULL,
        rule_name TEXT NOT NULL,
        status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'denied')),
        rule_json TEXT NOT NULL CHECK (json_valid(rule_json)),
        event_json TEXT NOT NULL CHECK (json_valid(event_json)),
        resolver TEXT,
        reason TEXT,
        trace_id TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_security_ask_events_timestamp
        ON security_ask_events(timestamp_unix_ms);
    CREATE INDEX IF NOT EXISTS idx_security_ask_events_ask_id
        ON security_ask_events(ask_id);
    CREATE INDEX IF NOT EXISTS idx_security_ask_events_event_id
        ON security_ask_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_security_ask_events_rule_id
        ON security_ask_events(rule_id);

    CREATE TABLE IF NOT EXISTS profile_mutation_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp_unix_ms INTEGER NOT NULL,
        mutation_id TEXT NOT NULL CHECK (length(mutation_id) = 12 AND mutation_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        profile_id TEXT NOT NULL,
        actor TEXT NOT NULL,
        category TEXT NOT NULL,
        filename TEXT NOT NULL,
        affected_path TEXT NOT NULL,
        target_kind TEXT NOT NULL,
        target_key TEXT NOT NULL,
        operation TEXT NOT NULL,
        rule_id TEXT,
        old_hash TEXT NOT NULL CHECK (length(old_hash) = 71 AND old_hash GLOB 'blake3:[0-9a-f]*'),
        old_size INTEGER NOT NULL,
        new_hash TEXT NOT NULL CHECK (length(new_hash) = 71 AND new_hash GLOB 'blake3:[0-9a-f]*'),
        new_size INTEGER NOT NULL,
        status TEXT NOT NULL CHECK (status IN ('applied', 'failed')),
        error TEXT,
        trace_id TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_profile_mutation_events_timestamp
        ON profile_mutation_events(timestamp_unix_ms);
    CREATE INDEX IF NOT EXISTS idx_profile_mutation_events_profile
        ON profile_mutation_events(profile_id);
    CREATE INDEX IF NOT EXISTS idx_profile_mutation_events_target
        ON profile_mutation_events(category, target_kind, target_key);
";

/// Create all tables and indexes on the given connection.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(CREATE_SCHEMA)
}

/// Attach the DB-owned in-memory schema and mirror hot ledger tables into it.
///
/// The canonical schema remains the disk schema. The memory schema is derived
/// from `main.sqlite_master` so table shape cannot drift into a second hand
/// written contract. Blob storage stays disk-owned and bounded.
pub fn memory_uri_for_path(path: &Path) -> String {
    memory_uri_for_name(&path.to_string_lossy())
}

pub fn memory_uri_for_name(name: &str) -> String {
    let hash = blake3::hash(name.as_bytes()).to_hex();
    format!(
        "file:capsem-ledger-mem-{}?mode=memory&cache=shared",
        &hash[..16]
    )
}

pub(crate) fn with_memory_schema_lock<T>(
    operation: impl FnOnce() -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let _guard = MEMORY_SCHEMA_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    operation()
}

pub fn create_memory_tables(conn: &Connection, memory_uri: &str) -> rusqlite::Result<()> {
    attach_memory_schema(conn, memory_uri)?;
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {MEMORY_SCHEMA}.__capsem_memory_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );"
    ))?;

    let mut stmt = conn.prepare(
        "SELECT name, sql
         FROM main.sqlite_master
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let tables = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for table in tables {
        let (name, sql) = table?;
        if is_disk_only_table(&name) {
            continue;
        }
        let mem_sql = memory_table_sql(&name, &sql)
            .ok_or_else(|| rusqlite::Error::InvalidParameterName(name.clone()))?;
        conn.execute_batch(&mem_sql)?;
    }

    Ok(())
}

pub fn create_memory_read_views(conn: &Connection) -> rusqlite::Result<()> {
    for (table, _) in READY_SCHEMA_COLUMNS {
        if is_disk_only_table(table) {
            continue;
        }
        if !table_exists(conn, MEMORY_SCHEMA, table)? {
            continue;
        }
        conn.execute_batch(&format!(
            "CREATE TEMP VIEW IF NOT EXISTS {table} AS SELECT * FROM {MEMORY_SCHEMA}.{table};"
        ))?;
    }
    Ok(())
}

pub fn sync_memory_tables_from_disk<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
) -> rusqlite::Result<()> {
    for table in tables {
        if is_disk_only_table(table) {
            continue;
        }
        if !table_exists(conn, "main", table)? || !table_exists(conn, MEMORY_SCHEMA, table)? {
            continue;
        }
        conn.execute_batch(&format!(
            "DELETE FROM {MEMORY_SCHEMA}.{table};
             INSERT OR REPLACE INTO {MEMORY_SCHEMA}.{table}
             SELECT * FROM main.{table};"
        ))?;
    }
    Ok(())
}

pub fn rehydrate_memory_tables_from_disk_once<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
) -> rusqlite::Result<()> {
    let already_rehydrated = conn
        .query_row(
            &format!(
                "SELECT value FROM {MEMORY_SCHEMA}.__capsem_memory_state
                 WHERE key = 'rehydrated' LIMIT 1"
            ),
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some();
    if already_rehydrated {
        return Ok(());
    }
    sync_memory_tables_from_disk(conn, tables)?;
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {MEMORY_SCHEMA}.__capsem_memory_state (key, value)
             VALUES ('rehydrated', '1')"
        ),
        [],
    )?;
    Ok(())
}

fn table_exists(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<bool> {
    let query = if schema == "main" {
        "SELECT 1 FROM main.sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1".to_string()
    } else {
        format!("SELECT 1 FROM {schema}.sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1")
    };
    let found = conn
        .query_row(&query, [table], |_| Ok(()))
        .optional()?
        .is_some();
    Ok(found)
}

fn attach_memory_schema(conn: &Connection, memory_uri: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("PRAGMA database_list")?;
    let databases = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for database in databases {
        if database? == MEMORY_SCHEMA {
            return Ok(());
        }
    }
    let escaped_uri = memory_uri.replace('\'', "''");
    conn.execute_batch(&format!(
        "ATTACH DATABASE '{escaped_uri}' AS {MEMORY_SCHEMA}"
    ))
}

pub(crate) fn is_disk_only_table(name: &str) -> bool {
    DISK_ONLY_TABLES.contains(&name)
}

pub(crate) fn hot_ledger_tables() -> BTreeSet<&'static str> {
    READY_SCHEMA_COLUMNS
        .iter()
        .filter_map(|(table, _)| (!is_disk_only_table(table)).then_some(*table))
        .collect()
}

fn memory_table_sql(table: &str, sql: &str) -> Option<String> {
    let create = format!("CREATE TABLE {table}");
    let create_if_not_exists = format!("CREATE TABLE IF NOT EXISTS {table}");
    if let Some(rest) = sql.strip_prefix(&create_if_not_exists) {
        return Some(format!(
            "CREATE TABLE IF NOT EXISTS {MEMORY_SCHEMA}.{table}{rest}"
        ));
    }
    sql.strip_prefix(&create)
        .map(|rest| format!("CREATE TABLE IF NOT EXISTS {MEMORY_SCHEMA}.{table}{rest}"))
}

/// Apply write-mode pragmas: WAL journal + relaxed synchronous.
/// Only call on read-write connections (the writer).
pub fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

const READY_SCHEMA_COLUMNS: &[(&str, &[&str])] = &[
    (
        "net_events",
        &[
            "event_id",
            "timestamp",
            "domain",
            "decision",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "model_calls",
        &[
            "event_id",
            "provider",
            "protocol",
            "method",
            "path",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "model_items",
        &[
            "event_id",
            "model_call_id",
            "kind",
            "content_hash",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "tool_calls",
        &[
            "event_id",
            "model_call_id",
            "origin",
            "call_id",
            "tool_name",
            "decision",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "tool_responses",
        &[
            "model_call_id",
            "call_id",
            "content_preview",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "event_body_blobs",
        &[
            "event_id",
            "event_type",
            "source_table",
            "direction",
            "body_hash",
            "body",
            "trace_id",
            "turn_id",
        ],
    ),
    (
        "fs_events",
        &[
            "event_id",
            "timestamp",
            "action",
            "path",
            "directory",
            "name",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "exec_events",
        &[
            "event_id",
            "timestamp",
            "exec_id",
            "command",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "dns_events",
        &[
            "event_id",
            "timestamp",
            "qname",
            "qtype",
            "rcode",
            "decision",
            "answer_ip",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "audit_events",
        &[
            "event_id",
            "timestamp",
            "pid",
            "exe",
            "trace_id",
            "turn_id",
            "credential_ref",
        ],
    ),
    (
        "substitution_events",
        &[
            "event_id",
            "timestamp",
            "substitution_ref",
            "outcome",
            "provider",
            "trace_id",
        ],
    ),
    (
        "security_rule_events",
        &[
            "event_id",
            "rule_id",
            "rule_action",
            "detection_level",
            "rule_json",
            "event_json",
            "credential_ref",
        ],
    ),
    (
        "security_decision_events",
        &["event_id", "stage", "effective_decision", "credential_ref"],
    ),
    (
        "security_ask_events",
        &["event_id", "ask_id", "status", "event_json", "trace_id"],
    ),
    (
        "profile_mutation_events",
        &["mutation_id", "profile_id", "status"],
    ),
];

/// Validate that a session DB is structurally ready for ledger routes.
///
/// This intentionally fails on missing tables or columns. A valid empty DB is
/// ready; a partially migrated or corrupted DB is not. Routes must surface this
/// as a DB contract error rather than returning invented empty ledgers.
pub fn validate_ready_schema(conn: &Connection) -> Result<(), String> {
    let integrity = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(|error| format!("session db integrity check failed: {error}"))?;
    if integrity != "ok" {
        return Err(format!("session db integrity check failed: {integrity}"));
    }

    for (table, required_columns) in READY_SCHEMA_COLUMNS {
        validate_table_columns(conn, "main", table, required_columns)?;
        if !is_disk_only_table(table) {
            validate_table_columns(conn, MEMORY_SCHEMA, table, required_columns)?;
        }
    }

    Ok(())
}

fn validate_table_columns(
    conn: &Connection,
    schema: &str,
    table: &str,
    required_columns: &[&str],
) -> Result<(), String> {
    let pragma = if schema == "main" {
        format!("PRAGMA table_info({table})")
    } else {
        format!("PRAGMA {schema}.table_info({table})")
    };
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?;
    if columns.is_empty() {
        return Err(format!(
            "session db missing required table {schema}.{table}"
        ));
    }
    for column in required_columns {
        if !columns.contains(*column) {
            return Err(format!(
                "session db table {schema}.{table} missing required column {column}"
            ));
        }
    }
    Ok(())
}

fn table_sql(conn: &Connection, table: &str) -> Option<String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn column_is_not_null(conn: &Connection, table: &str, column: &str) -> bool {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(stmt) => stmt,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))
    }) {
        Ok(rows) => rows,
        Err(_) => return false,
    };
    for row in rows.flatten() {
        if row.0 == column {
            return row.1 != 0;
        }
    }
    false
}

fn rebuild_tool_calls_nullable_model_call(conn: &Connection) {
    if !column_is_not_null(conn, "tool_calls", "model_call_id") {
        return;
    }
    let _ = conn.execute_batch(
        "DROP TABLE IF EXISTS tool_calls_new;
        CREATE TABLE tool_calls_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
            timestamp TEXT NOT NULL DEFAULT '',
            model_call_id INTEGER,
            provider TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'observed' CHECK (status IN ('requested', 'observed', 'responded', 'error')),
            call_index INTEGER NOT NULL,
            call_id TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            arguments TEXT,
            response_preview TEXT,
            origin TEXT NOT NULL DEFAULT 'native',
            server_name TEXT,
            method TEXT,
            request_id TEXT,
            decision TEXT NOT NULL DEFAULT 'allowed',
            duration_ms INTEGER DEFAULT 0,
            error_message TEXT,
            process_name TEXT,
            bytes_sent INTEGER DEFAULT 0,
            bytes_received INTEGER DEFAULT 0,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT,
            trace_id TEXT,
            turn_id TEXT,
            credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))
        );
        INSERT INTO tool_calls_new (
            id, event_id, timestamp, model_call_id, provider, status, call_index, call_id,
            tool_name, arguments, response_preview, origin, server_name, method,
            request_id, decision, duration_ms, error_message, process_name, bytes_sent,
            bytes_received, policy_mode, policy_action, policy_rule, policy_reason,
            trace_id, turn_id, credential_ref
        )
        SELECT
            id, event_id, timestamp, model_call_id, provider, status, call_index, call_id,
            tool_name, arguments, response_preview, origin, server_name, method,
            request_id, decision, duration_ms, error_message, process_name, bytes_sent,
            bytes_received, policy_mode, policy_action, policy_rule, policy_reason,
            trace_id, turn_id, credential_ref
        FROM tool_calls;
        DROP TABLE tool_calls;
        ALTER TABLE tool_calls_new RENAME TO tool_calls;",
    );
}

fn rebuild_event_body_blobs_source_check(conn: &Connection) {
    let Some(sql) = table_sql(conn, "event_body_blobs") else {
        return;
    };
    if sql.contains("'tool_calls'") {
        return;
    }
    let _ = conn.execute_batch(
        "DROP TABLE IF EXISTS event_body_blobs_new;
        CREATE TABLE event_body_blobs_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
            event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask')),
            source_table TEXT NOT NULL CHECK (source_table IN ('net_events', 'model_calls', 'tool_calls')),
            direction TEXT NOT NULL CHECK (direction IN ('request', 'response')),
            content_type TEXT,
            original_bytes INTEGER NOT NULL CHECK (original_bytes >= 0),
            stored_bytes INTEGER NOT NULL CHECK (stored_bytes >= 0 AND stored_bytes <= original_bytes),
            truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)),
            body_hash TEXT NOT NULL CHECK (length(body_hash) = 71 AND body_hash GLOB 'blake3:[0-9a-f]*'),
            body BLOB NOT NULL,
            trace_id TEXT,
            turn_id TEXT,
            created_at TEXT NOT NULL,
            UNIQUE(event_id, source_table, direction)
        );
        INSERT INTO event_body_blobs_new (
            id, event_id, event_type, source_table, direction, content_type,
            original_bytes, stored_bytes, truncated, body_hash, body, trace_id, turn_id, created_at
        )
        SELECT
            id, event_id, event_type, source_table, direction, content_type,
            original_bytes, stored_bytes, truncated, body_hash, body, trace_id, turn_id, created_at
        FROM event_body_blobs;
        DROP TABLE event_body_blobs;
        ALTER TABLE event_body_blobs_new RENAME TO event_body_blobs;",
    );
}

/// Migrate existing databases to add new columns/tables.
/// Idempotent: safe to call on databases that already have the changes.
pub fn migrate(conn: &Connection) {
    for tbl in [
        "net_events",
        "model_calls",
        "model_items",
        "tool_calls",
        "tool_responses",
        "event_body_blobs",
        "fs_events",
        "exec_events",
        "dns_events",
        "audit_events",
        "substitution_events",
        "security_rule_events",
        "security_decision_events",
        "security_ask_events",
    ] {
        let _ = conn.execute(&format!("ALTER TABLE {tbl} ADD COLUMN turn_id TEXT"), []);
        let _ = conn.execute(
            &format!("CREATE INDEX IF NOT EXISTS idx_{tbl}_turn_id ON {tbl}(turn_id)"),
            [],
        );
    }
    for tbl in ["security_rule_events", "security_decision_events"] {
        let _ = conn.execute(
            &format!("ALTER TABLE {tbl} ADD COLUMN credential_ref TEXT {CREDENTIAL_REF_CHECK}"),
            [],
        );
        let _ = conn.execute(
            &format!(
                "CREATE INDEX IF NOT EXISTS idx_{tbl}_credential_ref ON {tbl}(credential_ref)"
            ),
            [],
        );
    }
    let _ = conn.execute("ALTER TABLE model_calls ADD COLUMN trace_id TEXT", []);
    let _ = conn.execute(
        &format!("ALTER TABLE model_calls ADD COLUMN protocol TEXT {MODEL_PROTOCOL_CHECK}"),
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_model_calls_trace_id ON model_calls(trace_id)",
        [],
    );
    // Replace cache_read_tokens with usage_details TEXT column.
    // SQLite doesn't support DROP COLUMN before 3.35, so just add the new one.
    let _ = conn.execute("ALTER TABLE model_calls ADD COLUMN usage_details TEXT", []);
    // Add unified tool ledger columns to tool_calls (for DBs created before this feature).
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN origin TEXT NOT NULL DEFAULT 'native'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN event_id TEXT NOT NULL DEFAULT '000000000000' CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]')",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN timestamp TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN provider TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN status TEXT NOT NULL DEFAULT 'observed' CHECK (status IN ('requested', 'observed', 'responded', 'error'))",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN response_preview TEXT",
        [],
    );
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN server_name TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN method TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN request_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN decision TEXT NOT NULL DEFAULT 'allowed'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN duration_ms INTEGER DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN error_message TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN process_name TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN bytes_sent INTEGER DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN bytes_received INTEGER DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN policy_mode TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN policy_action TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN policy_rule TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN policy_reason TEXT", []);
    rebuild_tool_calls_nullable_model_call(conn);
    // Add policy decision metadata to net_events for security rule HTTP/DNS audit.
    let _ = conn.execute("ALTER TABLE net_events ADD COLUMN policy_mode TEXT", []);
    let _ = conn.execute("ALTER TABLE net_events ADD COLUMN policy_action TEXT", []);
    let _ = conn.execute("ALTER TABLE net_events ADD COLUMN policy_rule TEXT", []);
    let _ = conn.execute("ALTER TABLE net_events ADD COLUMN policy_reason TEXT", []);
    // Add indexes for tool_calls/tool_responses call_id lookups.
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tool_calls_call_id ON tool_calls(call_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tool_responses_call_id ON tool_responses(call_id)",
        [],
    );
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS model_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
            model_call_id INTEGER NOT NULL,
            timestamp TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT,
            path TEXT NOT NULL,
            trace_id TEXT,
            kind TEXT NOT NULL CHECK (kind IN ('request', 'reasoning', 'response', 'tool_call', 'tool_response')),
            item_index INTEGER NOT NULL,
            call_id TEXT NOT NULL DEFAULT '',
            tool_name TEXT,
            arguments TEXT,
            content TEXT,
            content_hash TEXT NOT NULL CHECK (length(content_hash) = 71 AND content_hash GLOB 'blake3:[0-9a-f]*'),
            credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*')),
            UNIQUE(trace_id, kind, content_hash, call_id)
        );
        CREATE INDEX IF NOT EXISTS idx_model_items_trace_id ON model_items(trace_id);
        CREATE INDEX IF NOT EXISTS idx_model_items_call_id ON model_items(call_id);
        CREATE INDEX IF NOT EXISTS idx_model_items_provider_path_model ON model_items(provider, path, model);",
    );
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_body_blobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
            event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask')),
            source_table TEXT NOT NULL CHECK (source_table IN ('net_events', 'model_calls', 'tool_calls')),
            direction TEXT NOT NULL CHECK (direction IN ('request', 'response')),
            content_type TEXT,
            original_bytes INTEGER NOT NULL CHECK (original_bytes >= 0),
            stored_bytes INTEGER NOT NULL CHECK (stored_bytes >= 0 AND stored_bytes <= original_bytes),
            truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)),
            body_hash TEXT NOT NULL CHECK (length(body_hash) = 71 AND body_hash GLOB 'blake3:[0-9a-f]*'),
            body BLOB NOT NULL,
            trace_id TEXT,
            created_at TEXT NOT NULL,
            UNIQUE(event_id, source_table, direction)
        );
        CREATE INDEX IF NOT EXISTS idx_event_body_blobs_event_id ON event_body_blobs(event_id);
        CREATE INDEX IF NOT EXISTS idx_event_body_blobs_trace_id ON event_body_blobs(trace_id);
        CREATE INDEX IF NOT EXISTS idx_event_body_blobs_hash ON event_body_blobs(body_hash);",
    );
    rebuild_event_body_blobs_source_check(conn);
    // Add fs_events table if not present (for DBs created before this feature).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fs_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            action TEXT NOT NULL,
            path TEXT NOT NULL,
            directory TEXT,
            name TEXT,
            size INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_fs_events_timestamp ON fs_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_fs_events_path ON fs_events(path);",
    );
    // Snapshot metadata is host recovery state, not session.db activity.
    let _ = conn.execute_batch("DROP TABLE IF EXISTS snapshot_events;");
    // Add exec_events table if not present (for DBs created before this feature).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS exec_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            exec_id INTEGER NOT NULL,
            command TEXT NOT NULL,
            exit_code INTEGER,
            duration_ms INTEGER,
            stdout_preview TEXT,
            stderr_preview TEXT,
            stdout_bytes INTEGER DEFAULT 0,
            stderr_bytes INTEGER DEFAULT 0,
            source TEXT NOT NULL DEFAULT 'api',
            trace_id TEXT,
            process_name TEXT,
            pid INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_exec_events_timestamp ON exec_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_exec_events_exec_id ON exec_events(exec_id);
        CREATE INDEX IF NOT EXISTS idx_exec_events_trace_id ON exec_events(trace_id);
        CREATE INDEX IF NOT EXISTS idx_exec_events_source ON exec_events(source);",
    );
    // T3.3: Add dns_events table if not present (for DBs created before
    // T3 landed). The host-side DNS proxy writes one row per resolved
    // query; trace_id correlates back to the same agent action that
    // emitted the corresponding net_events / model_calls rows.
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dns_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            qname TEXT NOT NULL,
            qtype INTEGER NOT NULL,
            qclass INTEGER NOT NULL,
            rcode INTEGER NOT NULL,
            answer_ip TEXT,
            decision TEXT NOT NULL,
            matched_rule TEXT,
            source_proto TEXT,
            process_name TEXT,
            upstream_resolver_ms INTEGER DEFAULT 0,
            trace_id TEXT,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_dns_events_timestamp ON dns_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_dns_events_qname ON dns_events(qname);
        CREATE INDEX IF NOT EXISTS idx_dns_events_trace_id ON dns_events(trace_id);
        CREATE INDEX IF NOT EXISTS idx_dns_events_decision ON dns_events(decision);
        CREATE INDEX IF NOT EXISTS idx_dns_events_policy_rule ON dns_events(policy_rule);",
    );
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN policy_mode TEXT", []);
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN policy_action TEXT", []);
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN policy_rule TEXT", []);
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN policy_reason TEXT", []);
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN answer_ip TEXT", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_dns_events_policy_rule ON dns_events(policy_rule)",
        [],
    );
    let _ = conn.execute("ALTER TABLE fs_events ADD COLUMN directory TEXT", []);
    let _ = conn.execute("ALTER TABLE fs_events ADD COLUMN name TEXT", []);

    // Add audit_events table if not present (for DBs created before this feature).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS audit_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            pid INTEGER NOT NULL,
            ppid INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            exe TEXT NOT NULL,
            comm TEXT,
            argv TEXT NOT NULL,
            cwd TEXT,
            exit_code INTEGER,
            session_id INTEGER,
            tty TEXT,
            audit_id TEXT,
            exec_event_id INTEGER,
            parent_exe TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp ON audit_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_audit_events_exe ON audit_events(exe);
        CREATE INDEX IF NOT EXISTS idx_audit_events_pid ON audit_events(pid);
        CREATE INDEX IF NOT EXISTS idx_audit_events_ppid ON audit_events(ppid);",
    );

    // W6: trace_id everywhere. Adding the column to the seven tables that
    // didn't already have it lets `capsem_timeline --trace_id <X>` join
    // every event class for one logical user action. NULL for rows that
    // pre-date W4's trace propagation; downstream queries handle that
    // gracefully (`WHERE trace_id = ? OR trace_id IS NULL`).
    for tbl in [
        "net_events",
        "fs_events",
        "tool_calls",
        "tool_responses",
        "audit_events",
    ] {
        let _ = conn.execute(&format!("ALTER TABLE {tbl} ADD COLUMN trace_id TEXT"), []);
        let _ = conn.execute(
            &format!("CREATE INDEX IF NOT EXISTS idx_{tbl}_trace_id ON {tbl}(trace_id)"),
            [],
        );
    }

    for tbl in [
        "net_events",
        "model_calls",
        "fs_events",
        "exec_events",
        "tool_responses",
        "dns_events",
        "audit_events",
    ] {
        let _ = conn.execute(
            &format!("ALTER TABLE {tbl} ADD COLUMN credential_ref TEXT {CREDENTIAL_REF_CHECK}"),
            [],
        );
        let _ = conn.execute(
            &format!(
                "CREATE INDEX IF NOT EXISTS idx_{tbl}_credential_ref ON {tbl}(credential_ref)"
            ),
            [],
        );
    }

    for tbl in [
        "net_events",
        "model_calls",
        "fs_events",
        "exec_events",
        "dns_events",
        "audit_events",
        "substitution_events",
    ] {
        let _ = conn.execute(&format!("ALTER TABLE {tbl} ADD COLUMN event_id TEXT"), []);
        let _ = conn.execute(
            &format!("CREATE INDEX IF NOT EXISTS idx_{tbl}_event_id ON {tbl}(event_id)"),
            [],
        );
    }

    let _ = conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS substitution_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            material_class TEXT NOT NULL,
            source TEXT NOT NULL,
            event_type TEXT,
            algorithm TEXT NOT NULL,
            substitution_ref TEXT NOT NULL {SUBSTITUTION_REF_CHECK},
            outcome TEXT NOT NULL {SUBSTITUTION_OUTCOME_CHECK},
            provider TEXT,
            confidence REAL,
            trace_id TEXT,
            context_json TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_substitution_events_timestamp
            ON substitution_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_substitution_events_ref
            ON substitution_events(substitution_ref);
        CREATE INDEX IF NOT EXISTS idx_substitution_events_material
            ON substitution_events(material_class);"
    ));

    let _ = conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS security_rule_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_unix_ms INTEGER NOT NULL,
            event_id TEXT NOT NULL {SECURITY_EVENT_ID_CHECK},
            event_type TEXT NOT NULL {SECURITY_EVENT_TYPE_CHECK},
            rule_id TEXT NOT NULL,
            rule_action TEXT NOT NULL {RULE_ACTION_CHECK},
            detection_level TEXT NOT NULL DEFAULT 'none' {DETECTION_LEVEL_CHECK},
            rule_json TEXT NOT NULL CHECK (json_valid(rule_json)),
            event_json TEXT NOT NULL CHECK (json_valid(event_json)),
            trace_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_security_rule_events_timestamp
            ON security_rule_events(timestamp_unix_ms);
        CREATE INDEX IF NOT EXISTS idx_security_rule_events_event_id
            ON security_rule_events(event_id);
        CREATE INDEX IF NOT EXISTS idx_security_rule_events_rule_id
            ON security_rule_events(rule_id);
        CREATE INDEX IF NOT EXISTS idx_security_rule_events_event_type
            ON security_rule_events(event_type);"
    ));
    let _ = conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS security_decision_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_unix_ms INTEGER NOT NULL,
            event_id TEXT NOT NULL {SECURITY_EVENT_ID_CHECK},
            event_type TEXT NOT NULL {SECURITY_EVENT_TYPE_CHECK},
            stage TEXT NOT NULL {SECURITY_DECISION_STAGE_CHECK},
            actor TEXT NOT NULL,
            rule_id TEXT,
            plugin_id TEXT,
            previous_decision TEXT NOT NULL,
            requested_decision TEXT NOT NULL,
            effective_decision TEXT NOT NULL,
            reason TEXT,
            event_json TEXT NOT NULL CHECK (json_valid(event_json)),
            trace_id TEXT,
            {SECURITY_DECISION_CHECK}
        );
        CREATE INDEX IF NOT EXISTS idx_security_decision_events_timestamp
            ON security_decision_events(timestamp_unix_ms);
        CREATE INDEX IF NOT EXISTS idx_security_decision_events_event_id
            ON security_decision_events(event_id);
        CREATE INDEX IF NOT EXISTS idx_security_decision_events_actor
            ON security_decision_events(actor);"
    ));
    let _ = conn.execute(
        "ALTER TABLE security_rule_events ADD COLUMN rule_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(rule_json))",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE security_rule_events ADD COLUMN event_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(event_json))",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE security_rule_events ADD COLUMN detection_level TEXT NOT NULL DEFAULT 'none' CHECK (detection_level IN ('none', 'informational', 'low', 'medium', 'high', 'critical'))",
        [],
    );

    let _ = conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS security_ask_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_unix_ms INTEGER NOT NULL,
            ask_id TEXT NOT NULL {SECURITY_EVENT_ID_CHECK},
            event_id TEXT NOT NULL {SECURITY_EVENT_ID_CHECK},
            event_type TEXT NOT NULL {SECURITY_EVENT_TYPE_CHECK},
            rule_id TEXT NOT NULL,
            rule_name TEXT NOT NULL,
            status TEXT NOT NULL {ASK_STATUS_CHECK},
            rule_json TEXT NOT NULL CHECK (json_valid(rule_json)),
            event_json TEXT NOT NULL CHECK (json_valid(event_json)),
            resolver TEXT,
            reason TEXT,
            trace_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_security_ask_events_timestamp
            ON security_ask_events(timestamp_unix_ms);
        CREATE INDEX IF NOT EXISTS idx_security_ask_events_ask_id
            ON security_ask_events(ask_id);
        CREATE INDEX IF NOT EXISTS idx_security_ask_events_event_id
            ON security_ask_events(event_id);
        CREATE INDEX IF NOT EXISTS idx_security_ask_events_rule_id
            ON security_ask_events(rule_id);"
    ));
    let _ = conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS profile_mutation_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_unix_ms INTEGER NOT NULL,
            mutation_id TEXT NOT NULL {SECURITY_EVENT_ID_CHECK},
            profile_id TEXT NOT NULL,
            actor TEXT NOT NULL,
            category TEXT NOT NULL,
            filename TEXT NOT NULL,
            affected_path TEXT NOT NULL,
            target_kind TEXT NOT NULL,
            target_key TEXT NOT NULL,
            operation TEXT NOT NULL,
            rule_id TEXT,
            old_hash TEXT NOT NULL,
            old_size INTEGER NOT NULL,
            new_hash TEXT NOT NULL,
            new_size INTEGER NOT NULL,
            status TEXT NOT NULL {PROFILE_MUTATION_STATUS_CHECK},
            error TEXT,
            trace_id TEXT,
            {BLAKE3_REF_CHECK}
        );
        CREATE INDEX IF NOT EXISTS idx_profile_mutation_events_timestamp
            ON profile_mutation_events(timestamp_unix_ms);
        CREATE INDEX IF NOT EXISTS idx_profile_mutation_events_profile
            ON profile_mutation_events(profile_id);
        CREATE INDEX IF NOT EXISTS idx_profile_mutation_events_target
            ON profile_mutation_events(category, target_kind, target_key);"
    ));
}

/// Apply read-safe pragmas for DB-owned query connections.
///
/// These connections may be opened read-write briefly so the DB layer can
/// attach and populate its private `mem` schema. After setup, `query_only`
/// prevents writes through the read worker.
pub fn apply_reader_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "query_only", "ON")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OpenFlags;

    fn columns_for_schema(conn: &Connection, schema: &str, table: &str) -> BTreeSet<String> {
        let pragma = if schema == "main" {
            format!("PRAGMA table_info({table})")
        } else {
            format!("PRAGMA {schema}.table_info({table})")
        };
        let mut stmt = conn.prepare(&pragma).unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<BTreeSet<_>, _>>()
            .unwrap()
    }

    #[test]
    fn create_tables_succeeds() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
    }

    #[test]
    fn create_tables_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        create_tables(&conn).unwrap();
    }

    #[test]
    fn db_mem_tables_match_schema() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        create_memory_tables(&conn, &memory_uri_for_name("db_mem_tables_match_schema")).unwrap();

        for (table, _) in READY_SCHEMA_COLUMNS {
            let main_columns = columns_for_schema(&conn, "main", table);
            if is_disk_only_table(table) {
                let mem_columns = columns_for_schema(&conn, MEMORY_SCHEMA, table);
                assert!(
                    mem_columns.is_empty(),
                    "{table} must stay disk-only; blob payloads are bounded durable storage, not DB-owned hot memory tables"
                );
                continue;
            }
            let mem_columns = columns_for_schema(&conn, MEMORY_SCHEMA, table);
            assert_eq!(
                mem_columns, main_columns,
                "{MEMORY_SCHEMA}.{table} must mirror main.{table}; memory schema is derived from the canonical disk schema"
            );
        }
    }

    #[test]
    fn db_mem_disk_ready_rejects_missing_memory_schema() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);

        let error = validate_ready_schema(&conn)
            .expect_err("ready() must fail if DB-owned memory tables were not created");
        assert!(
            error.contains("mem.net_events"),
            "missing memory schema must fail loudly instead of route projections hiding stale state: {error}"
        );
    }

    #[test]
    fn db_mem_disk_memory_tables_work_before_query_only_guard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        {
            let conn = Connection::open(&path).unwrap();
            apply_pragmas(&conn).unwrap();
            create_tables(&conn).unwrap();
            migrate(&conn);
        }

        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&path, flags).unwrap();
        create_memory_tables(
            &conn,
            &memory_uri_for_name("db_mem_disk_memory_tables_work_before_query_only_guard"),
        )
        .unwrap();
        apply_reader_pragmas(&conn).unwrap();
        validate_ready_schema(&conn)
            .expect("query-only connection must still own its DB-local memory schema");
        let error = conn
            .execute("INSERT INTO mem.net_events (timestamp, domain, decision) VALUES ('t', 'example.com', 'allowed')", [])
            .expect_err("query_only must prevent writes after DB-owned memory setup");
        assert!(
            error.to_string().contains("readonly"),
            "query_only should make the reader worker effectively read-only after setup: {error}"
        );
    }

    #[test]
    fn apply_pragmas_succeeds() {
        let conn = Connection::open_in_memory().unwrap();
        apply_pragmas(&conn).unwrap();
    }

    #[test]
    fn migrate_trace_columns_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        // Run twice -- second call must not error.
        migrate(&conn);
        migrate(&conn);
        // Verify trace_id column exists by inserting a row with it.
        conn.execute(
            "INSERT INTO model_calls (timestamp, provider, method, path, trace_id)
             VALUES ('2024-01-01T00:00:00Z', 'test', 'POST', '/v1', 'trace_abc')",
            [],
        )
        .unwrap();
        let trace_id: String = conn
            .query_row(
                "SELECT trace_id FROM model_calls WHERE trace_id = 'trace_abc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trace_id, "trace_abc");
    }

    #[test]
    fn create_tables_includes_fs_events() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn.execute(
            "INSERT INTO fs_events (timestamp, action, path, size)
             VALUES ('2026-01-01T00:00:00Z', 'created', 'project/app.js', 1234)",
            [],
        )
        .unwrap();
        let action: String = conn
            .query_row(
                "SELECT action FROM fs_events WHERE path = 'project/app.js'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(action, "created");
    }

    #[test]
    fn migrate_fs_events_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        migrate(&conn);
        conn.execute(
            "INSERT INTO fs_events (timestamp, action, path)
             VALUES ('2026-01-01T00:00:00Z', 'deleted', 'project/old.txt')",
            [],
        )
        .unwrap();
        let path: String = conn
            .query_row(
                "SELECT path FROM fs_events WHERE action = 'deleted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(path, "project/old.txt");
    }

    #[test]
    fn migrate_tool_calls_origin_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        migrate(&conn);
        // Verify origin/server/method columns exist by inserting one unified MCP-origin row.
        conn.execute(
            "INSERT INTO model_calls (timestamp, provider, method, path)
             VALUES ('2024-01-01T00:00:00Z', 'test', 'POST', '/v1')",
            [],
        )
        .unwrap();
        let mc_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tool_calls (
                model_call_id, call_index, call_id, tool_name, origin, server_name, method
             ) VALUES (?1, 0, 'call_01', 'fetch_http', 'mcp', 'local', 'tools/call')",
            [mc_id],
        )
        .unwrap();
        let origin: String = conn
            .query_row(
                "SELECT origin FROM tool_calls WHERE call_id = 'call_01'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(origin, "mcp");
    }

    #[test]
    fn migrate_tool_calls_allows_orphan_mcp_origin_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                model_call_id INTEGER NOT NULL,
                call_index INTEGER NOT NULL,
                call_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                arguments TEXT
            );",
        )
        .unwrap();

        migrate(&conn);
        migrate(&conn);

        conn.execute(
            "INSERT INTO tool_calls (
                event_id, timestamp, model_call_id, provider, status, call_index,
                call_id, tool_name, arguments, response_preview, origin, server_name,
                method, request_id, decision, duration_ms
             ) VALUES (
                '012345abcdef', '2026-01-01T00:00:00Z', NULL, '', 'responded', 0,
                'mcp_01', 'fetch_http', '{\"url\":\"http://127.0.0.1\"}',
                'Status: 200 OK', 'mcp', 'local', 'tools/call', 'req-1', 'allowed', 7
             )",
            [],
        )
        .unwrap();

        let row: (Option<i64>, String, String) = conn
            .query_row(
                "SELECT model_call_id, origin, response_preview FROM tool_calls WHERE call_id = 'mcp_01'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, None);
        assert_eq!(row.1, "mcp");
        assert_eq!(row.2, "Status: 200 OK");
    }

    #[test]
    fn migrate_event_body_blobs_accepts_tool_calls_source() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE event_body_blobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL CHECK (length(event_id) = 12),
                event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call')),
                source_table TEXT NOT NULL CHECK (source_table IN ('net_events', 'model_calls')),
                direction TEXT NOT NULL CHECK (direction IN ('request', 'response')),
                content_type TEXT,
                original_bytes INTEGER NOT NULL CHECK (original_bytes >= 0),
                stored_bytes INTEGER NOT NULL CHECK (stored_bytes >= 0 AND stored_bytes <= original_bytes),
                truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)),
                body_hash TEXT NOT NULL CHECK (length(body_hash) = 71),
                body BLOB NOT NULL,
                trace_id TEXT,
                created_at TEXT NOT NULL,
                UNIQUE(event_id, source_table, direction)
            );",
        )
        .unwrap();

        migrate(&conn);
        migrate(&conn);

        conn.execute(
            "INSERT INTO event_body_blobs (
                event_id, event_type, source_table, direction, content_type,
                original_bytes, stored_bytes, truncated, body_hash, body,
                trace_id, created_at
             ) VALUES (
                '012345abcdef', 'mcp.tool_call', 'tool_calls', 'request',
                'application/json', 2, 2, 0,
                'blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                '{}', 'trace-1', '2026-01-01T00:00:00Z'
             )",
            [],
        )
        .unwrap();

        let source: String = conn
            .query_row(
                "SELECT source_table FROM event_body_blobs WHERE event_id = '012345abcdef'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source, "tool_calls");
    }

    #[test]
    fn create_tables_include_shared_credential_ref_columns() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        for table in [
            "net_events",
            "model_calls",
            "fs_events",
            "exec_events",
            "dns_events",
            "audit_events",
            "tool_calls",
            "tool_responses",
            "security_rule_events",
            "security_decision_events",
        ] {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert!(
                cols.iter().any(|col| col == "credential_ref"),
                "{table} missing top-level shared credential_ref column: {cols:?}"
            );
        }
    }

    #[test]
    fn create_tables_include_shared_turn_id_columns() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        for table in [
            "net_events",
            "model_calls",
            "model_items",
            "tool_calls",
            "tool_responses",
            "event_body_blobs",
            "fs_events",
            "exec_events",
            "dns_events",
            "audit_events",
            "security_rule_events",
            "security_decision_events",
        ] {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert!(
                cols.iter().any(|col| col == "turn_id"),
                "{table} missing first-class turn_id column: {cols:?}"
            );
        }
    }

    #[test]
    fn create_tables_include_shared_event_id_columns() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        for table in [
            "net_events",
            "model_calls",
            "fs_events",
            "exec_events",
            "dns_events",
            "audit_events",
            "substitution_events",
            "security_rule_events",
        ] {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert!(
                cols.iter().any(|col| col == "event_id"),
                "{table} missing shared event_id column: {cols:?}"
            );
        }
    }

    #[test]
    fn model_calls_include_strict_protocol_column() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(model_calls)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert!(
            cols.iter().any(|col| col == "protocol"),
            "model_calls must carry model wire protocol separately from provider: {cols:?}"
        );

        conn.execute(
            "INSERT INTO model_calls (timestamp, provider, protocol, method, path)
             VALUES ('2024-01-01T00:00:00Z', 'unknown', 'openai', 'POST', '/v1/chat/completions')",
            [],
        )
        .unwrap();
        let err = conn
            .execute(
                "INSERT INTO model_calls (timestamp, provider, protocol, method, path)
                 VALUES ('2024-01-01T00:00:00Z', 'unknown', 'madeup', 'POST', '/v1/chat/completions')",
                [],
            )
            .expect_err("unknown model wire protocols must be rejected");
        assert!(err.to_string().contains("CHECK"));
    }

    #[test]
    fn create_tables_reject_raw_credential_ref_values() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let err = conn
            .execute(
                "INSERT INTO net_events (
                    timestamp, domain, decision, credential_ref
                 ) VALUES (
                    '2026-01-01T00:00:00Z', 'api.github.com', 'allowed', 'ghp_raw_secret'
                 )",
                [],
            )
            .expect_err("raw credentials must not be accepted as credential_ref");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );
    }

    #[test]
    fn substitution_events_require_brokered_reference() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        conn.execute(
            "INSERT INTO substitution_events (
                timestamp, material_class, source, event_type,
                algorithm, substitution_ref, outcome
             ) VALUES (
                '2026-01-01T00:00:00Z', 'credential', 'http.authorization',
                'http.request', 'blake3',
                'credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                'captured'
             )",
            [],
        )
        .unwrap();

        let err = conn
            .execute(
                "INSERT INTO substitution_events (
                    timestamp, material_class, source, algorithm,
                    substitution_ref, outcome
                 ) VALUES (
                    '2026-01-01T00:00:00Z', 'credential', 'http.authorization',
                    'blake3', 'Bearer raw-secret', 'captured'
                 )",
                [],
            )
            .expect_err("substitution_ref must be a brokered reference");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );

        for outcome in ["substituted", "ignored"] {
            let err = conn
                .execute(
                    "INSERT INTO substitution_events (
                        timestamp, material_class, source, event_type,
                        algorithm, substitution_ref, outcome
                     ) VALUES (
                        '2026-01-01T00:00:00Z', 'credential', 'http.authorization',
                        'http.request', 'blake3',
                        'credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                        ?1
                     )",
                    [outcome],
                )
                .expect_err("substitution_events outcome must be a closed broker verb");
            assert!(
                err.to_string().contains("CHECK"),
                "expected CHECK constraint failure for outcome {outcome}, got: {err}"
            );
        }
    }

    #[test]
    fn create_tables_includes_security_rule_events_contract() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        conn.execute(
            "INSERT INTO security_rule_events (
                timestamp_unix_ms, event_id, event_type, rule_id,
                rule_action, detection_level, rule_json, event_json
             ) VALUES (
                1789000000000, 'abcdef123456', 'model.call',
                'openai_api_block', 'block', 'critical',
                '{\"name\":\"openai_api_block\",\"match\":\"model.provider == \\\"openai\\\"\"}',
                '{\"common\":{\"event_type\":\"model.call\"},\"model\":{\"provider\":\"openai\"}}'
             )",
            [],
        )
        .unwrap();

        let (event_id, rule_action, detection_level): (String, String, String) = conn
            .query_row(
                "SELECT event_id, rule_action, detection_level
                 FROM security_rule_events WHERE rule_id = 'openai_api_block'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(event_id, "abcdef123456");
        assert_eq!(rule_action, "block");
        assert_eq!(detection_level, "critical");
    }

    #[test]
    fn create_tables_includes_security_ask_events_contract() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        conn.execute(
            "INSERT INTO security_ask_events (
                timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                rule_name, status, rule_json, event_json
             ) VALUES (
                1789000000000, 'abcdef123456', '111111abcdef',
                'http.request', 'profiles.rules.ask_openai', 'ask_openai',
                'pending', '{\"name\":\"ask_openai\"}',
                '{\"http\":{\"host\":\"api.openai.com\"}}'
             )",
            [],
        )
        .unwrap();

        let err = conn
            .execute(
                "INSERT INTO security_ask_events (
                    timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123457', '111111abcdeg',
                    'http.request', 'profiles.rules.ask_openai', 'ask_openai',
                    'maybe', '{}', '{}'
                 )",
                [],
            )
            .expect_err("ask status and ids must be strict");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );
    }

    #[test]
    fn security_rule_events_reject_unknown_rule_action() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let err = conn
            .execute(
                "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'old_detect', 'detect', '{}', '{}'
                 )",
                [],
            )
            .expect_err("detect is not a rule action");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );
    }

    #[test]
    fn security_rule_events_accept_rewrite_rule_action() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        conn.execute(
            "INSERT INTO security_rule_events (
                timestamp_unix_ms, event_id, event_type, rule_id,
                rule_action, rule_json, event_json
             ) VALUES (
                1789000000000, 'abcdef123456', 'model.call',
                'profiles.rules.redact_model', 'rewrite', '{}', '{}'
             )",
            [],
        )
        .expect("rewrite is a canonical stored action");
    }

    #[test]
    fn security_decision_events_record_explicit_decisions_and_reject_magic_outcome() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        conn.execute(
            "INSERT INTO security_decision_events (
                timestamp_unix_ms, event_id, event_type, stage, actor,
                rule_id, plugin_id, previous_decision, requested_decision,
                effective_decision, reason, event_json
             ) VALUES (
                1789000000000, 'abcdef123456', 'file.import', 'rewrite',
                'dummy_pre_eicar', 'profiles.rules.scan_eicar', 'dummy_pre_eicar',
                'allow', 'block', 'block', 'EICAR test seed observed', '{}'
             )",
            [],
        )
        .expect("explicit decision transition must persist");

        let err = conn
            .execute(
                "INSERT INTO security_decision_events (
                    timestamp_unix_ms, event_id, event_type, stage, actor,
                    previous_decision, requested_decision, effective_decision,
                    event_json
                 ) VALUES (
                    1789000000001, 'abcdef123457', 'file.import', 'rewrite',
                    'dummy_pre_eicar', 'allow', 'outcome', 'block', '{}'
                 )",
                [],
            )
            .expect_err("requested_decision must be an explicit decision, not magic outcome");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );

        let err = conn
            .execute(
                "INSERT INTO security_decision_events (
                    timestamp_unix_ms, event_id, event_type, stage, actor,
                    previous_decision, requested_decision, effective_decision,
                    event_json
                 ) VALUES (
                    1789000002, 'abcdef123458', 'file.import', 'mystery',
                    'dummy_pre_eicar', 'allow', 'block', 'block', '{}'
                 )",
                [],
            )
            .expect_err("stage must be canonical");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );
    }

    #[test]
    fn security_rule_events_reject_non_hex_event_id() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let err = conn
            .execute(
                "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'evt_abc123', 'model.call',
                    'bad_event_id', 'allow', '{}', '{}'
                 )",
                [],
            )
            .expect_err("event_id must be 12 lowercase hex characters");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );
    }

    #[test]
    fn security_rule_events_reject_unknown_event_type() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        for event_type in ["dns.response", "model.request", "file.ingress"] {
            let err = conn
                .execute(
                    "INSERT INTO security_rule_events (
                        timestamp_unix_ms, event_id, event_type, rule_id,
                        rule_action, rule_json, event_json
                     ) VALUES (
                        1789000000000, 'abcdef123456', ?1,
                        'stale_event_type', 'allow', '{}', '{}'
                     )",
                    [event_type],
                )
                .expect_err("event_type must be a backed runtime event type");
            assert!(
                err.to_string().contains("CHECK"),
                "expected CHECK constraint failure for {event_type}, got: {err}"
            );
        }
    }

    #[test]
    fn security_ask_events_reject_unknown_event_type() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let err = conn
            .execute(
                "INSERT INTO security_ask_events (
                    timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', '111111abcdef',
                    'model.request', 'profiles.rules.ask_model', 'ask_model',
                    'pending', '{}', '{}'
                 )",
                [],
            )
            .expect_err("ask event_type must be a backed runtime event type");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );
    }

    #[test]
    fn security_rule_events_reject_unknown_detection_level() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let err = conn
            .execute(
                "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'bad_level', 'allow', 'info', '{}', '{}'
                 )",
                [],
            )
            .expect_err("DB stores only canonical detection levels");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );
    }

    #[test]
    fn security_rule_events_reject_null_detection_level() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let err = conn
            .execute(
                "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'ambiguous_level', 'allow', NULL, '{}', '{}'
                 )",
                [],
            )
            .expect_err("detection_level must be explicit none, not NULL");
        assert!(
            err.to_string().contains("NOT NULL") || err.to_string().contains("CHECK"),
            "expected NOT NULL/CHECK constraint failure, got: {err}"
        );
    }

    #[test]
    fn security_rule_events_reject_non_json_forensic_payloads() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let err = conn
            .execute(
                "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'bad_payload', 'allow', 'not json', '{}'
                 )",
                [],
            )
            .expect_err("rule_json must be valid JSON");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure, got: {err}"
        );
    }

    /// Writer pragmas (WAL + synchronous) must only be applied to read-write
    /// connections. Read-only connections must use apply_reader_pragmas instead.
    #[test]
    fn reader_pragmas_work_on_readonly_connection() {
        // Create a file-backed DB first (writer sets WAL).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        {
            let conn = Connection::open(&path).unwrap();
            apply_pragmas(&conn).unwrap();
            create_tables(&conn).unwrap();
        }

        // Open read-only -- apply_reader_pragmas must not fail.
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&path, flags).unwrap();
        apply_reader_pragmas(&conn).unwrap();
    }

    #[test]
    fn create_tables_keeps_snapshots_out_of_session_db() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='snapshot_events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "snapshots are host recovery state; session.db is the user/security activity ledger"
        );
    }

    #[test]
    fn security_event_type_check_rejects_snapshot_event() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        let result = conn.execute(
            "INSERT INTO security_rule_events (
                timestamp_unix_ms, event_id, event_type, rule_id, rule_name,
                rule_action, detection_level, provider, rule_snapshot, event_payload
             ) VALUES (
                1, 'abcdef123456', 'snapshot.event', 'profiles.rules.snapshot',
                'snapshot', 'allow', 'none', 'profiles', '{}', '{}'
             )",
            [],
        );
        assert!(
            result.is_err(),
            "snapshot.event must not be a security-event type"
        );
    }

    #[test]
    fn create_tables_includes_dns_events() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn.execute(
            "INSERT INTO dns_events (
                timestamp, qname, qtype, qclass, rcode, decision,
                policy_mode, policy_action, policy_rule, policy_reason
             )
             VALUES (
                '2026-01-01T00:00:00Z', 'anthropic.com', 1, 1, 0, 'allowed',
                'enforce', 'allow', 'policy.dns.allow_example', 'allowed by dns policy'
             )",
            [],
        )
        .unwrap();
        let (qname, policy_rule): (String, String) = conn
            .query_row(
                "SELECT qname, policy_rule FROM dns_events WHERE decision = 'allowed'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(qname, "anthropic.com");
        assert_eq!(policy_rule, "policy.dns.allow_example");
    }

    #[test]
    fn migrate_dns_events_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        // Run migrate twice -- second call must not error.
        migrate(&conn);
        migrate(&conn);
        // Verify dns_events table exists and accepts a row.
        conn.execute(
            "INSERT INTO dns_events (timestamp, qname, qtype, qclass, rcode, decision, trace_id)
             VALUES ('2026-01-01T00:00:00Z', 'pypi.org', 1, 1, 0, 'allowed', 'tr_abc')",
            [],
        )
        .unwrap();
        let trace: String = conn
            .query_row(
                "SELECT trace_id FROM dns_events WHERE qname = 'pypi.org'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trace, "tr_abc");
    }

    #[test]
    fn dns_events_has_indexes() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        for idx in [
            "idx_dns_events_timestamp",
            "idx_dns_events_qname",
            "idx_dns_events_trace_id",
            "idx_dns_events_decision",
            "idx_dns_events_policy_rule",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name = ?1",
                    [idx],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing index {idx}");
        }
    }
}
