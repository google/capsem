//! Tests for `writer` (extracted from inline `mod tests`).

use super::*;
use std::collections::BTreeMap;

use capsem_security_engine::{
    BlockResponse, DetectionFinding, HttpBodySecuritySubject, HttpSecuritySubject,
    ResolvedEventStep, SecurityEvent, SecurityEventCommon, TraceHistoryEntry,
    RESOLVED_EVENT_SCHEMA_VERSION,
};
use serde::Serialize;

fn assert_sql_enum<T>(value: T)
where
    T: SqlEnumText + Serialize + Copy,
{
    let serialized = serde_json::to_value(value)
        .unwrap()
        .as_str()
        .expect("canonical enum serialization must be a string")
        .to_string();
    assert_eq!(value.sql_text(), serialized);
}

#[test]
fn ai_evidence_sql_enum_text_matches_canonical_serde_names() {
    for value in [
        AiProvider::Openai,
        AiProvider::Anthropic,
        AiProvider::GoogleGemini,
        AiProvider::Unknown,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        AiApiFamily::OpenaiChatCompletions,
        AiApiFamily::OpenaiResponses,
        AiApiFamily::AnthropicMessages,
        AiApiFamily::GoogleGeminiContent,
        AiApiFamily::Mcp,
        AiApiFamily::Unknown,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        AiAttributionScope::Host,
        AiAttributionScope::Vm,
        AiAttributionScope::Profile,
        AiAttributionScope::Session,
        AiAttributionScope::Unknown,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        AiOriginKind::GuestNetwork,
        AiOriginKind::HostService,
        AiOriginKind::HostAdmin,
        AiOriginKind::HostWorkbench,
        AiOriginKind::TestFixture,
        AiOriginKind::Unknown,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        ArgumentsStatus::ValidJson,
        ArgumentsStatus::PartialJson,
        ArgumentsStatus::MalformedJson,
        ArgumentsStatus::NotJson,
        ArgumentsStatus::Redacted,
        ArgumentsStatus::Absent,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        ParseStatus::Complete,
        ParseStatus::Partial,
        ParseStatus::Malformed,
        ParseStatus::Unsupported,
        ParseStatus::Redacted,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        EvidenceStatus::Complete,
        EvidenceStatus::Partial,
        EvidenceStatus::Ambiguous,
        EvidenceStatus::Orphaned,
        EvidenceStatus::Untrusted,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        ToolOrigin::NativeProviderTool,
        ToolOrigin::McpTool,
        ToolOrigin::LocalBuiltinTool,
        ToolOrigin::Unknown,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        LinkStatus::Linked,
        LinkStatus::UnlinkedPending,
        LinkStatus::OrphanModelToolCall,
        LinkStatus::OrphanMcpExecution,
        LinkStatus::Ambiguous,
        LinkStatus::NotApplicable,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        ToolCallStatus::Proposed,
        ToolCallStatus::Executed,
        ToolCallStatus::Blocked,
        ToolCallStatus::ReturnedToModel,
        ToolCallStatus::Error,
        ToolCallStatus::Unknown,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        AiContentKind::Text,
        AiContentKind::Json,
        AiContentKind::Image,
        AiContentKind::File,
        AiContentKind::ToolUse,
        AiContentKind::ToolResult,
        AiContentKind::Reasoning,
        AiContentKind::CacheMarker,
        AiContentKind::Redacted,
        AiContentKind::Unknown,
    ] {
        assert_sql_enum(value);
    }
    for value in [Confidence::Low, Confidence::Medium, Confidence::High] {
        assert_sql_enum(value);
    }
    for value in [
        SourceEngine::Network,
        SourceEngine::File,
        SourceEngine::Process,
        SourceEngine::Conversation,
        SourceEngine::Security,
        SourceEngine::Vm,
        SourceEngine::Profile,
        SourceEngine::HostAi,
    ] {
        assert_sql_enum(value);
    }
}

#[test]
fn security_event_sql_enum_text_matches_canonical_serde_names() {
    for value in [
        EventFamily::Dns,
        EventFamily::Http,
        EventFamily::Mcp,
        EventFamily::Model,
        EventFamily::File,
        EventFamily::Process,
        EventFamily::Credential,
        EventFamily::Vm,
        EventFamily::Profile,
        EventFamily::Conversation,
        EventFamily::Snapshot,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        Enforceability::InlineBlockable,
        Enforceability::ObserveOnly,
        Enforceability::RemediationOnly,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        RedactionState::Raw,
        RedactionState::Redacted,
        RedactionState::SummaryOnly,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        ResolvedEventStepKind::Preprocessor,
        ResolvedEventStepKind::PluginCallback,
        ResolvedEventStepKind::EnforcementMatch,
        ResolvedEventStepKind::Confirm,
        ResolvedEventStepKind::RateLimitCheck,
        ResolvedEventStepKind::DetectionMatch,
        ResolvedEventStepKind::Postprocessor,
        ResolvedEventStepKind::EmitterDelivery,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        StepStatus::Applied,
        StepStatus::Matched,
        StepStatus::Skipped,
        StepStatus::Error,
    ] {
        assert_sql_enum(value);
    }
    for value in [
        Severity::Info,
        Severity::Low,
        Severity::Medium,
        Severity::High,
        Severity::Critical,
    ] {
        assert_sql_enum(value);
    }
}

