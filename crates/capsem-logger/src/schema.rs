use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Mutex,
};

use rusqlite::{Connection, OptionalExtension};

const MEMORY_SCHEMA: &str = "mem";
const DISK_ONLY_TABLES: &[&str] = &["event_body_blobs"];
static MEMORY_SCHEMA_LOCK: Mutex<()> = Mutex::new(());

const CREDENTIAL_REF_CHECK: &str =
    "CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*'))";
const SUBSTITUTION_REF_CHECK: &str =
    "CHECK (substitution_ref IS NULL OR (length(substitution_ref) = 82 AND substitution_ref GLOB 'credential:blake3:[0-9a-f]*'))";
const SUBSTITUTION_OUTCOME_CHECK: &str = "CHECK (outcome IN ('captured', 'brokered', 'injected', 'error'))";
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
        transport TEXT NOT NULL DEFAULT 'unknown' CHECK (transport IN ('http', 'sse', 'websocket', 'vsock_frame', 'direct', 'unknown')),
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
        trace_id TEXT,
        turn_id TEXT
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
mod memory_sync;
#[cfg(test)]
pub(crate) use memory_sync::UPDATABLE_HOT_TABLES;
pub use memory_sync::{
    flush_memory_tables_to_disk, rehydrate_memory_tables_from_disk_once, sync_memory_tables_from_disk,
};
pub(crate) use memory_sync::{initial_memory_flush_watermarks, MemoryFlushWatermarks};

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
    format!("file:capsem-ledger-mem-{}?mode=memory&cache=shared", &hash[..16])
}

pub(crate) fn with_memory_schema_lock<T>(operation: impl FnOnce() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let _guard = MEMORY_SCHEMA_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    operation()
}

pub fn create_memory_tables(conn: &Connection, memory_uri: &str) -> rusqlite::Result<()> {
    attach_memory_schema(conn, memory_uri)?;
    reconcile_memory_tables_from_disk(conn)
}

/// Reconcile the attached DB-owned memory schema with the current disk schema.
///
/// An external reader can observe `session.db` after SQLite creates the file but
/// before the writer process finishes its canonical DDL.  The reader must not
/// freeze that partial snapshot for the rest of the service lifetime.  This
/// function is intentionally DB-owned: route callers neither inspect nor repair
/// ledger schema.
pub fn reconcile_memory_tables_from_disk(conn: &Connection) -> rusqlite::Result<()> {
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
    let tables = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    for table in tables {
        let (name, sql) = table;
        if is_disk_only_table(&name) {
            continue;
        }
        let disk_columns = table_column_names(conn, "main", &name)?;
        let memory_columns = table_column_names(conn, MEMORY_SCHEMA, &name)?;
        if !memory_columns.is_empty() && memory_columns != disk_columns {
            conn.execute_batch(&format!(
                "DROP VIEW IF EXISTS temp.{name};
                 DROP TABLE {MEMORY_SCHEMA}.{name};"
            ))?;
        }
        let mem_sql =
            memory_table_sql(&name, &sql).ok_or_else(|| rusqlite::Error::InvalidParameterName(name.clone()))?;
        conn.execute_batch(&mem_sql)?;
    }

    Ok(())
}

fn table_column_names(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns)
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

fn table_exists(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<bool> {
    let query = if schema == "main" {
        "SELECT 1 FROM main.sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1".to_string()
    } else {
        format!("SELECT 1 FROM {schema}.sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1")
    };
    let found = conn.query_row(&query, [table], |_| Ok(())).optional()?.is_some();
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
    conn.execute_batch(&format!("ATTACH DATABASE '{escaped_uri}' AS {MEMORY_SCHEMA}"))
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

fn canonical_hot_table(table: &str) -> Option<&'static str> {
    READY_SCHEMA_COLUMNS
        .iter()
        .map(|(name, _)| *name)
        .find(|name| *name == table && !is_disk_only_table(name))
}

fn max_table_id(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        &format!("SELECT COALESCE(MAX(id), 0) FROM {schema}.{table}"),
        [],
        |row| row.get::<_, i64>(0),
    )
}

fn memory_table_sql(table: &str, sql: &str) -> Option<String> {
    let create = format!("CREATE TABLE {table}");
    let create_if_not_exists = format!("CREATE TABLE IF NOT EXISTS {table}");
    if let Some(rest) = sql.strip_prefix(&create_if_not_exists) {
        return Some(format!("CREATE TABLE IF NOT EXISTS {MEMORY_SCHEMA}.{table}{rest}"));
    }
    sql.strip_prefix(&create)
        .map(|rest| format!("CREATE TABLE IF NOT EXISTS {MEMORY_SCHEMA}.{table}{rest}"))
}

