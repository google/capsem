//! The canonical ledger DDL.
//!
//! Every table and index a fresh session database is created with, as one
//! executable statement batch. It lives apart from `schema.rs` because that
//! module is behavior -- creation, the writer's memory schema -- and this is the
//! contract those behaviors operate on.
//!
//! It is the only place a session table is defined. `schema::migrate` used to
//! be a second one: seventy `let _ = conn.execute("ALTER TABLE ... ADD COLUMN
//! ...")` statements whose results were discarded, which on an older file
//! produced a third shape belonging to neither build. `session.db` is created
//! per session and never carried across builds, so a file an older build wrote
//! fails here, naming what it lacks.

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
        -- 1 when either header blob above was cut at HEADER_BYTES. A header
        -- set that stops mid-line must not read as one that ended there.
        headers_truncated INTEGER NOT NULL DEFAULT 0 CHECK (headers_truncated IN (0, 1)),
        request_body_preview TEXT, -- display excerpt; the full body is in event_body_blobs
        response_body_preview TEXT, -- display excerpt; the full body is in event_body_blobs
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
        system_prompt_preview TEXT, -- display excerpt; the full body is in event_body_blobs
        messages_count INTEGER DEFAULT 0,
        tools_count INTEGER DEFAULT 0,
        request_bytes INTEGER DEFAULT 0,
        request_body_preview TEXT, -- display excerpt; the full body is in event_body_blobs
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

    -- The one generation selected by this SQLite snapshot. Its UUID fields
    -- are raw canonical UUID bytes, and every other archive table belongs to
    -- this row. Absence is an incomplete or incompatible ledger.
    CREATE TABLE IF NOT EXISTS archive_state (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        archive_id BLOB NOT NULL CHECK (typeof(archive_id) = 'blob' AND length(archive_id) = 16),
        generation_id BLOB NOT NULL CHECK (typeof(generation_id) = 'blob' AND length(generation_id) = 16),
        format_version INTEGER NOT NULL CHECK (typeof(format_version) = 'integer' AND format_version = 4),
        committed_end INTEGER NOT NULL CHECK (typeof(committed_end) = 'integer' AND committed_end >= 80),
        revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision >= 1)
    );

    -- The session's counters: one named-field MessagePack snapshot, rewritten
    -- in the same transaction as the rows it counts, so a reader takes every
    -- total with one primary-key lookup instead of aggregating the tables.
    -- Created with the ledger; absence is a broken ledger, not zero activity.
    CREATE TABLE IF NOT EXISTS ledger_counters (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        counters BLOB NOT NULL CHECK (typeof(counters) = 'blob')
    );

    -- One block of an archive generation. The bytes live in the archive file;
    -- SQLite records where each block landed so a reader never scans. A block
    -- stays open across disk flushes and grows by one segment per flush, so
    -- the row is upserted as it grows: `raw_len` and `disk_len` are what the
    -- committed segments hold (disk_len counting the block and segment
    -- headers), and `sealed_at` is when the last of them was written.
    CREATE TABLE IF NOT EXISTS body_blocks (
        block_offset INTEGER PRIMARY KEY,
        raw_len INTEGER NOT NULL CHECK (raw_len > 0),
        disk_len INTEGER NOT NULL CHECK (disk_len > 0),
        sealed_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_body_blocks_sealed_at ON body_blocks(sealed_at);

    -- The index into `session.bodies`: one row per archived body, naming the
    -- block it sits in and its span inside that block's inflated bytes.
    CREATE TABLE IF NOT EXISTS event_body_blobs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.decision', 'security.ask')),
        source_table TEXT NOT NULL CHECK (source_table IN ('net_events', 'model_calls', 'tool_calls', 'tool_responses', 'exec_events', 'security_rule_events', 'security_decision_events', 'security_ask_events')),
        direction TEXT NOT NULL CHECK (direction IN ('request', 'response', 'payload', 'stdout', 'stderr')),
        content_type TEXT,
        original_bytes INTEGER NOT NULL CHECK (original_bytes >= 0),
        stored_bytes INTEGER NOT NULL CHECK (stored_bytes >= 0 AND stored_bytes <= original_bytes),
        truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)),
        -- blake3 of the ARCHIVED bytes, not of whatever they were cut from:
        -- a read verifies what it got against this, so a corrupted or
        -- edited index row is caught instead of served as a body.
        body_hash TEXT NOT NULL CHECK (length(body_hash) = 71 AND body_hash GLOB 'blake3:[0-9a-f]*'),
        block_offset INTEGER NOT NULL REFERENCES body_blocks(block_offset),
        body_offset INTEGER NOT NULL CHECK (body_offset >= 0),
        body_len INTEGER NOT NULL CHECK (body_len = stored_bytes),
        trace_id TEXT,
        turn_id TEXT,
        created_at TEXT NOT NULL,
        UNIQUE(event_id, source_table, direction)
    );
    CREATE INDEX IF NOT EXISTS idx_event_body_blobs_hash
        ON event_body_blobs(body_hash);
    CREATE INDEX IF NOT EXISTS idx_event_body_blobs_archive_order
        ON event_body_blobs(block_offset, body_offset, id);

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
        arguments TEXT, -- native tool-call arguments; for origin='mcp' this is the
                        -- MCP request display excerpt, and the full body is in event_body_blobs
        response_preview TEXT, -- display excerpt; the full body is in event_body_blobs
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
        event_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(6)))) CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
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
    CREATE INDEX IF NOT EXISTS idx_tool_calls_event_id
        ON tool_calls(event_id, id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_model_call
        ON tool_responses(model_call_id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_event_id
        ON tool_responses(event_id, id);
    CREATE INDEX IF NOT EXISTS idx_model_calls_trace_id
        ON model_calls(trace_id);
    -- The timeline reads each layer's window from a cutoff in time order.
    CREATE INDEX IF NOT EXISTS idx_model_calls_timestamp
        ON model_calls(timestamp);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_timestamp
        ON tool_calls(timestamp);

    -- The diagnostics triage filters tool errors by counted origin.
    CREATE INDEX IF NOT EXISTS idx_tool_calls_origin
        ON tool_calls(origin);

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
        kind TEXT NOT NULL DEFAULT 'file' CHECK (kind IN ('file','dir','symlink','other')),
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

    -- One canonical rule snapshot for repeated matches. Occurrence rows retain
    -- their own event ID and timestamp so decisions and source events still
    -- correlate exactly. The counter and bounds commit with those rows.
    CREATE TABLE IF NOT EXISTS security_rule_runs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_type TEXT NOT NULL,
        rule_id TEXT NOT NULL,
        rule_action TEXT NOT NULL,
        detection_level TEXT NOT NULL,
        rule_json TEXT NOT NULL CHECK (json_valid(rule_json)),
        count INTEGER NOT NULL CHECK (typeof(count) = 'integer' AND count > 0),
        first_timestamp_unix_ms INTEGER NOT NULL,
        last_timestamp_unix_ms INTEGER NOT NULL,
        CHECK (last_timestamp_unix_ms >= first_timestamp_unix_ms),
        UNIQUE (event_type, rule_id, rule_action, detection_level, rule_json)
    );

    CREATE TABLE IF NOT EXISTS security_rule_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp_unix_ms INTEGER NOT NULL,
        event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask', 'network.connect', 'network.connect_result', 'network.close', 'network.lifecycle', 'network.probe', 'network.probe_result')),
        rule_id TEXT NOT NULL,
        rule_action TEXT NOT NULL CHECK (rule_action IN ('allow', 'ask', 'block', 'preprocess', 'rewrite', 'postprocess')),
        detection_level TEXT NOT NULL DEFAULT 'none' CHECK (detection_level IN ('none', 'informational', 'low', 'medium', 'high', 'critical')),
        rule_json TEXT,
        run_id INTEGER REFERENCES security_rule_runs(id),
        -- The matched event's payload is NOT here: it is a body like any
        -- other and lives in `session.bodies`, indexed by `event_body_blobs`
        -- with direction 'payload'. It averaged a kilobyte and peaked at
        -- 297 KB in one real session, and every row of this table is mirrored
        -- in RAM.
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*')),
        CHECK ((rule_json IS NOT NULL AND json_valid(rule_json) AND run_id IS NULL)
            OR (rule_json IS NULL AND run_id IS NOT NULL))
    );
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_timestamp
        ON security_rule_events(timestamp_unix_ms);
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_event_id
        ON security_rule_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_rule_id
        ON security_rule_events(rule_id);
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_event_type
        ON security_rule_events(event_type);

    CREATE TABLE IF NOT EXISTS security_decision_runs (
        id INTEGER PRIMARY KEY,
        event_type TEXT NOT NULL,
        stage TEXT NOT NULL,
        actor TEXT NOT NULL,
        rule_id TEXT,
        plugin_id TEXT,
        previous_decision TEXT NOT NULL,
        requested_decision TEXT NOT NULL,
        effective_decision TEXT NOT NULL,
        reason TEXT,
        count INTEGER NOT NULL CHECK (typeof(count) = 'integer' AND count > 0),
        first_timestamp_unix_ms INTEGER NOT NULL,
        last_timestamp_unix_ms INTEGER NOT NULL,
        CHECK (last_timestamp_unix_ms >= first_timestamp_unix_ms)
    );
    -- Nullable shape fields need an unambiguous key: N for absent, S plus
    -- the original UTF-8 bytes for present. The index is tiny (one entry per
    -- distinct decision), while no JSON null placeholders enter storage.
    CREATE UNIQUE INDEX IF NOT EXISTS idx_security_decision_runs_shape
        ON security_decision_runs(
            event_type, stage, actor,
            CASE WHEN rule_id IS NULL THEN 'N' ELSE 'S' || hex(CAST(rule_id AS BLOB)) END,
            CASE WHEN plugin_id IS NULL THEN 'N' ELSE 'S' || hex(CAST(plugin_id AS BLOB)) END,
            previous_decision, requested_decision, effective_decision,
            CASE WHEN reason IS NULL THEN 'N' ELSE 'S' || hex(CAST(reason AS BLOB)) END
        );

    CREATE TABLE IF NOT EXISTS security_decision_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp_unix_ms INTEGER NOT NULL,
        event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask', 'network.connect', 'network.connect_result', 'network.close', 'network.lifecycle', 'network.probe', 'network.probe_result')),
        stage TEXT CHECK (stage IN ('preprocess', 'rule', 'rewrite', 'postprocess', 'ask_resolution')),
        actor TEXT,
        rule_id TEXT,
        plugin_id TEXT,
        previous_decision TEXT CHECK (previous_decision IN ('allow', 'ask', 'block')),
        requested_decision TEXT CHECK (requested_decision IN ('allow', 'ask', 'block')),
        effective_decision TEXT CHECK (effective_decision IN ('allow', 'ask', 'block')),
        reason TEXT,
        -- The event this decision was made about is archive-backed, like a
        -- rule match's: `event_body_blobs` with direction 'payload'. Roughly
        -- 25 decisions a request, each carrying the same event a rule match
        -- does, made this the largest table in a session and all of it
        -- mirrored in RAM.
        trace_id TEXT,
        turn_id TEXT,
        credential_ref TEXT CHECK (credential_ref IS NULL OR (length(credential_ref) = 82 AND credential_ref GLOB 'credential:blake3:[0-9a-f]*')),
        run_id INTEGER REFERENCES security_decision_runs(id),
        CHECK ((run_id IS NULL AND stage IS NOT NULL AND actor IS NOT NULL
                AND previous_decision IS NOT NULL AND requested_decision IS NOT NULL AND effective_decision IS NOT NULL)
            OR (run_id IS NOT NULL AND stage IS NULL AND actor IS NULL AND rule_id IS NULL AND plugin_id IS NULL
                AND previous_decision IS NULL AND requested_decision IS NULL AND effective_decision IS NULL AND reason IS NULL))
    );
    CREATE INDEX IF NOT EXISTS idx_security_decision_events_timestamp
        ON security_decision_events(timestamp_unix_ms);
    CREATE INDEX IF NOT EXISTS idx_security_decision_events_event_id
        ON security_decision_events(event_id);

    CREATE TABLE IF NOT EXISTS security_ask_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp_unix_ms INTEGER NOT NULL,
        ask_id TEXT NOT NULL CHECK (length(ask_id) = 12 AND ask_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_id TEXT NOT NULL CHECK (length(event_id) = 12 AND event_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
        event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask', 'network.connect', 'network.connect_result', 'network.close', 'network.lifecycle', 'network.probe', 'network.probe_result')),
        rule_id TEXT NOT NULL,
        rule_name TEXT NOT NULL,
        status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'denied')),
        rule_json TEXT NOT NULL CHECK (json_valid(rule_json)),
        -- The asked-about event is archive-backed, one way for every security
        -- payload: `event_body_blobs` with direction 'payload'.
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


    -- Correlation indexes: one per table that carries the column.
    CREATE INDEX IF NOT EXISTS idx_net_events_turn_id ON net_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_model_calls_turn_id ON model_calls(turn_id);
    CREATE INDEX IF NOT EXISTS idx_model_items_turn_id ON model_items(turn_id);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_turn_id ON tool_calls(turn_id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_turn_id ON tool_responses(turn_id);
    CREATE INDEX IF NOT EXISTS idx_fs_events_turn_id ON fs_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_exec_events_turn_id ON exec_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_dns_events_turn_id ON dns_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_audit_events_turn_id ON audit_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_substitution_events_turn_id ON substitution_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_turn_id ON security_rule_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_security_decision_events_turn_id ON security_decision_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_security_ask_events_turn_id ON security_ask_events(turn_id);
    CREATE INDEX IF NOT EXISTS idx_security_rule_events_credential_ref ON security_rule_events(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_security_decision_events_credential_ref ON security_decision_events(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_net_events_credential_ref ON net_events(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_model_calls_credential_ref ON model_calls(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_fs_events_credential_ref ON fs_events(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_exec_events_credential_ref ON exec_events(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_credential_ref ON tool_responses(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_dns_events_credential_ref ON dns_events(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_audit_events_credential_ref ON audit_events(credential_ref);
    CREATE INDEX IF NOT EXISTS idx_net_events_trace_id ON net_events(trace_id);
    CREATE INDEX IF NOT EXISTS idx_fs_events_trace_id ON fs_events(trace_id);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_trace_id ON tool_calls(trace_id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_trace_id ON tool_responses(trace_id);
    CREATE INDEX IF NOT EXISTS idx_audit_events_trace_id ON audit_events(trace_id);
    CREATE INDEX IF NOT EXISTS idx_net_events_event_id ON net_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_model_calls_event_id ON model_calls(event_id);
    CREATE INDEX IF NOT EXISTS idx_fs_events_event_id ON fs_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_exec_events_event_id ON exec_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_dns_events_event_id ON dns_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_audit_events_event_id ON audit_events(event_id);
    CREATE INDEX IF NOT EXISTS idx_substitution_events_event_id ON substitution_events(event_id);
";

/// The transport ledger, created once and thereafter only asserted.
///
/// It is apart from `CREATE_SCHEMA` because `CREATE TABLE IF NOT EXISTS`
/// silently recreates a table that is gone, and for this ledger that would be
/// the wrong answer: `transport_events` is the record of which connections
/// were allowed and which were blocked, so a file that has lost it must read
/// as corrupt, not as a session that never touched the network.
/// `create_tables` applies this batch only when the marker is absent, which
/// is to say only on a ledger that has never had it, and `assert_current`
/// speaks for every open after that.
pub(super) const CREATE_TRANSPORT: &str = "
    CREATE TABLE IF NOT EXISTS transport_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL UNIQUE CHECK(length(event_id)=12 AND event_id NOT GLOB '*[^0-9a-f]*'),
        timestamp_unix_ms INTEGER NOT NULL CHECK(timestamp_unix_ms >= 0),
        event_type TEXT NOT NULL CHECK(event_type IN ('network.connect','network.connect_result','network.close','network.lifecycle','network.probe','network.probe_result')),
        network_id TEXT,
        connection_id TEXT,
        event_json TEXT NOT NULL CHECK(length(CAST(event_json AS BLOB)) <= 65536 AND json_valid(event_json))
    );
    CREATE INDEX IF NOT EXISTS idx_transport_events_network ON transport_events(network_id,id);
    CREATE INDEX IF NOT EXISTS idx_transport_events_connection ON transport_events(connection_id,id);
    CREATE INDEX IF NOT EXISTS idx_transport_events_timestamp ON transport_events(timestamp_unix_ms,id);
    -- The transport ledger's own version marker. `user_version` belongs to
    -- SessionIndex in the shared main.db, so this one is logger-owned and
    -- disk-only; `transport::assert_current` refuses a version it does not
    -- know rather than upgrading the file. The gate that keeps a deleted
    -- marker from coming back is the TABLE's absence, checked in
    -- `create_tables`, not the row's: `OR IGNORE` here is for two writers
    -- opening the same fresh ledger at once, where the loser must no-op
    -- rather than fail on the primary key.
    CREATE TABLE IF NOT EXISTS transport_schema (
        id INTEGER PRIMARY KEY CHECK(id=1),
        version INTEGER NOT NULL
    );
    INSERT OR IGNORE INTO transport_schema(id,version) VALUES(1,1);
";