fn security_common(event_id: &str) -> SecurityEventCommon {
    SecurityEventCommon {
        event_id: event_id.to_string(),
        parent_event_id: Some("evt-parent".to_string()),
        stream_id: Some("stream-1".to_string()),
        activity_id: Some("activity-1".to_string()),
        sequence_no: Some(7),
        source_engine: SourceEngine::Network,
        attribution_scope: AiAttributionScope::Vm,
        origin_kind: AiOriginKind::GuestNetwork,
        accounting_owner: Some("vm:vm-1".to_string()),
        enforceability: Enforceability::InlineBlockable,
        trace_id: Some("trace-1".to_string()),
        span_id: Some("span-1".to_string()),
        timestamp_unix_ms: 1_700_000_123_456,
        vm_id: Some("vm-1".to_string()),
        session_id: Some("session-1".to_string()),
        profile_id: Some("coding".to_string()),
        profile_revision: Some("rev-a".to_string()),
        profile_pack_ids: Vec::new(),
        enforcement_packs: Vec::new(),
        detection_packs: Vec::new(),
        user_id: Some("user-1".to_string()),
        process_id: Some("pid-42".to_string()),
        parent_process_id: Some("pid-1".to_string()),
        exec_id: Some("exec-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        message_id: Some("message-1".to_string()),
        tool_call_id: Some("tool-call-1".to_string()),
        mcp_call_id: Some("mcp-call-1".to_string()),
        event_type: "http.request".to_string(),
        redaction_state: RedactionState::Raw,
    }
}

#[test]
fn resolved_security_event_writes_structured_event_steps_findings_and_links() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-events.db");

    let mut headers = BTreeMap::new();
    headers.insert(
        "authorization".to_string(),
        vec!["Bearer secret-token".to_string()],
    );
    let mut event = SecurityEvent::http(
        security_common("evt-sec-1"),
        HttpSecuritySubject {
            method: "POST".to_string(),
            scheme: Some("https".to_string()),
            host: "api.example.com".to_string(),
            port: Some(443),
            path: Some("/admin".to_string()),
            query: None,
            url: Some("https://api.example.com/admin".to_string()),
            path_class: "admin".to_string(),
            request_bytes: 42,
            request_headers: headers,
            request_body: Some(HttpBodySecuritySubject::text("secret payload")),
            response_status: None,
            response_headers: BTreeMap::new(),
            response_bytes: None,
            response_body: None,
        },
    );
    event.labels.push("http".to_string());
    event.trace.history.push(TraceHistoryEntry {
        event_id: "evt-dns-1".to_string(),
        event_type: "dns.request".to_string(),
        labels: vec!["dns".to_string()],
    });
    event.context.history.push(TraceHistoryEntry {
        event_id: "evt-model-1".to_string(),
        event_type: "model.request".to_string(),
        labels: vec!["model".to_string()],
    });

    let finding = DetectionFinding {
        finding_id: "finding-1".to_string(),
        event_id: "evt-sec-1".to_string(),
        rule_id: "detect.admin_path".to_string(),
        pack_id: "pack-detect".to_string(),
        sigma_id: Some("sigma-admin".to_string()),
        title: "Admin path access".to_string(),
        severity: Severity::High,
        confidence: Confidence::High,
        tags: vec![
            "attack.initial_access".to_string(),
            "capsem.http".to_string(),
        ],
    };

    let resolved = ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event,
        steps: vec![
            ResolvedEventStep {
                kind: ResolvedEventStepKind::Preprocessor,
                status: StepStatus::Applied,
                rule_id: None,
                pack_id: Some("pack-runtime".to_string()),
                message: Some("credential redaction ran".to_string()),
            },
            ResolvedEventStep {
                kind: ResolvedEventStepKind::DetectionMatch,
                status: StepStatus::Matched,
                rule_id: Some("detect.admin_path".to_string()),
                pack_id: Some("pack-detect".to_string()),
                message: Some("sigma matched admin path".to_string()),
            },
            ResolvedEventStep {
                kind: ResolvedEventStepKind::EnforcementMatch,
                status: StepStatus::Matched,
                rule_id: Some("enforce.block_admin".to_string()),
                pack_id: Some("pack-runtime".to_string()),
                message: Some("blocked admin".to_string()),
            },
        ],
        plugin_transforms: Vec::new(),
        detection_findings: vec![finding],
        final_action: SecurityAction::Block(BlockResponse {
            reason_code: "blocked_admin".to_string(),
            rule_id: Some("enforce.block_admin".to_string()),
        }),
        emitter_results: Vec::new(),
    };

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer.write(WriteOp::ResolvedSecurityEvent(resolved)).await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let event_row: (String, String, String, String, String, String, i64, i64) = conn
        .query_row(
            "SELECT event_family, event_type, source_engine, final_action,
                    attribution_scope, profile_id, label_count, finding_count
             FROM security_events WHERE event_id = 'evt-sec-1'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        event_row,
        (
            "http".to_string(),
            "http.request".to_string(),
            "network".to_string(),
            "block".to_string(),
            "vm".to_string(),
            "coding".to_string(),
            1,
            1,
        )
    );

    let steps: Vec<(String, String, Option<String>)> = {
        let mut stmt = conn
            .prepare(
                "SELECT kind, status, rule_id FROM security_event_steps
                 WHERE event_id = 'evt-sec-1' ORDER BY step_index ASC",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        steps,
        vec![
            ("preprocessor".to_string(), "applied".to_string(), None),
            (
                "detection_match".to_string(),
                "matched".to_string(),
                Some("detect.admin_path".to_string()),
            ),
            (
                "enforcement_match".to_string(),
                "matched".to_string(),
                Some("enforce.block_admin".to_string()),
            ),
        ]
    );

    let finding_row: (String, String, String, String, String) = conn
        .query_row(
            "SELECT finding_id, rule_id, sigma_id, severity, confidence
             FROM detection_findings WHERE event_id = 'evt-sec-1'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        finding_row,
        (
            "finding-1".to_string(),
            "detect.admin_path".to_string(),
            "sigma-admin".to_string(),
            "high".to_string(),
            "high".to_string(),
        )
    );

    let tags: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT tag FROM detection_finding_tags
                 WHERE finding_id = 'finding-1' ORDER BY tag_index ASC",
            )
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(tags, vec!["attack.initial_access", "capsem.http"]);

    let links: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT linked_event_id, link_type FROM security_event_links
                 WHERE event_id = 'evt-sec-1' ORDER BY id ASC",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        links,
        vec![
            ("evt-parent".to_string(), "parent".to_string()),
            ("evt-dns-1".to_string(), "trace_history".to_string()),
            ("evt-model-1".to_string(), "context_history".to_string()),
        ]
    );
}