/// SQLite mmap window for file-backed ledger databases.
///
/// Keep this in the DB layer: routes and security components should not know
/// whether a query reads through SQLite's page cache, mmap, or DB-owned memory
/// tables.
pub const SQLITE_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024;
pub const DB_SQLITE_MMAP_CONFIG_BYTES: &str = "db.sqlite_mmap_config_bytes";
pub const DB_SQLITE_MMAP_EFFECTIVE_BYTES: &str = "db.sqlite_mmap_effective_bytes";
pub const DB_SQLITE_FILE_SIZE_BYTES: &str = "db.sqlite_file_size_bytes";
pub const DB_SQLITE_WAL_SIZE_BYTES: &str = "db.sqlite_wal_size_bytes";
pub const DB_SQLITE_MMAP_COVERAGE_RATIO: &str = "db.sqlite_mmap_coverage_ratio";
pub const DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL: &str = "db.sqlite_mmap_budget_checks_total";

fn apply_mmap_pragma(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "mmap_size", SQLITE_MMAP_SIZE_BYTES)
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{}", path.display(), suffix))
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

pub fn record_sqlite_mmap_telemetry(conn: &Connection, path: &Path, role: &'static str, phase: &'static str) {
    let effective_mmap: i64 = conn.query_row("PRAGMA mmap_size", [], |row| row.get(0)).unwrap_or(0);
    let db_file_size = file_len(path);
    let wal_file_size = file_len(&sqlite_sidecar_path(path, "-wal"));
    let status = if db_file_size == 0 {
        "empty"
    } else if db_file_size <= effective_mmap.max(0) as u64 {
        "within_window"
    } else {
        "over_window"
    };
    let coverage_ratio = if db_file_size == 0 {
        1.0
    } else {
        (effective_mmap.max(0) as u64).min(db_file_size) as f64 / db_file_size as f64
    };

    ::metrics::gauge!(DB_SQLITE_MMAP_CONFIG_BYTES, "role" => role, "phase" => phase).set(SQLITE_MMAP_SIZE_BYTES as f64);
    ::metrics::gauge!(DB_SQLITE_MMAP_EFFECTIVE_BYTES, "role" => role, "phase" => phase).set(effective_mmap as f64);
    ::metrics::gauge!(DB_SQLITE_FILE_SIZE_BYTES, "role" => role, "phase" => phase).set(db_file_size as f64);
    ::metrics::gauge!(DB_SQLITE_WAL_SIZE_BYTES, "role" => role, "phase" => phase).set(wal_file_size as f64);
    ::metrics::gauge!(DB_SQLITE_MMAP_COVERAGE_RATIO, "role" => role, "phase" => phase).set(coverage_ratio);
    ::metrics::counter!(
        DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL,
        "role" => role,
        "phase" => phase,
        "status" => status
    )
    .increment(1);

    tracing::debug!(
        target: "capsem.db",
        db_path = %path.display(),
        role,
        phase,
        mmap_config_bytes = SQLITE_MMAP_SIZE_BYTES,
        mmap_effective_bytes = effective_mmap,
        db_file_size_bytes = db_file_size,
        wal_file_size_bytes = wal_file_size,
        mmap_coverage_ratio = coverage_ratio,
        mmap_budget_status = status,
        "sqlite mmap telemetry recorded"
    );
}

/// Apply write-mode pragmas: WAL journal + relaxed synchronous.
/// Only call on read-write connections (the writer).
pub fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    apply_mmap_pragma(conn)?;
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
    ("profile_mutation_events", &["mutation_id", "profile_id", "status"]),
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
    let pragma = format!("PRAGMA {schema}.table_info({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?;
    if columns.is_empty() {
        return Err(format!("session db missing required table {schema}.{table}"));
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
    let rows = match stmt.query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))) {
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
            transport TEXT NOT NULL DEFAULT 'unknown' CHECK (transport IN ('http', 'sse', 'websocket', 'vsock_frame', 'direct', 'unknown')),
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
            tool_name, arguments, response_preview, origin, transport, server_name, method,
            request_id, decision, duration_ms, error_message, process_name, bytes_sent,
            bytes_received, policy_mode, policy_action, policy_rule, policy_reason,
            trace_id, turn_id, credential_ref
        )
        SELECT
            id, event_id, timestamp, model_call_id, provider, status, call_index, call_id,
            tool_name, arguments, response_preview, origin,
            CASE
                WHEN origin = 'mcp' THEN 'vsock_frame'
                WHEN origin = 'local' THEN 'direct'
                ELSE 'unknown'
            END,
            server_name, method,
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
            &format!("CREATE INDEX IF NOT EXISTS idx_{tbl}_credential_ref ON {tbl}(credential_ref)"),
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
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN response_preview TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN transport TEXT NOT NULL DEFAULT 'unknown' CHECK (transport IN ('http', 'sse', 'websocket', 'vsock_frame', 'direct', 'unknown'))",
        [],
    );
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN server_name TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN method TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN request_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN decision TEXT NOT NULL DEFAULT 'allowed'",
        [],
    );
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN duration_ms INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN error_message TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN process_name TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN bytes_sent INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN bytes_received INTEGER DEFAULT 0", []);
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
            &format!("CREATE INDEX IF NOT EXISTS idx_{tbl}_credential_ref ON {tbl}(credential_ref)"),
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
            trace_id TEXT,
            turn_id TEXT
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
    apply_mmap_pragma(conn)?;
    conn.pragma_update(None, "query_only", "ON")?;
    Ok(())
}

#[cfg(test)]
mod tests;