#[test]
fn writer_metrics_snapshot_counts_resolved_security_decisions_and_findings() {
    let writer = DbWriter::open_in_memory(64).unwrap();
    let mut common = security_common("evt-metrics-block");
    common.timestamp_unix_ms = 1_700_000_123_999;
    let event = SecurityEvent::http(
        common,
        HttpSecuritySubject {
            method: "GET".to_string(),
            scheme: Some("https".to_string()),
            host: "blocked.example".to_string(),
            port: Some(443),
            path: Some("/secret".to_string()),
            query: None,
            url: Some("https://blocked.example/secret".to_string()),
            path_class: "secret".to_string(),
            request_bytes: 0,
            request_headers: BTreeMap::new(),
            request_body: None,
            response_status: None,
            response_headers: BTreeMap::new(),
            response_bytes: None,
            response_body: None,
        },
    );
    let finding = DetectionFinding {
        finding_id: "finding-metrics".to_string(),
        event_id: "evt-metrics-block".to_string(),
        rule_id: "detect.secret".to_string(),
        pack_id: "pack-detect".to_string(),
        sigma_id: None,
        title: "Secret path".to_string(),
        severity: Severity::High,
        confidence: Confidence::High,
        tags: Vec::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        writer
            .write(WriteOp::ResolvedSecurityEvent(ResolvedSecurityEvent {
                schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
                event,
                steps: Vec::new(),
                plugin_transforms: Vec::new(),
                detection_findings: vec![finding],
                final_action: SecurityAction::Block(BlockResponse {
                    reason_code: "secret blocked".to_string(),
                    rule_id: Some("enforce.secret".to_string()),
                }),
                emitter_results: Vec::new(),
            }))
            .await;
    });

    let snapshot = writer.metrics_snapshot("vm-1", true, 1_700_000_124_000);

    assert_eq!(snapshot.vm_id, "vm-1");
    assert!(snapshot.persistent);
    assert_eq!(snapshot.security.security_events_total, 1);
    assert_eq!(snapshot.security.blocks_total, 1);
    assert_eq!(snapshot.security.detection_findings_total, 1);
    assert_eq!(
        snapshot.security.latest_block_event_id.as_deref(),
        Some("evt-metrics-block")
    );
    assert_eq!(
        snapshot.security.latest_block_rule_id.as_deref(),
        Some("enforce.secret")
    );
    assert_eq!(
        snapshot.security.latest_detection_rule_id.as_deref(),
        Some("detect.secret")
    );
}

#[test]
fn cap_field_none_returns_none() {
    assert!(cap_field(&None).is_none());
}

#[test]
fn cap_field_short_string_unchanged() {
    let s = Some("hello world".to_string());
    assert_eq!(cap_field(&s).as_deref(), Some("hello world"));
}

#[test]
fn cap_field_exact_limit_unchanged() {
    let s = Some("x".repeat(MAX_FIELD_BYTES));
    let result = cap_field(&s).unwrap();
    assert_eq!(result.len(), MAX_FIELD_BYTES);
}

#[test]
fn cap_field_over_limit_truncated() {
    let s = Some("a".repeat(MAX_FIELD_BYTES + 100));
    let result = cap_field(&s).unwrap();
    assert_eq!(result.len(), MAX_FIELD_BYTES);
}

#[test]
fn cap_field_utf8_boundary_safe() {
    // Multi-byte UTF-8: each char is 4 bytes
    let emoji = "\u{1F600}"; // 4-byte emoji
    assert_eq!(emoji.len(), 4);
    // Fill up to just past the limit with 4-byte chars
    let count = MAX_FIELD_BYTES / 4 + 1; // slightly over
    let s = Some(emoji.repeat(count));
    let result = cap_field(&s).unwrap();
    assert!(result.len() <= MAX_FIELD_BYTES);
    // Truncated at a char boundary -- must be valid UTF-8
    assert!(result.is_char_boundary(result.len()));
    // Length should be a multiple of 4 (each emoji is 4 bytes)
    assert_eq!(result.len() % 4, 0);
}

#[test]
fn cap_field_two_byte_utf8_boundary() {
    // 2-byte char: e.g. 'a' with accent
    let ch = "\u{00E9}"; // e-acute, 2 bytes
    assert_eq!(ch.len(), 2);
    let count = MAX_FIELD_BYTES / 2 + 1;
    let s = Some(ch.repeat(count));
    let result = cap_field(&s).unwrap();
    assert!(result.len() <= MAX_FIELD_BYTES);
    assert_eq!(result.len() % 2, 0);
}

#[test]
fn cap_field_three_byte_utf8_boundary() {
    // 3-byte char: CJK character
    let ch = "\u{4E16}"; // Chinese char, 3 bytes
    assert_eq!(ch.len(), 3);
    let count = MAX_FIELD_BYTES / 3 + 1;
    let s = Some(ch.repeat(count));
    let result = cap_field(&s).unwrap();
    assert!(result.len() <= MAX_FIELD_BYTES);
    assert_eq!(result.len() % 3, 0);
}

#[test]
fn cap_field_empty_string_unchanged() {
    let s = Some(String::new());
    assert_eq!(cap_field(&s).as_deref(), Some(""));
}

#[test]
fn cap_field_mixed_ascii_and_multibyte() {
    // Fill most of the buffer with ASCII, end with a 4-byte char that straddles the limit
    let mut s = "x".repeat(MAX_FIELD_BYTES - 1);
    s.push('\u{1F600}'); // 4 bytes, total = MAX_FIELD_BYTES + 3
    let result = cap_field(&Some(s)).unwrap();
    assert!(result.len() <= MAX_FIELD_BYTES);
    // Should have truncated to MAX_FIELD_BYTES - 1 (dropping the emoji)
    assert_eq!(result.len(), MAX_FIELD_BYTES - 1);
    assert!(result.chars().all(|c| c == 'x'));
}

#[test]
fn db_writer_checkpoints_wal_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // Write some events, then drop the writer.
    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::FileEvent(crate::events::FileEvent {
                    timestamp: std::time::SystemTime::now(),
                    action: crate::events::FileAction::Created,
                    path: "/tmp/test".to_string(),
                    size: Some(42),
                    trace_id: None,
                }))
                .await;
        });
        // DbWriter::drop runs here -- should checkpoint WAL.
    }

    // After drop, WAL should be truncated (empty or zero-length).
    let wal_path = dir.path().join("test.db-wal");
    if wal_path.exists() {
        let wal_size = std::fs::metadata(&wal_path).unwrap().len();
        assert_eq!(wal_size, 0, "WAL should be empty after checkpoint");
    }

    // Verify data is in the main DB file (not just WAL).
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM fs_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn telemetry_identity_roundtrip_updates_single_session_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("identity.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::TelemetryIdentity(
                    crate::events::TelemetryIdentity {
                        timestamp: std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(1_779_000_000),
                        vm_id: "vm-a".to_string(),
                        profile_id: "everyday-work".to_string(),
                        user_id: "elie".to_string(),
                    },
                ))
                .await;
            writer
                .write(WriteOp::TelemetryIdentity(
                    crate::events::TelemetryIdentity {
                        timestamp: std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(1_779_000_001),
                        vm_id: "vm-a".to_string(),
                        profile_id: "locked-down".to_string(),
                        user_id: "elie".to_string(),
                    },
                ))
                .await;
        });
    }

    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    let identity = reader
        .session_identity()
        .unwrap()
        .expect("identity row must exist");
    assert_eq!(identity.vm_id, "vm-a");
    assert_eq!(identity.profile_id, "locked-down");
    assert_eq!(identity.user_id, "elie");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM session_identity", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1, "identity must update in place, not append");
}

#[test]
fn snapshot_event_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("snap.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                    timestamp: std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(1_700_000_000),
                    slot: 3,
                    origin: "auto".to_string(),
                    name: None,
                    files_count: 42,
                    start_fs_event_id: 10,
                    stop_fs_event_id: 25,
                    trace_id: None,
                }))
                .await;
            writer
                .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                    timestamp: std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(1_700_000_100),
                    slot: 10,
                    origin: "manual".to_string(),
                    name: Some("checkpoint_1".to_string()),
                    files_count: 55,
                    start_fs_event_id: 25,
                    stop_fs_event_id: 40,
                    trace_id: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM snapshot_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);

    let (slot, origin, name, files, start_id, stop_id): (
        i64,
        String,
        Option<String>,
        i64,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT slot, origin, name, files_count, start_fs_event_id, stop_fs_event_id
             FROM snapshot_events ORDER BY id ASC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(slot, 3);
    assert_eq!(origin, "auto");
    assert!(name.is_none());
    assert_eq!(files, 42);
    assert_eq!(start_id, 10);
    assert_eq!(stop_id, 25);

    let (slot2, origin2, name2): (i64, String, Option<String>) = conn
        .query_row(
            "SELECT slot, origin, name FROM snapshot_events ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(slot2, 10);
    assert_eq!(origin2, "manual");
    assert_eq!(name2.as_deref(), Some("checkpoint_1"));
}

#[test]
fn snapshot_fs_events_cross_reference() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cross.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            // Write some fs_events first.
            for i in 0..5 {
                writer
                    .write(WriteOp::FileEvent(crate::events::FileEvent {
                        timestamp: std::time::SystemTime::now(),
                        action: crate::events::FileAction::Created,
                        path: format!("file_{i}.txt"),
                        size: Some(100),
                        trace_id: None,
                    }))
                    .await;
            }
            for i in 5..8 {
                writer
                    .write(WriteOp::FileEvent(crate::events::FileEvent {
                        timestamp: std::time::SystemTime::now(),
                        action: crate::events::FileAction::Modified,
                        path: format!("file_{i}.txt"),
                        size: Some(200),
                        trace_id: None,
                    }))
                    .await;
            }
            writer
                .write(WriteOp::FileEvent(crate::events::FileEvent {
                    timestamp: std::time::SystemTime::now(),
                    action: crate::events::FileAction::Deleted,
                    path: "old.txt".to_string(),
                    size: None,
                    trace_id: None,
                }))
                .await;

            // Snapshot 1: covers fs_events 1..5 (5 created)
            writer
                .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                    timestamp: std::time::SystemTime::now(),
                    slot: 0,
                    origin: "auto".to_string(),
                    name: None,
                    files_count: 5,
                    start_fs_event_id: 0,
                    stop_fs_event_id: 5,
                    trace_id: None,
                }))
                .await;

            // Snapshot 2: covers fs_events 6..9 (3 modified + 1 deleted)
            writer
                .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                    timestamp: std::time::SystemTime::now(),
                    slot: 1,
                    origin: "auto".to_string(),
                    name: None,
                    files_count: 8,
                    start_fs_event_id: 5,
                    stop_fs_event_id: 9,
                    trace_id: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Verify snapshot 1 sees 5 created files.
    let (created, modified, deleted): (i64, i64, i64) = conn
        .query_row(
            "SELECT
                SUM(CASE WHEN action='created' THEN 1 ELSE 0 END),
                SUM(CASE WHEN action='modified' THEN 1 ELSE 0 END),
                SUM(CASE WHEN action='deleted' THEN 1 ELSE 0 END)
             FROM fs_events WHERE id > 0 AND id <= 5",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(created, 5);
    assert_eq!(modified, 0);
    assert_eq!(deleted, 0);

    // Verify snapshot 2 sees 3 modified + 1 deleted.
    let (created2, modified2, deleted2): (i64, i64, i64) = conn
        .query_row(
            "SELECT
                SUM(CASE WHEN action='created' THEN 1 ELSE 0 END),
                SUM(CASE WHEN action='modified' THEN 1 ELSE 0 END),
                SUM(CASE WHEN action='deleted' THEN 1 ELSE 0 END)
             FROM fs_events WHERE id > 5 AND id <= 9",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(created2, 0);
    assert_eq!(modified2, 3);
    assert_eq!(deleted2, 1);
}

#[test]
fn snapshot_ring_buffer_dedup_query() {
    // Tests the SQL pattern used by the frontend: MAX(id) GROUP BY slot
    // ensures only the latest event per slot is returned when the ring
    // buffer overwrites a slot.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("ring.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            // Slot 0, first pass.
            writer
                .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                    timestamp: std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(1000),
                    slot: 0,
                    origin: "auto".to_string(),
                    name: None,
                    files_count: 5,
                    start_fs_event_id: 0,
                    stop_fs_event_id: 3,
                    trace_id: None,
                }))
                .await;
            // Slot 1.
            writer
                .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                    timestamp: std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(2000),
                    slot: 1,
                    origin: "auto".to_string(),
                    name: None,
                    files_count: 8,
                    start_fs_event_id: 3,
                    stop_fs_event_id: 7,
                    trace_id: None,
                }))
                .await;
            // Slot 0 again (ring buffer wrapped).
            writer
                .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                    timestamp: std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(3000),
                    slot: 0,
                    origin: "auto".to_string(),
                    name: None,
                    files_count: 12,
                    start_fs_event_id: 7,
                    stop_fs_event_id: 15,
                    trace_id: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Total rows = 3 (all insertions).
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM snapshot_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(total, 3);

    // Dedup query: latest per slot. Should return 2 rows (slot 0 latest + slot 1).
    let dedup: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM snapshot_events
             WHERE id IN (SELECT MAX(id) FROM snapshot_events GROUP BY slot)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dedup, 2);

    // Slot 0 should show files_count=12 (the newer entry), not 5.
    let files: i64 = conn
        .query_row(
            "SELECT files_count FROM snapshot_events
             WHERE id IN (SELECT MAX(id) FROM snapshot_events GROUP BY slot)
             AND slot = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(files, 12);
}

#[test]
fn shutdown_blocking_through_arc_flushes_wal() {
    // Verifies the explicit-cleanup contract: callers holding
    // Arc<DbWriter> can drain the writer thread synchronously through
    // &self, without waiting for the last Arc clone to drop. This is
    // the path taken by capsem-process's SIGTERM handler.
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("shutdown.db");
    let writer = Arc::new(DbWriter::open(&db_path, 64).unwrap());

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        writer
            .write(WriteOp::FileEvent(crate::events::FileEvent {
                timestamp: std::time::SystemTime::now(),
                action: crate::events::FileAction::Created,
                path: "/x".into(),
                size: Some(1),
                trace_id: None,
            }))
            .await;
    });

    // Additional Arc clone stays alive across shutdown; the explicit
    // shutdown must not require the clone to drop first.
    let _keep = Arc::clone(&writer);
    writer.shutdown_blocking();

    let wal_path = dir.path().join("shutdown.db-wal");
    if wal_path.exists() {
        assert_eq!(
            std::fs::metadata(&wal_path).unwrap().len(),
            0,
            "WAL must be checkpointed after shutdown_blocking"
        );
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM fs_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "durable write must survive shutdown_blocking");
}

#[test]
fn shutdown_blocking_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("idemp.db"), 16).unwrap();
    writer.shutdown_blocking();
    // Second call must not panic or double-join.
    writer.shutdown_blocking();
}

#[test]
fn write_after_shutdown_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("no.db"), 16).unwrap();
    writer.shutdown_blocking();
    assert!(
        !writer.try_write(WriteOp::FileEvent(crate::events::FileEvent {
            timestamp: std::time::SystemTime::now(),
            action: crate::events::FileAction::Created,
            path: "/after".into(),
            size: None,
            trace_id: None,
        }))
    );
}

#[test]
fn slow_checkpoint_hook_delays_shutdown() {
    // Sets CAPSEM_TEST_SLOW_CHECKPOINT_MS on the spawned writer thread
    // (env var is inherited by the thread). Asserts shutdown_blocking
    // waits for the delayed checkpoint rather than returning early --
    // which is precisely what an implicit runtime-drop path would fail
    // to guarantee under a tight SIGKILL budget.
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: std::env::set_var is unsafe on 2024 edition -- single
    // writer in this test, no concurrent readers.
    unsafe { std::env::set_var("CAPSEM_TEST_SLOW_CHECKPOINT_MS", "200") };
    let writer = DbWriter::open(&dir.path().join("slow.db"), 16).unwrap();
    let start = std::time::Instant::now();
    writer.shutdown_blocking();
    let elapsed = start.elapsed();
    unsafe { std::env::remove_var("CAPSEM_TEST_SLOW_CHECKPOINT_MS") };
    assert!(
        elapsed >= std::time::Duration::from_millis(150),
        "shutdown_blocking must wait for slow checkpoint (elapsed={elapsed:?})"
    );
    let wal_path = dir.path().join("slow.db-wal");
    if wal_path.exists() {
        assert_eq!(std::fs::metadata(&wal_path).unwrap().len(), 0);
    }
}

#[test]
fn try_write_on_open_writer_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("t.db"), 64).unwrap();
    let accepted = writer.try_write(WriteOp::FileEvent(crate::events::FileEvent {
        timestamp: std::time::SystemTime::now(),
        action: crate::events::FileAction::Created,
        path: "/x".into(),
        size: None,
        trace_id: None,
    }));
    assert!(accepted);
}

#[test]
fn reader_for_in_memory_writer_fails() {
    let writer = DbWriter::open_in_memory(16).unwrap();
    match writer.reader() {
        Err(rusqlite::Error::InvalidPath(_)) => {}
        Err(other) => panic!("expected InvalidPath, got {other:?}"),
        Ok(_) => panic!("expected reader() to fail for :memory:"),
    }
}

#[test]
fn path_accessor_returns_configured_path() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("mydb.db");
    let writer = DbWriter::open(&p, 16).unwrap();
    assert_eq!(writer.path(), p);
}

#[test]
fn exec_event_insert_then_update_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("exec.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::ExecEvent(crate::events::ExecEvent {
                    timestamp: std::time::SystemTime::now(),
                    exec_id: 42,
                    command: "ls -la".into(),
                    source: "mcp".into(),
                    mcp_call_id: Some(7),
                    trace_id: Some("t1".into()),
                    process_name: Some("capsem".into()),
                }))
                .await;

            writer
                .write(WriteOp::ExecEventComplete(
                    crate::events::ExecEventComplete {
                        exec_id: 42,
                        exit_code: 0,
                        duration_ms: 120,
                        stdout_preview: Some("out".into()),
                        stderr_preview: None,
                        stdout_bytes: 128,
                        stderr_bytes: 0,
                        pid: Some(1234),
                    },
                ))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (command, source, exit, duration, stdout_preview, stderr_preview, stdout_bytes, pid) = conn.query_row(
        "SELECT command, source, exit_code, duration_ms, stdout_preview, stderr_preview, stdout_bytes, pid
         FROM exec_events WHERE exec_id = 42",
        [],
        |r| {
            let command: String = r.get(0)?;
            let source: String = r.get(1)?;
            let exit: i64 = r.get(2)?;
            let duration: i64 = r.get(3)?;
            let stdout_preview: Option<String> = r.get(4)?;
            let stderr_preview: Option<String> = r.get(5)?;
            let stdout_bytes: i64 = r.get(6)?;
            let pid: Option<i64> = r.get(7)?;
            Ok((command, source, exit, duration, stdout_preview, stderr_preview, stdout_bytes, pid))
        },
    ).unwrap();
    assert_eq!(command, "ls -la");
    assert_eq!(source, "mcp");
    assert_eq!(exit, 0);
    assert_eq!(duration, 120);
    assert_eq!(stdout_preview.as_deref(), Some("out"));
    assert!(stderr_preview.is_none());
    assert_eq!(stdout_bytes, 128);
    assert_eq!(pid, Some(1234));
}

#[test]
fn mcp_call_insert_populates_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("mcp.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::McpCall(crate::events::McpCall {
                    timestamp: std::time::SystemTime::now(),
                    server_name: "github".into(),
                    method: "tools/call".into(),
                    tool_name: Some("list_issues".into()),
                    request_id: Some("r1".into()),
                    request_preview: Some("{}".into()),
                    response_preview: None,
                    decision: "allowed".into(),
                    duration_ms: 50,
                    error_message: None,
                    process_name: Some("agent".into()),
                    bytes_sent: 64,
                    bytes_received: 128,
                    policy_mode: Some("audit_only".into()),
                    policy_action: Some("allow".into()),
                    policy_rule: Some("mcp.tool.github__list_issues".into()),
                    policy_reason: Some("local policy allow".into()),
                    trace_id: None,
                }))
                .await;
        });
    }

    struct McpCallRow {
        server: String,
        method: String,
        tool: Option<String>,
        decision: String,
        sent: i64,
        recv: i64,
        mode: Option<String>,
        action: Option<String>,
        rule: Option<String>,
        reason: Option<String>,
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row = conn
        .query_row(
            "SELECT server_name, method, tool_name, decision, bytes_sent, bytes_received,
                policy_mode, policy_action, policy_rule, policy_reason
         FROM mcp_calls",
            [],
            |r| {
                Ok(McpCallRow {
                    server: r.get(0)?,
                    method: r.get(1)?,
                    tool: r.get(2)?,
                    decision: r.get(3)?,
                    sent: r.get(4)?,
                    recv: r.get(5)?,
                    mode: r.get(6)?,
                    action: r.get(7)?,
                    rule: r.get(8)?,
                    reason: r.get(9)?,
                })
            },
        )
        .unwrap();
    assert_eq!(row.server, "github");
    assert_eq!(row.method, "tools/call");
    assert_eq!(row.tool.as_deref(), Some("list_issues"));
    assert_eq!(row.decision, "allowed");
    assert_eq!(row.sent, 64);
    assert_eq!(row.recv, 128);
    assert_eq!(row.mode.as_deref(), Some("audit_only"));
    assert_eq!(row.action.as_deref(), Some("allow"));
    assert_eq!(row.rule.as_deref(), Some("mcp.tool.github__list_issues"));
    assert_eq!(row.reason.as_deref(), Some("local policy allow"));
}

#[test]
fn audit_event_insert_populates_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("audit.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::AuditEvent(crate::events::AuditEvent {
                    timestamp: std::time::SystemTime::now(),
                    pid: 100,
                    ppid: 1,
                    uid: 501,
                    exe: "/usr/bin/ls".into(),
                    comm: Some("ls".into()),
                    argv: "ls -la".into(),
                    cwd: Some("/tmp".into()),
                    tty: None,
                    session_id: Some(42),
                    audit_id: Some("a1".into()),
                    exec_event_id: Some(7),
                    parent_exe: Some("/bin/bash".into()),
                    trace_id: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (pid, ppid, uid, exe, argv, cwd, parent_exe): (
        i64,
        i64,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT pid, ppid, uid, exe, argv, cwd, parent_exe FROM audit_events",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(pid, 100);
    assert_eq!(ppid, 1);
    assert_eq!(uid, 501);
    assert_eq!(exe, "/usr/bin/ls");
    assert_eq!(argv, "ls -la");
    assert_eq!(cwd.as_deref(), Some("/tmp"));
    assert_eq!(parent_exe.as_deref(), Some("/bin/bash"));
}

#[test]
fn dns_event_insert_populates_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("dns.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::DnsEvent(crate::events::DnsEvent {
                    timestamp: std::time::SystemTime::now(),
                    qname: "anthropic.com".into(),
                    qtype: 1,
                    qclass: 1,
                    rcode: 0,
                    decision: "allowed".into(),
                    matched_rule: None,
                    source_proto: Some("udp".into()),
                    process_name: None,
                    upstream_resolver_ms: 27,
                    trace_id: Some("abc1234567890def".into()),
                    policy_mode: None,
                    policy_action: None,
                    policy_rule: None,
                    policy_reason: None,
                }))
                .await;
            writer
                .write(WriteOp::DnsEvent(crate::events::DnsEvent {
                    timestamp: std::time::SystemTime::now(),
                    qname: "blocked.example.com".into(),
                    qtype: 28,
                    qclass: 1,
                    rcode: 3,
                    decision: "denied".into(),
                    matched_rule: Some("*.example.com".into()),
                    source_proto: Some("udp".into()),
                    process_name: None,
                    upstream_resolver_ms: 0,
                    trace_id: None,
                    policy_mode: Some("enforce".into()),
                    policy_action: Some("block".into()),
                    policy_rule: Some("policy.dns.block_example".into()),
                    policy_reason: Some("DNS block from Policy".into()),
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row = |sql: &str| -> (String, i64, i64, i64, String) {
        conn.query_row(sql, [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
    };
    let (qname, qtype, qclass, rcode, decision) = row(
        "SELECT qname, qtype, qclass, rcode, decision FROM dns_events
         WHERE qname = 'anthropic.com'",
    );
    let matched: Option<String> = conn
        .query_row(
            "SELECT matched_rule FROM dns_events WHERE qname = 'anthropic.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (proto, ms, trace): (Option<String>, i64, Option<String>) = conn
        .query_row(
            "SELECT source_proto, upstream_resolver_ms, trace_id FROM dns_events
         WHERE qname = 'anthropic.com'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(qname, "anthropic.com");
    assert_eq!(qtype, 1);
    assert_eq!(qclass, 1);
    assert_eq!(rcode, 0);
    assert_eq!(decision, "allowed");
    assert!(matched.is_none());
    assert_eq!(proto.as_deref(), Some("udp"));
    assert_eq!(ms, 27);
    assert_eq!(trace.as_deref(), Some("abc1234567890def"));

    let (rcode_blocked, matched_blocked): (i64, String) = conn
        .query_row(
            "SELECT rcode, matched_rule FROM dns_events WHERE qname = 'blocked.example.com'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(rcode_blocked, 3);
    assert_eq!(matched_blocked, "*.example.com");

    let (mode, action, rule, reason): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT policy_mode, policy_action, policy_rule, policy_reason
             FROM dns_events WHERE qname = 'blocked.example.com'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(mode.as_deref(), Some("enforce"));
    assert_eq!(action.as_deref(), Some("block"));
    assert_eq!(rule.as_deref(), Some("policy.dns.block_example"));
    assert_eq!(reason.as_deref(), Some("DNS block from Policy"));
}

#[test]
fn dns_events_indexed_by_trace_id_for_join() {
    // The promise of T3.3: a single trace_id joins dns_events to
    // net_events for one logical agent action. Verify the index
    // exists so the join is fast even at 100k+ rows.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("dns_idx.db");
    let _ = DbWriter::open(&db_path, 8).unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
         WHERE type='index' AND name='idx_dns_events_trace_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "missing idx_dns_events_trace_id");
}
