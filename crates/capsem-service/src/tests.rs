use super::*;
use capsem_core::settings_profiles::{VmArchAssets, VmAssetDeclaration};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

static SETTINGS_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn startup_asset_requirement_reads_profile_vm_assets() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("profiles/base");
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::write(
        profile_dir.join("everyday-work.toml"),
        r#"
version = 1
id = "everyday-work"
name = "Everyday Work"
best_for = "Daily sessions."
profile_type = "everyday-work"

[vm.assets.arm64.kernel]
url = "https://assets.example.test/vmlinuz"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
signature_url = "https://assets.example.test/vmlinuz.minisig"
size = 10
content_type = "application/octet-stream"

[vm.assets.arm64.initrd]
url = "https://assets.example.test/initrd.img"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
signature_url = "https://assets.example.test/initrd.img.minisig"
size = 11
content_type = "application/octet-stream"

[vm.assets.arm64.rootfs]
url = "https://assets.example.test/rootfs.squashfs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
signature_url = "https://assets.example.test/rootfs.squashfs.minisig"
size = 12
content_type = "application/vnd.squashfs"
"#,
    )
    .unwrap();
    let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
    settings.profiles.base_dirs = vec![profile_dir];
    settings.profiles.default_profile = "everyday-work".to_string();

    let requirement = startup_asset_requirement(&settings, "arm64", false).unwrap();

    let AssetRequirement::Profile(required) = requirement else {
        panic!("expected profile-backed asset requirement");
    };
    assert_eq!(required.asset_version(), "everyday-work");
    assert_eq!(required.expected_hashes().kernel, "a".repeat(64));
}

#[test]
fn startup_asset_requirement_includes_installed_profile_payload_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("profiles/base");
    let corp_dir = dir.path().join("profiles/corp");
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::create_dir_all(corp_dir.join(".catalog/profiles/everyday-work")).unwrap();
    std::fs::write(
        profile_dir.join("everyday-work.toml"),
        r#"
version = 1
id = "everyday-work"
name = "Everyday Work"
best_for = "Daily sessions."
profile_type = "everyday-work"

[vm.assets.arm64.kernel]
url = "https://assets.example.test/vmlinuz?token=secret"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
signature_url = "https://assets.example.test/vmlinuz.minisig"
size = 10
content_type = "application/octet-stream"

[vm.assets.arm64.initrd]
url = "https://assets.example.test/initrd.img"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
signature_url = "https://assets.example.test/initrd.img.minisig"
size = 11
content_type = "application/octet-stream"

[vm.assets.arm64.rootfs]
url = "https://assets.example.test/rootfs.squashfs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
signature_url = "https://assets.example.test/rootfs.squashfs.minisig"
size = 12
content_type = "application/vnd.squashfs"
"#,
    )
    .unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profiles/everyday-work/current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        }"#,
    )
    .unwrap();
    let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
    settings.profiles.base_dirs = vec![profile_dir];
    settings.profiles.corp_dirs = vec![corp_dir];
    settings.profiles.default_profile = "everyday-work".to_string();

    let requirement = startup_asset_requirement(&settings, "arm64", false).unwrap();
    let supervisor = AssetSupervisor::new(
        dir.path().join("assets"),
        requirement,
        std::time::Duration::from_secs(60),
    );
    let health = supervisor.snapshot();

    assert_eq!(health.profile_revision.as_deref(), Some("2026.0520.1"));
    assert_eq!(
        health.profile_payload_hash.as_deref(),
        Some("blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
    );
    assert_eq!(
        health.profile_assets[0].source_url,
        "https://assets.example.test/vmlinuz"
    );
}

#[test]
fn startup_asset_requirement_rejects_profiles_without_vm_assets_when_dev_fallback_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("profiles/base");
    std::fs::create_dir_all(&profile_dir).unwrap();
    write_profile_fixture(
        &profile_dir.join("everyday-work.toml"),
        "everyday-work",
        "Everyday Work",
    );
    let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
    settings.profiles.base_dirs = vec![profile_dir];
    settings.profiles.default_profile = "everyday-work".to_string();

    let err = startup_asset_requirement(&settings, "arm64", false).unwrap_err();

    assert!(
        format!("{err:#}").contains("old asset manifests are not runtime authority"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn process_env_allowlist_forwards_mcp_timeout_knobs() {
    assert!(
        PROCESS_ENV_ALLOWLIST.contains(&"CAPSEM_HOME"),
        "CAPSEM_HOME must reach capsem-process so tests and custom installs use the same config root as capsem-service"
    );

    for key in [
        "CAPSEM_MCP_DEFAULT_TIMEOUT_SECS",
        "CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS",
        "CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS",
        "CAPSEM_TEST_UPSTREAM_OVERRIDES",
    ] {
        assert!(
            PROCESS_ENV_ALLOWLIST.contains(&key),
            "{key} must reach capsem-process because McpTimeouts::from_env() is read there"
        );
    }
}

#[tokio::test]
async fn triage_session_db_surfaces_policy_signals() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let now = std::time::SystemTime::now();

    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            timestamp: now,
            domain: "blocked.example".into(),
            port: 443,
            decision: capsem_logger::Decision::Denied,
            process_name: Some("curl".into()),
            pid: Some(123),
            method: Some("GET".into()),
            path: Some("/".into()),
            query: None,
            status_code: Some(403),
            bytes_sent: 12,
            bytes_received: 0,
            duration_ms: 7,
            matched_rule: Some("blocked.example".into()),
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            conn_type: Some("https".into()),
            policy_mode: Some("v2".into()),
            policy_action: Some("block".into()),
            policy_rule: Some("policy.http.block_example".into()),
            policy_reason: Some("test block".into()),
            trace_id: Some("trace_t6".into()),
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::DnsEvent(capsem_logger::DnsEvent {
            timestamp: now,
            qname: "blocked.example".into(),
            qtype: 1,
            qclass: 1,
            rcode: 5,
            decision: "denied".into(),
            matched_rule: Some("blocked.example".into()),
            source_proto: Some("udp".into()),
            process_name: Some("curl".into()),
            upstream_resolver_ms: 0,
            trace_id: Some("trace_t6".into()),
            policy_mode: Some("v2".into()),
            policy_action: Some("block".into()),
            policy_rule: Some("policy.dns.block_example".into()),
            policy_reason: Some("test dns block".into()),
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::McpCall(capsem_logger::McpCall {
            timestamp: now,
            server_name: "builtin".into(),
            method: "tools/call".into(),
            tool_name: Some("danger".into()),
            request_id: Some("req1".into()),
            request_preview: Some("{}".into()),
            response_preview: None,
            decision: "error".into(),
            duration_ms: 5,
            error_message: Some("policy denied".into()),
            process_name: Some("agent".into()),
            bytes_sent: 2,
            bytes_received: 0,
            policy_mode: Some("v2".into()),
            policy_action: Some("block".into()),
            policy_rule: Some("policy.mcp.block_danger".into()),
            policy_reason: Some("test mcp block".into()),
            trace_id: Some("trace_t6".into()),
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEvent(
            capsem_logger::ExecEvent {
                timestamp: now,
                exec_id: 44,
                command: "false".into(),
                source: "api".into(),
                mcp_call_id: None,
                trace_id: Some("trace_t6".into()),
                process_name: Some("false".into()),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 44,
                exit_code: 1,
                duration_ms: 9,
                stdout_preview: None,
                stderr_preview: Some("nope".into()),
                stdout_bytes: 0,
                stderr_bytes: 4,
                pid: Some(444),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::PolicyHookEvent(
            capsem_logger::events::PolicyHookEvent {
                timestamp: now,
                endpoint_id: "corp-hook".into(),
                spec_version: "0.1.0".into(),
                spec_hash: "sha256:test".into(),
                decision_id: None,
                callback: "mcp".into(),
                decision: None,
                rule_id: None,
                reason: None,
                latency_ms: 12,
                status: "error".into(),
                error: Some("schema violation".into()),
                fallback: Some("fail_closed".into()),
                audit_tags: vec!["test".into()],
                trace_id: Some("trace_t6".into()),
                session_id: Some("vm-t6".into()),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::AuditEvent(
            capsem_logger::AuditEvent {
                timestamp: now,
                pid: 444,
                ppid: 1,
                uid: 1000,
                exe: "/usr/bin/false".into(),
                comm: Some("false".into()),
                argv: "false".into(),
                cwd: Some("/capsem/workspace".into()),
                tty: None,
                session_id: Some(1),
                audit_id: Some("audit-t6".into()),
                exec_event_id: Some(44),
                parent_exe: Some("/bin/sh".into()),
                trace_id: Some("trace_t6".into()),
            },
        ))
        .await;
    drop(writer);

    let triage = session_db_triage(&db_path, 10).unwrap();
    let text = triage.to_string();
    for expected in [
        "policy.http.block_example",
        "policy.dns.block_example",
        "policy.mcp.block_danger",
        "corp-hook",
        "fail_closed",
        "audit-t6",
        "trace_t6",
    ] {
        assert!(
            text.contains(expected),
            "triage output should contain {expected}: {text}"
        );
    }
}

#[test]
fn timeline_allowed_layers_include_policy_tables() {
    for expected in ["dns", "hook", "audit", "snapshot"] {
        assert!(
            ALLOWED_TIMELINE_LAYERS.contains(&expected),
            "timeline layer allowlist missing {expected}"
        );
    }
}

#[test]
fn timeline_existing_tables_lists_policy_tables() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    drop(writer);
    let reader = capsem_logger::DbReader::open(&db_path).unwrap();

    let tables = timeline_existing_tables(&reader).unwrap();

    for expected in [
        "dns_events",
        "policy_hook_events",
        "audit_events",
        "snapshot_events",
    ] {
        assert!(
            tables.contains(expected),
            "timeline schema discovery missing {expected}: {tables:?}"
        );
    }
}

#[test]
fn timeline_column_helpers_fallback_for_legacy_schema() {
    let columns = HashMap::from([(
        "net_events".to_string(),
        HashSet::from([
            "id".to_string(),
            "timestamp".to_string(),
            "domain".to_string(),
            "decision".to_string(),
        ]),
    )]);

    assert_eq!(
        timeline_col(&columns, "net_events", "trace_id", "NULL"),
        "NULL"
    );
    assert_eq!(timeline_policy_suffix(&columns, "net_events", None), "''");
}

#[test]
fn timeline_column_helpers_emit_policy_suffix_for_current_schema() {
    let columns = HashMap::from([(
        "mcp_calls".to_string(),
        HashSet::from([
            "id".to_string(),
            "timestamp".to_string(),
            "policy_action".to_string(),
            "policy_rule".to_string(),
            "trace_id".to_string(),
        ]),
    )]);

    assert_eq!(
        timeline_alias_col(&columns, "mcp_calls", "m", "trace_id", "NULL"),
        "m.trace_id"
    );
    assert_eq!(
        timeline_policy_suffix(&columns, "mcp_calls", Some("m")),
        "COALESCE(' policy=' || m.policy_action || '/' || m.policy_rule, '')"
    );
}

#[tokio::test]
async fn timeline_handler_returns_policy_layers_and_null_trace_rows() {
    let (state, _dir) = make_test_state_with_tempdir();
    let vm_id = "timeline-vm";
    let session_dir = state.run_dir.join("sessions").join(vm_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let now = std::time::SystemTime::now();

    writer
        .write(capsem_logger::WriteOp::ModelCall(
            capsem_logger::ModelCall {
                timestamp: now,
                provider: "anthropic".into(),
                model: Some("claude".into()),
                process_name: Some("agent".into()),
                pid: Some(10),
                method: "POST".into(),
                path: "/v1/messages".into(),
                stream: false,
                system_prompt_preview: None,
                messages_count: 1,
                tools_count: 0,
                request_bytes: 2,
                request_body_preview: Some("{}".into()),
                message_id: Some("msg_t6".into()),
                status_code: Some(200),
                text_content: Some("ok".into()),
                thinking_content: None,
                stop_reason: Some("end_turn".into()),
                input_tokens: Some(3),
                output_tokens: Some(4),
                usage_details: Default::default(),
                duration_ms: 20,
                response_bytes: 5,
                estimated_cost_usd: 0.0,
                trace_id: Some("trace_t6".into()),
                ai_evidence: None,
                tool_calls: Vec::new(),
                tool_responses: Vec::new(),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::McpCall(capsem_logger::McpCall {
            timestamp: now,
            server_name: "builtin".into(),
            method: "tools/call".into(),
            tool_name: Some("policy_check".into()),
            request_id: Some("req_t6".into()),
            request_preview: Some("{}".into()),
            response_preview: Some("{\"ok\":true}".into()),
            decision: "allowed".into(),
            duration_ms: 11,
            error_message: None,
            process_name: Some("agent".into()),
            bytes_sent: 2,
            bytes_received: 3,
            policy_mode: Some("v2".into()),
            policy_action: Some("allow".into()),
            policy_rule: Some("policy.mcp.allow_policy_check".into()),
            policy_reason: Some("fixture".into()),
            trace_id: Some("trace_t6".into()),
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            timestamp: now,
            domain: "example.com".into(),
            port: 443,
            decision: capsem_logger::Decision::Allowed,
            process_name: Some("curl".into()),
            pid: Some(20),
            method: Some("GET".into()),
            path: Some("/".into()),
            query: None,
            status_code: Some(200),
            bytes_sent: 10,
            bytes_received: 20,
            duration_ms: 3,
            matched_rule: Some("example.com".into()),
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            conn_type: Some("https".into()),
            policy_mode: Some("v2".into()),
            policy_action: Some("allow".into()),
            policy_rule: Some("policy.http.allow_example".into()),
            policy_reason: Some("fixture".into()),
            trace_id: Some("trace_t6".into()),
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::DnsEvent(capsem_logger::DnsEvent {
            timestamp: now,
            qname: "example.com".into(),
            qtype: 1,
            qclass: 1,
            rcode: 0,
            decision: "allowed".into(),
            matched_rule: Some("example.com".into()),
            source_proto: Some("udp".into()),
            process_name: Some("curl".into()),
            upstream_resolver_ms: 1,
            trace_id: Some("trace_t6".into()),
            policy_mode: Some("v2".into()),
            policy_action: Some("allow".into()),
            policy_rule: Some("policy.dns.allow_example".into()),
            policy_reason: Some("fixture".into()),
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEvent(
            capsem_logger::ExecEvent {
                timestamp: now,
                exec_id: 77,
                command: "echo timeline".into(),
                source: "api".into(),
                mcp_call_id: None,
                trace_id: Some("trace_t6".into()),
                process_name: Some("sh".into()),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 77,
                exit_code: 0,
                duration_ms: 2,
                stdout_preview: Some("timeline".into()),
                stderr_preview: None,
                stdout_bytes: 8,
                stderr_bytes: 0,
                pid: Some(77),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::FileEvent(
            capsem_logger::FileEvent {
                timestamp: now,
                action: capsem_logger::FileAction::Created,
                path: "timeline.txt".into(),
                size: Some(8),
                trace_id: Some("trace_t6".into()),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::FileEvent(
            capsem_logger::FileEvent {
                timestamp: now,
                action: capsem_logger::FileAction::Modified,
                path: "pre-trace.txt".into(),
                size: Some(1),
                trace_id: None,
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::SnapshotEvent(
            capsem_logger::SnapshotEvent {
                timestamp: now,
                slot: 1,
                origin: "manual".into(),
                name: Some("checkpoint".into()),
                files_count: 2,
                start_fs_event_id: 0,
                stop_fs_event_id: 2,
                trace_id: Some("trace_t6".into()),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::PolicyHookEvent(
            capsem_logger::events::PolicyHookEvent {
                timestamp: now,
                endpoint_id: "hook".into(),
                spec_version: "policy-hook/v0".into(),
                spec_hash: "sha256:timeline".into(),
                decision_id: Some("decision_t6".into()),
                callback: "http.request".into(),
                decision: Some("allow".into()),
                rule_id: Some("policy.hook.allow_example".into()),
                reason: Some("fixture".into()),
                latency_ms: 4,
                status: "allowed".into(),
                error: None,
                fallback: None,
                audit_tags: vec!["timeline".into()],
                trace_id: Some("trace_t6".into()),
                session_id: Some(vm_id.into()),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::AuditEvent(
            capsem_logger::AuditEvent {
                timestamp: now,
                pid: 77,
                ppid: 1,
                uid: 1000,
                exe: "/bin/echo".into(),
                comm: Some("echo".into()),
                argv: "echo timeline".into(),
                cwd: Some("/capsem/workspace".into()),
                tty: None,
                session_id: Some(1),
                audit_id: Some("audit_t6".into()),
                exec_event_id: Some(77),
                parent_exe: Some("/bin/sh".into()),
                trace_id: Some("trace_t6".into()),
            },
        ))
        .await;
    drop(writer);

    state.instances.lock().unwrap().insert(
        vm_id.into(),
        InstanceInfo {
            id: vm_id.into(),
            pid: std::process::id(),
            uds_path: state.run_dir.join("timeline.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: None,
            profile_pin: None,
        },
    );

    let response = handle_timeline(
        State(state),
        Path(vm_id.into()),
        axum::extract::Query(TimelineQuery {
            trace_id: Some("trace_t6".into()),
            since: None,
            limit: Some(100),
            layers: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let rows = json["rows"].as_array().unwrap();
    let layers: HashSet<String> = rows
        .iter()
        .filter_map(|row| row.as_array()?.get(1)?.as_str().map(str::to_string))
        .collect();

    for expected in [
        "exec", "mcp", "net", "dns", "hook", "audit", "snapshot", "fs", "model",
    ] {
        assert!(
            layers.contains(expected),
            "missing timeline layer {expected}: {json}"
        );
    }
    assert!(
        rows.iter().any(|row| row
            .as_array()
            .and_then(|cells| cells.get(6))
            .is_some_and(|trace| trace.is_null())),
        "trace filter should retain pre-trace NULL rows: {json}"
    );
}

#[test]
fn find_orphan_capsem_pids_matches_capsem_process_under_run_dir() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-abc");
    let ps = "\
  1502 /path/to/target/debug/capsem-process --env CAPSEM_VM_ID=orphan --id orphan --session-dir /var/folders/XY/T/capsem-test-abc/sessions/orphan --uds-path /tmp/capsem/abc.sock
  1742 /path/to/target/debug/capsem-process --id victim --session-dir /var/folders/XY/T/capsem-test-abc/persistent/victim --uds-path /tmp/capsem/def.sock
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(pids, vec![1502, 1742]);
}

#[test]
fn find_orphan_capsem_pids_skips_processes_for_other_run_dirs() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-mine");
    let ps = "\
  1502 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-other/sessions/foo
  1742 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-mine/sessions/bar
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(
        pids,
        vec![1742],
        "must not match neighbouring test run dirs"
    );
}

#[test]
fn find_orphan_capsem_pids_skips_non_capsem_process_binaries() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-abc");
    // A stray cargo invocation that happens to mention the run_dir path.
    let ps = "\
  99 /bin/cargo build --manifest-path /var/folders/XY/T/capsem-test-abc/Cargo.toml
  1502 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-abc/sessions/orphan
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(
        pids,
        vec![1502],
        "match must require 'capsem-process' in the line"
    );
}

#[test]
fn find_orphan_capsem_pids_returns_empty_on_no_match() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-empty");
    let ps = "\
  1 /sbin/launchd
  42 /usr/bin/bash
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert!(pids.is_empty());
}

fn test_magika() -> Mutex<magika::Session> {
    Mutex::new(
        magika::Session::builder()
            .with_inter_threads(1)
            .with_intra_threads(1)
            .build()
            .expect("magika init"),
    )
}

fn test_asset_supervisor(assets_dir: PathBuf) -> Arc<AssetSupervisor> {
    Arc::new(AssetSupervisor::new(
        assets_dir,
        AssetRequirement::DevLogical {
            arch: host_asset_arch().to_string(),
        },
        std::time::Duration::from_secs(60),
    ))
}

fn test_profile_asset_declaration(base_url: &str, name: &str, bytes: &[u8]) -> VmAssetDeclaration {
    VmAssetDeclaration {
        url: format!("{base_url}/{name}"),
        hash: format!("blake3:{}", blake3::hash(bytes).to_hex()),
        signature_url: format!("{base_url}/{name}.minisig"),
        size: bytes.len() as u64,
        content_type: "application/octet-stream".to_string(),
    }
}

fn test_profile_asset_supervisor(assets_dir: PathBuf, base_url: &str) -> Arc<AssetSupervisor> {
    Arc::new(AssetSupervisor::new(
        assets_dir,
        AssetRequirement::Profile(Box::new(
            ProfileAssetRequirement::new(
                "everyday-work".to_string(),
                Some("2026.0520.1".to_string()),
                host_asset_arch().to_string(),
                VmArchAssets {
                    kernel: test_profile_asset_declaration(base_url, "vmlinuz", b"kernel"),
                    initrd: test_profile_asset_declaration(base_url, "initrd.img", b"initrd"),
                    rootfs: test_profile_asset_declaration(base_url, "rootfs.squashfs", b"rootfs"),
                },
            )
            .with_profile_payload_hash(Some(test_profile_payload_hash())),
        )),
        std::time::Duration::from_secs(60),
    ))
}

async fn start_test_asset_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0_u8; 2048];
                let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                    .await
                    .unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .trim_start_matches('/');
                let body = match path {
                    "vmlinuz" => Some(b"kernel".as_slice()),
                    "initrd.img" => Some(b"initrd".as_slice()),
                    "rootfs.squashfs" => Some(b"rootfs".as_slice()),
                    _ => None,
                };
                if let Some(body) = body {
                    let header =
                        format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
                    let _ =
                        tokio::io::AsyncWriteExt::write_all(&mut stream, header.as_bytes()).await;
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, body).await;
                } else {
                    let _ = tokio::io::AsyncWriteExt::write_all(
                        &mut stream,
                        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n",
                    )
                    .await;
                }
            });
        }
    });
    (format!("http://{addr}"), handle)
}

async fn start_profile_catalog_manifest_server(
    manifest_json: String,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let manifest_json = manifest_json.clone();
            tokio::spawn(async move {
                let mut buf = [0_u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                let header = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                    manifest_json.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, header.as_bytes()).await;
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, manifest_json.as_bytes())
                    .await;
            });
        }
    });
    (format!("http://{addr}/profile-catalog.json"), handle)
}

async fn start_counted_blocking_asset_server() -> (
    String,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let first_request_seen = Arc::new(tokio::sync::Notify::new());
    let release_first_response = Arc::new(tokio::sync::Notify::new());
    let blocked_first_response = Arc::new(AtomicBool::new(false));

    let handle = {
        let request_count = Arc::clone(&request_count);
        let first_request_seen = Arc::clone(&first_request_seen);
        let release_first_response = Arc::clone(&release_first_response);
        let blocked_first_response = Arc::clone(&blocked_first_response);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let request_count = Arc::clone(&request_count);
                let first_request_seen = Arc::clone(&first_request_seen);
                let release_first_response = Arc::clone(&release_first_response);
                let blocked_first_response = Arc::clone(&blocked_first_response);
                tokio::spawn(async move {
                    let mut buf = [0_u8; 2048];
                    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                        .await
                        .unwrap_or(0);
                    request_count.fetch_add(1, Ordering::SeqCst);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .trim_start_matches('/');
                    let body = match path {
                        "vmlinuz" => Some(b"kernel".as_slice()),
                        "initrd.img" => Some(b"initrd".as_slice()),
                        "rootfs.squashfs" => Some(b"rootfs".as_slice()),
                        _ => None,
                    };
                    if let Some(body) = body {
                        if !blocked_first_response.swap(true, Ordering::SeqCst) {
                            first_request_seen.notify_one();
                            release_first_response.notified().await;
                        }
                        let header =
                            format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
                        let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, header.as_bytes())
                            .await;
                        let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, body).await;
                    } else {
                        let _ = tokio::io::AsyncWriteExt::write_all(
                            &mut stream,
                            b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n",
                        )
                        .await;
                    }
                });
            }
        })
    };

    (
        format!("http://{addr}"),
        handle,
        request_count,
        first_request_seen,
        release_first_response,
    )
}

fn test_asset_locations(
    assets_dir: PathBuf,
) -> capsem_core::settings_profiles::ResolvedServiceAssetLocations {
    capsem_core::settings_profiles::ResolvedServiceAssetLocations {
        assets_dir,
        assets_dir_origin: capsem_core::settings_profiles::ServiceSettingOrigin::Default,
        image_roots: Vec::new(),
        image_roots_origin: capsem_core::settings_profiles::ServiceSettingOrigin::Default,
        download_base_url: None,
    }
}

fn test_service_settings(run_dir: &FsPath) -> capsem_core::settings_profiles::ServiceSettings {
    let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
    let base_dir = run_dir.join("profiles/base");
    let corp_dir = run_dir.join("profiles/corp");
    let user_dir = run_dir.join("profiles/user");
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::create_dir_all(&corp_dir).unwrap();
    std::fs::create_dir_all(&user_dir).unwrap();
    settings.profiles.base_dirs = vec![base_dir];
    settings.profiles.corp_dirs = vec![corp_dir];
    settings.profiles.user_dirs = vec![user_dir];
    settings.profiles.default_profile =
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID.to_string();
    settings
}

fn make_test_state() -> Arc<ServiceState> {
    let registry_path = PathBuf::from("/tmp/capsem-test-svc/persistent_registry.json");
    let assets_dir = PathBuf::from("/nonexistent/assets");
    let current_version = "0.0.0";
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: assets_dir.clone(),
        asset_locations: test_asset_locations(assets_dir.clone()),
        service_settings: test_service_settings(FsPath::new("/tmp/capsem-test-svc")),
        run_dir: PathBuf::from("/tmp/capsem-test-svc"),
        job_counter: AtomicU64::new(1),
        asset_supervisor: test_asset_supervisor(assets_dir),
        current_version: current_version.into(),
        magika: test_magika(),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    })
}

#[tokio::test]
async fn handle_debug_report_returns_pasteable_text() {
    let (state, _dir) = make_test_state_with_tempdir();
    insert_fake_instance(&state, "debug-vm", std::process::id());

    let Json(report) = handle_debug_report(State(state)).await.unwrap();

    assert!(report.text.contains("Capsem Debug Report"));
    assert!(report.text.contains("capsem_version: 0.0.0"));
    assert!(report.text.contains("running_vm_count: 1"));
    assert!(report.text.contains("source: profile_v2_asset_health"));
    assert!(report.text.contains("profile_asset_health_present: true"));
}

#[tokio::test]
async fn handle_list_exposes_service_asset_supervisor_state() {
    let (state, _dir) = make_test_state_with_tempdir();
    state.asset_supervisor.refresh_local_state();

    let Json(list) = handle_list(State(state)).await;

    let assets = list.asset_health.expect("asset health should be present");
    assert_eq!(assets.state, AssetHealthState::Updating);
    assert!(!assets.ready);
    assert_eq!(
        assets.missing,
        vec!["vmlinuz", "initrd.img", "rootfs.squashfs"]
    );
}

#[tokio::test]
async fn handle_asset_status_exposes_service_asset_locations() {
    let (state, _dir) = make_test_state_with_tempdir();
    state.asset_supervisor.refresh_local_state();

    let Json(status) = handle_asset_status(State(state)).await;

    assert_eq!(
        status["asset_locations"]["assets_dir_origin"],
        serde_json::json!("default")
    );
    assert!(status["asset_locations"].get("manifest_source").is_none());
}

#[tokio::test]
async fn handle_asset_cleanup_preserves_profile_and_saved_vm_retention() {
    let (state, _dir) = make_test_state_with_tempdir();
    std::fs::create_dir_all(&state.assets_dir).unwrap();
    std::fs::write(state.assets_dir.join("vmlinuz"), b"current kernel").unwrap();
    std::fs::write(state.assets_dir.join("initrd.img"), b"current initrd").unwrap();
    std::fs::write(state.assets_dir.join("rootfs.squashfs"), b"current rootfs").unwrap();
    state.asset_supervisor.refresh_local_state();

    let corp_dir = state.service_settings.profiles.corp_dirs[0].clone();
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    std::fs::create_dir_all(record_dir.join("2026.0520.1")).unwrap();
    std::fs::write(
        record_dir.join("2026.0520.1").join("profile.json"),
        include_str!("../../../schemas/fixtures/profile-v2-valid.json"),
    )
    .unwrap();
    std::fs::write(
        record_dir.join("current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        }"#,
    )
    .unwrap();

    let arm64 = state.assets_dir.join("arm64");
    let legacy = state.assets_dir.join("v1.0.1776269479");
    std::fs::create_dir(&arm64).unwrap();
    std::fs::create_dir(&legacy).unwrap();
    let profile_kernel = arm64.join("vmlinuz-aaaaaaaaaaaaaaaa");
    let saved_kernel = arm64.join("vmlinuz-dddddddddddddddd");
    let stale_rootfs = arm64.join("rootfs-9999999999999999.squashfs");
    std::fs::write(&profile_kernel, b"profile kernel").unwrap();
    std::fs::write(&saved_kernel, b"saved kernel").unwrap();
    std::fs::write(&stale_rootfs, b"stale rootfs").unwrap();
    std::fs::write(legacy.join("rootfs.squashfs"), b"legacy").unwrap();

    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry.data.vms.insert(
            "saved-assets".into(),
            PersistentVmEntry {
                name: "saved-assets".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: Some(SavedVmBaseAssets {
                    asset_version: "saved-profile@2026.0520.1".into(),
                    arch: "arm64".into(),
                    kernel_hash: "d".repeat(64),
                    initrd_hash: "e".repeat(64),
                    rootfs_hash: "f".repeat(64),
                    guest_abi: Some("capsem-guest-v2".into()),
                }),
                profile_pin: None,
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/saved-assets"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let Json(result) = handle_asset_cleanup(State(state)).await.unwrap();

    assert_eq!(result["mode"], serde_json::json!("settings_profiles_v2"));
    assert_eq!(result["skipped"], serde_json::json!(false));
    assert_eq!(result["removed_count"], serde_json::json!(2));
    assert!(profile_kernel.exists());
    assert!(saved_kernel.exists());
    assert!(!stale_rootfs.exists());
    assert!(!legacy.exists());
}

#[tokio::test]
async fn handle_asset_cleanup_refuses_while_assets_are_updating() {
    let (state, _dir) = make_test_state_with_tempdir();
    std::fs::create_dir_all(state.assets_dir.join("arm64")).unwrap();
    let stale = state
        .assets_dir
        .join("arm64")
        .join("rootfs-9999999999999999.squashfs");
    std::fs::write(&stale, b"stale rootfs").unwrap();
    state.asset_supervisor.refresh_local_state();

    let err = handle_asset_cleanup(State(state)).await.unwrap_err();

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err
        .1
        .contains("asset cleanup is blocked while assets are updating"));
    assert!(stale.exists());
}

#[test]
fn ensure_vm_effective_settings_writes_default_profile_attachment() {
    let _env_lock = SETTINGS_ENV_LOCK.blocking_lock();
    let env_dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&env_dir);
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions").join("vm-effective");
    std::fs::create_dir_all(&session_dir).unwrap();

    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let loaded = capsem_core::settings_profiles::load_vm_effective_settings(&session_dir).unwrap();

    assert_eq!(
        loaded.profile_id,
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID
    );
}

#[test]
fn ensure_vm_effective_settings_regenerates_corrupt_file() {
    let _env_lock = SETTINGS_ENV_LOCK.blocking_lock();
    let env_dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&env_dir);
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions").join("vm-corrupt-effective");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(
        capsem_core::settings_profiles::vm_effective_settings_path(&session_dir),
        "not = [valid",
    )
    .unwrap();

    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let loaded = capsem_core::settings_profiles::load_vm_effective_settings(&session_dir).unwrap();

    assert_eq!(
        loaded.profile_id,
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID
    );
}

#[test]
fn ensure_vm_effective_settings_attaches_trace_alongside_settings() {
    let _env_lock = SETTINGS_ENV_LOCK.blocking_lock();
    let env_dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&env_dir);
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions").join("vm-effective-trace");
    std::fs::create_dir_all(&session_dir).unwrap();

    state.ensure_vm_effective_settings(&session_dir).unwrap();

    let trace = capsem_core::settings_profiles::load_vm_effective_trace(&session_dir).unwrap();
    assert!(
        !trace.events.is_empty(),
        "trace should contain at least the schema-default + profile events"
    );
    let head = trace.events.first().unwrap();
    assert_eq!(
        head.source_kind,
        capsem_core::settings_profiles::ResolverTraceSourceKind::Default
    );
}

#[test]
fn ensure_vm_effective_settings_regenerates_corrupt_trace_file() {
    let _env_lock = SETTINGS_ENV_LOCK.blocking_lock();
    let env_dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&env_dir);
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions").join("vm-corrupt-trace");
    std::fs::create_dir_all(&session_dir).unwrap();
    state.ensure_vm_effective_settings(&session_dir).unwrap();
    std::fs::write(
        capsem_core::settings_profiles::vm_effective_trace_path(&session_dir),
        "{ broken json",
    )
    .unwrap();

    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let trace = capsem_core::settings_profiles::load_vm_effective_trace(&session_dir).unwrap();
    assert!(!trace.events.is_empty());
}

#[test]
fn ensure_vm_effective_settings_regenerates_pair_when_trace_missing() {
    let _env_lock = SETTINGS_ENV_LOCK.blocking_lock();
    let env_dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&env_dir);
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir
        .path()
        .join("sessions")
        .join("vm-effective-trace-missing");
    std::fs::create_dir_all(&session_dir).unwrap();
    state.ensure_vm_effective_settings(&session_dir).unwrap();
    std::fs::remove_file(capsem_core::settings_profiles::vm_effective_trace_path(
        &session_dir,
    ))
    .unwrap();

    state.ensure_vm_effective_settings(&session_dir).unwrap();
    assert!(capsem_core::settings_profiles::vm_effective_trace_path(&session_dir).is_file());
    let loaded = capsem_core::settings_profiles::load_vm_effective_settings(&session_dir).unwrap();
    assert_eq!(
        loaded.profile_id,
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID
    );
}

fn test_saved_vm_base_assets() -> capsem_service::registry::SavedVmBaseAssets {
    capsem_service::registry::SavedVmBaseAssets {
        asset_version: "2026.0415.1".into(),
        arch: host_asset_arch().into(),
        kernel_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        initrd_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        rootfs_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
        guest_abi: Some("capsem-guest-v2".into()),
    }
}

fn test_saved_vm_profile_pin(
    base_assets: capsem_service::registry::SavedVmBaseAssets,
) -> SavedVmProfilePin {
    SavedVmProfilePin {
        profile_id: "everyday-work".into(),
        profile_revision: Some("2026.0520.1".into()),
        profile_payload_hash: Some(format!("blake3:{}", "e".repeat(64))),
        package_contract_hash: format!("blake3:{}", "d".repeat(64)),
        base_assets: Some(base_assets),
    }
}

fn test_profile_payload_hash() -> String {
    format!("blake3:{}", "e".repeat(64))
}

fn spawn_single_exec_server(
    sock_path: PathBuf,
    stdout: &'static [u8],
) -> std::thread::JoinHandle<()> {
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let _ = std::fs::remove_file(&sock_path);
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    std::fs::write(sock_path.with_extension("ready"), b"ready").unwrap();
    std::thread::spawn(move || {
        let (mut std_stream, _) = listener.accept().unwrap();
        capsem_core::ipc_handshake::negotiate_responder(&mut std_stream, "capsem-process-test", "")
            .unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let (tx, rx): (Sender<ProcessToService>, Receiver<ServiceToProcess>) =
                    channel_from_std(std_stream).unwrap();
                match rx.recv().await.unwrap() {
                    ServiceToProcess::Exec { id, .. } => {
                        tx.send(ProcessToService::ExecResult {
                            id,
                            stdout: stdout.to_vec(),
                            stderr: Vec::new(),
                            exit_code: 0,
                        })
                        .await
                        .unwrap();
                    }
                    other => panic!("unexpected command: {other:?}"),
                }
            });
    })
}

#[test]
fn saved_vm_current_base_assets_from_profile_records_boot_hashes() {
    let profile_assets = capsem_core::settings_profiles::VmArchAssets {
        kernel: capsem_core::settings_profiles::VmAssetDeclaration {
            url: "https://assets.example.test/vmlinuz".to_string(),
            hash: "blake3:a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"
                .to_string(),
            signature_url: "https://assets.example.test/vmlinuz.minisig".to_string(),
            size: 7_797_248,
            content_type: "application/octet-stream".to_string(),
        },
        initrd: capsem_core::settings_profiles::VmAssetDeclaration {
            url: "https://assets.example.test/initrd.img".to_string(),
            hash: "blake3:cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456"
                .to_string(),
            signature_url: "https://assets.example.test/initrd.img.minisig".to_string(),
            size: 2_270_154,
            content_type: "application/octet-stream".to_string(),
        },
        rootfs: capsem_core::settings_profiles::VmAssetDeclaration {
            url: "https://assets.example.test/rootfs.squashfs".to_string(),
            hash: "blake3:b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee"
                .to_string(),
            signature_url: "https://assets.example.test/rootfs.squashfs.minisig".to_string(),
            size: 454_230_016,
            content_type: "application/vnd.squashfs".to_string(),
        },
    };
    let supervisor = AssetSupervisor::new(
        PathBuf::from("/tmp/assets"),
        AssetRequirement::Profile(Box::new(ProfileAssetRequirement::new(
            "everyday-work".to_string(),
            Some("2026.0415.1".to_string()),
            "arm64".to_string(),
            profile_assets,
        ))),
        std::time::Duration::from_secs(60),
    );
    let base_assets = supervisor.current_base_assets().unwrap();

    assert_eq!(base_assets.asset_version, "everyday-work@2026.0415.1");
    assert_eq!(base_assets.arch, "arm64");
    assert_eq!(
        base_assets.kernel_hash,
        "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"
    );
    assert_eq!(
        base_assets.initrd_hash,
        "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456"
    );
    assert_eq!(
        base_assets.rootfs_hash,
        "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee"
    );
    assert_eq!(base_assets.guest_abi.as_deref(), Some("capsem-guest-v2"));
}

#[test]
fn vm_profile_pin_hashes_effective_package_contract_and_assets() {
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions/profile-pin");
    std::fs::create_dir_all(&session_dir).unwrap();
    let mut effective = capsem_core::settings_profiles::resolve_effective_vm_settings(
        &capsem_core::settings_profiles::ProfileRootSettings::default(),
        None,
    )
    .unwrap();
    effective
        .packages
        .value
        .runtimes
        .insert("python".to_string(), "3.12.3".to_string());
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

    let base_assets = test_saved_vm_base_assets();
    let pin = state
        .vm_profile_pin(
            &session_dir,
            Some("2026.0518.1".to_string()),
            Some(test_profile_payload_hash()),
            Some(base_assets.clone()),
        )
        .unwrap();
    let package_json = serde_json::to_vec(&effective.packages.value).unwrap();
    let expected_hash = format!("blake3:{}", blake3::hash(&package_json).to_hex());

    assert_eq!(pin.profile_id, "everyday-work");
    assert_eq!(pin.profile_revision.as_deref(), Some("2026.0518.1"));
    assert_eq!(
        pin.profile_payload_hash.as_deref(),
        Some(test_profile_payload_hash().as_str())
    );
    assert_eq!(pin.package_contract_hash, expected_hash);
    assert_eq!(pin.base_assets, Some(base_assets));
}

#[test]
fn vm_profile_pin_uses_installed_profile_revision_sidecar() {
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions/profile-pin-installed");
    std::fs::create_dir_all(&session_dir).unwrap();
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(
        &capsem_core::settings_profiles::ProfileRootSettings::default(),
        None,
    )
    .unwrap();
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();
    let corp_dir = state.service_settings.profiles.corp_dirs[0].clone();
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    let revision_dir = record_dir.join("2026.0520.1");
    std::fs::create_dir_all(&revision_dir).unwrap();
    std::fs::write(
        corp_dir.join("everyday-work.toml"),
        "version = 1\nid = \"everyday-work\"\n",
    )
    .unwrap();
    let payload = br#"{"id":"everyday-work"}"#;
    std::fs::write(revision_dir.join("profile.json"), payload).unwrap();
    let payload_hash = format!("blake3:{}", blake3::hash(payload).to_hex());
    std::fs::write(
        record_dir.join("current.json"),
        format!(
            r#"{{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "{payload_hash}"
        }}"#,
        ),
    )
    .unwrap();

    let pin = state
        .vm_profile_pin(&session_dir, None, None, Some(test_saved_vm_base_assets()))
        .unwrap();

    assert_eq!(pin.profile_id, "everyday-work");
    assert_eq!(pin.profile_revision.as_deref(), Some("2026.0520.1"));
    assert_eq!(
        pin.profile_payload_hash.as_deref(),
        Some(payload_hash.as_str())
    );
}

#[test]
fn vm_profile_pin_requires_signed_catalog_revision() {
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions/profile-pin-no-revision");
    std::fs::create_dir_all(&session_dir).unwrap();
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(
        &capsem_core::settings_profiles::ProfileRootSettings::default(),
        None,
    )
    .unwrap();
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

    let err = state
        .vm_profile_pin(&session_dir, None, None, Some(test_saved_vm_base_assets()))
        .unwrap_err();

    assert!(
        format!("{err:#}").contains("signed profile catalog revision"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn vm_profile_pin_requires_profile_payload_hash() {
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions/profile-pin-no-payload-hash");
    std::fs::create_dir_all(&session_dir).unwrap();
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(
        &capsem_core::settings_profiles::ProfileRootSettings::default(),
        None,
    )
    .unwrap();
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

    let err = state
        .vm_profile_pin(
            &session_dir,
            Some("2026.0520.1".into()),
            None,
            Some(test_saved_vm_base_assets()),
        )
        .unwrap_err();

    assert!(
        format!("{err:#}").contains("profile payload hash"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn required_vm_profile_pin_requires_profile_payload_hash() {
    let base_assets = test_saved_vm_base_assets();
    let mut pin = test_saved_vm_profile_pin(base_assets);
    pin.profile_payload_hash = None;

    let err = ensure_required_vm_profile_pin(Some(&pin), "source VM \"missing-hash\"").unwrap_err();

    assert!(
        format!("{err:#}").contains("profile payload hash"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn source_vm_base_assets_uses_profile_pin_as_authority() {
    let base_assets = test_saved_vm_base_assets();
    let entry = PersistentVmEntry {
        name: "source-vm".into(),
        ram_mb: 2048,
        cpus: 2,
        base_version: "0.0.0".into(),
        base_assets: None,
        profile_pin: Some(test_saved_vm_profile_pin(base_assets.clone())),
        created_at: "0".into(),
        session_dir: PathBuf::from("/tmp/source-vm"),
        forked_from: None,
        description: None,
        suspended: false,
        defunct: false,
        last_error: None,
        checkpoint_path: None,
        env: None,
    };

    assert_eq!(source_vm_base_assets(&entry).unwrap(), base_assets);
}

#[test]
fn source_vm_base_assets_rejects_registry_pin_drift() {
    let profile_assets = test_saved_vm_base_assets();
    let mut stored_assets = profile_assets.clone();
    stored_assets.rootfs_hash =
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into();
    let entry = PersistentVmEntry {
        name: "source-drift".into(),
        ram_mb: 2048,
        cpus: 2,
        base_version: "0.0.0".into(),
        base_assets: Some(stored_assets),
        profile_pin: Some(test_saved_vm_profile_pin(profile_assets)),
        created_at: "0".into(),
        session_dir: PathBuf::from("/tmp/source-drift"),
        forked_from: None,
        description: None,
        suspended: false,
        defunct: false,
        last_error: None,
        checkpoint_path: None,
        env: None,
    };

    let err = source_vm_base_assets(&entry).unwrap_err();

    assert!(
        format!("{err:#}").contains("conflicting pinned asset identity"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn fork_profile_pin_match_rejects_profile_payload_hash_drift() {
    let base_assets = test_saved_vm_base_assets();
    let source_pin = test_saved_vm_profile_pin(base_assets.clone());
    let mut fork_pin = test_saved_vm_profile_pin(base_assets);
    fork_pin.profile_payload_hash = Some(format!("blake3:{}", "f".repeat(64)));

    let err = ensure_fork_profile_pin_matches_source(&fork_pin, &source_pin, "fork-src")
        .expect_err("payload hash drift must reject the fork");

    assert!(
        format!("{err:#}").contains("payload hash"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn handle_list_reports_missing_saved_vm_dependencies_separately() {
    let (state, _dir) = make_test_state_with_tempdir();
    std::fs::create_dir_all(&state.assets_dir).unwrap();
    std::fs::write(state.assets_dir.join("vmlinuz"), b"current kernel").unwrap();
    std::fs::write(state.assets_dir.join("initrd.img"), b"current initrd").unwrap();
    std::fs::write(state.assets_dir.join("rootfs.squashfs"), b"current rootfs").unwrap();
    std::fs::write(
        state.assets_dir.join("vmlinuz-aaaaaaaaaaaaaaaa"),
        b"old kernel",
    )
    .unwrap();
    std::fs::write(
        state.assets_dir.join("initrd-bbbbbbbbbbbbbbbb.img"),
        b"old initrd",
    )
    .unwrap();
    state.asset_supervisor.refresh_local_state();

    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry.data.vms.insert(
            "saved-old".into(),
            PersistentVmEntry {
                name: "saved-old".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: Some(test_saved_vm_base_assets()),
                profile_pin: None,
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/saved-old"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let Json(list) = handle_list(State(state)).await;
    let assets = list.asset_health.expect("asset health should be present");

    assert_eq!(assets.state, AssetHealthState::Ready);
    assert!(assets.ready);
    assert!(assets.missing.is_empty());
    assert_eq!(assets.saved_vm_dependencies.len(), 1);
    assert_eq!(assets.saved_vm_dependencies[0].vm, "saved-old");
    assert_eq!(
        assets.saved_vm_dependencies[0].missing,
        vec!["rootfs.squashfs"]
    );
}

#[tokio::test]
async fn handle_list_reports_profile_status_for_each_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    let catalog_path = state.service_settings.profiles.corp_dirs[0]
        .join(".catalog")
        .join("profile-manifest.json");
    std::fs::create_dir_all(catalog_path.parent().unwrap()).unwrap();
    std::fs::write(&catalog_path, profile_status_manifest_json()).unwrap();

    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry.data.vms.insert(
            "vm-current".into(),
            pinned_vm_entry(&state, "vm-current", "everyday-work", Some("2026.0520.2")),
        );
        registry.data.vms.insert(
            "vm-update".into(),
            pinned_vm_entry(&state, "vm-update", "everyday-work", Some("2026.0520.1")),
        );
        registry.data.vms.insert(
            "vm-deprecated".into(),
            pinned_vm_entry(&state, "vm-deprecated", "coding", Some("2026.0520.1")),
        );
        registry.data.vms.insert(
            "vm-revoked".into(),
            pinned_vm_entry(&state, "vm-revoked", "research", Some("2026.0520.1")),
        );
        registry.data.vms.insert(
            "vm-corrupted".into(),
            PersistentVmEntry {
                name: "vm-corrupted".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: None,
                profile_pin: None,
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/vm-corrupted"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let Json(list) = handle_list(State(state)).await;
    let by_id = list
        .sandboxes
        .iter()
        .map(|info| (info.id.as_str(), info))
        .collect::<std::collections::HashMap<_, _>>();

    let current = by_id["vm-current"];
    assert_eq!(current.profile_id.as_deref(), Some("everyday-work"));
    assert_eq!(current.profile_revision.as_deref(), Some("2026.0520.2"));
    assert_eq!(current.profile_status, Some(VmProfileStatus::Current));

    let update = by_id["vm-update"];
    assert_eq!(update.profile_id.as_deref(), Some("everyday-work"));
    assert_eq!(update.profile_revision.as_deref(), Some("2026.0520.1"));
    assert_eq!(update.profile_status, Some(VmProfileStatus::NeedsUpdate));

    assert_eq!(
        by_id["vm-deprecated"].profile_status,
        Some(VmProfileStatus::Deprecated)
    );
    assert_eq!(
        by_id["vm-revoked"].profile_status,
        Some(VmProfileStatus::Revoked)
    );
    assert_eq!(
        by_id["vm-corrupted"].profile_status,
        Some(VmProfileStatus::Corrupted)
    );
}

fn pinned_vm_entry(
    state: &ServiceState,
    name: &str,
    profile_id: &str,
    revision: Option<&str>,
) -> PersistentVmEntry {
    let base_assets = test_saved_vm_base_assets();
    PersistentVmEntry {
        name: name.into(),
        ram_mb: 2048,
        cpus: 2,
        base_version: "0.0.0".into(),
        base_assets: Some(base_assets.clone()),
        profile_pin: Some(SavedVmProfilePin {
            profile_id: profile_id.into(),
            profile_revision: revision.map(str::to_string),
            profile_payload_hash: Some(format!("blake3:{}", "e".repeat(64))),
            package_contract_hash: format!("blake3:{}", "d".repeat(64)),
            base_assets: Some(base_assets),
        }),
        created_at: "0".into(),
        session_dir: state.run_dir.join("persistent").join(name),
        forked_from: None,
        description: None,
        suspended: false,
        defunct: false,
        last_error: None,
        checkpoint_path: None,
        env: None,
    }
}

fn profile_status_manifest_json() -> &'static str {
    r#"{
      "format": 1,
      "profiles": {
        "everyday-work": {
          "current_revision": "2026.0520.2",
          "revisions": {
            "2026.0520.1": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///tmp/everyday-work-1/profile.json",
              "profile_hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "profile_signature_url": "file:///tmp/everyday-work-1/profile.json.minisig"
            },
            "2026.0520.2": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///tmp/everyday-work-2/profile.json",
              "profile_hash": "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "profile_signature_url": "file:///tmp/everyday-work-2/profile.json.minisig"
            }
          }
        },
        "coding": {
          "current_revision": "2026.0520.2",
          "revisions": {
            "2026.0520.1": {
              "status": "deprecated",
              "min_binary": "1.0.0",
              "profile_url": "file:///tmp/coding-1/profile.json",
              "profile_hash": "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
              "profile_signature_url": "file:///tmp/coding-1/profile.json.minisig"
            },
            "2026.0520.2": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///tmp/coding-2/profile.json",
              "profile_hash": "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
              "profile_signature_url": "file:///tmp/coding-2/profile.json.minisig"
            }
          }
        },
        "research": {
          "current_revision": "2026.0520.2",
          "revisions": {
            "2026.0520.1": {
              "status": "revoked",
              "min_binary": "1.0.0",
              "profile_url": "file:///tmp/research-1/profile.json",
              "profile_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
              "profile_signature_url": "file:///tmp/research-1/profile.json.minisig"
            },
            "2026.0520.2": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///tmp/research-2/profile.json",
              "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
              "profile_signature_url": "file:///tmp/research-2/profile.json.minisig"
            }
          }
        }
      }
    }"#
}

#[test]
fn resume_saved_vm_fails_when_pinned_rootfs_is_missing() {
    let (state, _dir) = make_test_state_with_tempdir();
    std::fs::create_dir_all(&state.assets_dir).unwrap();
    std::fs::write(
        state.assets_dir.join("vmlinuz-aaaaaaaaaaaaaaaa"),
        b"old kernel",
    )
    .unwrap();
    std::fs::write(
        state.assets_dir.join("initrd-bbbbbbbbbbbbbbbb.img"),
        b"old initrd",
    )
    .unwrap();
    let session_dir = state.run_dir.join("persistent/saved-old");
    std::fs::create_dir_all(&session_dir).unwrap();
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        let base_assets = test_saved_vm_base_assets();
        registry.data.vms.insert(
            "saved-old".into(),
            PersistentVmEntry {
                name: "saved-old".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: Some(base_assets.clone()),
                profile_pin: Some(SavedVmProfilePin {
                    profile_id: "everyday-work".into(),
                    profile_revision: Some("2026.0520.1".into()),
                    profile_payload_hash: Some(format!("blake3:{}", "e".repeat(64))),
                    package_contract_hash: format!("blake3:{}", "d".repeat(64)),
                    base_assets: Some(base_assets),
                }),
                created_at: "0".into(),
                session_dir,
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let err = state.resume_sandbox("saved-old", None, None).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("saved VM saved-old"), "{msg}");
    assert!(msg.contains("rootfs.squashfs"), "{msg}");
}

#[test]
fn resume_saved_vm_requires_forward_profile_pin() {
    let (state, _dir) = make_test_state_with_tempdir();
    std::fs::create_dir_all(&state.assets_dir).unwrap();
    std::fs::write(state.assets_dir.join("vmlinuz"), b"current kernel").unwrap();
    std::fs::write(state.assets_dir.join("initrd.img"), b"current initrd").unwrap();
    std::fs::write(state.assets_dir.join("rootfs.squashfs"), b"current rootfs").unwrap();
    state.asset_supervisor.refresh_local_state();
    let session_dir = state.run_dir.join("persistent/unpinned");
    std::fs::create_dir_all(&session_dir).unwrap();
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry.data.vms.insert(
            "unpinned".into(),
            PersistentVmEntry {
                name: "unpinned".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: None,
                profile_pin: None,
                created_at: "0".into(),
                session_dir,
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let err = state.resume_sandbox("unpinned", None, None).unwrap_err();

    assert!(
        err.to_string().contains("missing required profile pin"),
        "unexpected error: {err:#}"
    );
}

fn insert_fake_instance(state: &ServiceState, id: &str, pid: u32) {
    state.instances.lock().unwrap().insert(
        id.to_string(),
        InstanceInfo {
            id: id.to_string(),
            pid,
            uds_path: PathBuf::from(format!("/tmp/{}.sock", id)),
            session_dir: PathBuf::from(format!("/tmp/sessions/{}", id)),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: None,
            profile_pin: None,
        },
    );
}

// -----------------------------------------------------------------------
// next_job_id
// -----------------------------------------------------------------------

#[test]
fn next_job_id_starts_at_1() {
    let state = make_test_state();
    assert_eq!(state.next_job_id(), 1);
}

#[test]
fn next_job_id_increments() {
    let state = make_test_state();
    let a = state.next_job_id();
    let b = state.next_job_id();
    let c = state.next_job_id();
    assert_eq!(b, a + 1);
    assert_eq!(c, a + 2);
}

#[test]
fn next_job_id_unique_across_many() {
    let state = make_test_state();
    let ids: Vec<u64> = (0..1000).map(|_| state.next_job_id()).collect();
    let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), 1000);
}

// -----------------------------------------------------------------------
// Instance map CRUD
// -----------------------------------------------------------------------

#[test]
fn instance_insert_and_lookup() {
    let state = make_test_state();
    insert_fake_instance(&state, "test-vm", std::process::id());
    let instances = state.instances.lock().unwrap();
    assert!(instances.contains_key("test-vm"));
    assert_eq!(instances["test-vm"].ram_mb, 2048);
}

#[test]
fn instance_remove() {
    let state = make_test_state();
    insert_fake_instance(&state, "test-vm", std::process::id());
    state.instances.lock().unwrap().remove("test-vm");
    assert!(!state.instances.lock().unwrap().contains_key("test-vm"));
}

#[test]
fn instance_lookup_missing() {
    let state = make_test_state();
    assert!(!state.instances.lock().unwrap().contains_key("no-such-vm"));
}

#[test]
fn instance_count() {
    let state = make_test_state();
    insert_fake_instance(&state, "vm-1", std::process::id());
    insert_fake_instance(&state, "vm-2", std::process::id());
    insert_fake_instance(&state, "vm-3", std::process::id());
    assert_eq!(state.instances.lock().unwrap().len(), 3);
}

// -----------------------------------------------------------------------
// cleanup_stale_instances
// -----------------------------------------------------------------------

#[test]
fn cleanup_removes_dead_pid() {
    let state = make_test_state();
    // PID 99999999 should not exist
    insert_fake_instance(&state, "dead-vm", 99999999);
    assert_eq!(state.instances.lock().unwrap().len(), 1);
    state.cleanup_stale_instances();
    assert_eq!(state.instances.lock().unwrap().len(), 0);
}

#[test]
fn cleanup_keeps_live_pid() {
    let state = make_test_state();
    // Current process PID should be alive
    insert_fake_instance(&state, "live-vm", std::process::id());
    state.cleanup_stale_instances();
    assert_eq!(state.instances.lock().unwrap().len(), 1);
}

#[test]
fn cleanup_mixed_live_and_dead() {
    let state = make_test_state();
    insert_fake_instance(&state, "live", std::process::id());
    insert_fake_instance(&state, "dead", 99999999);
    state.cleanup_stale_instances();
    let instances = state.instances.lock().unwrap();
    assert_eq!(instances.len(), 1);
    assert!(instances.contains_key("live"));
}

#[tokio::test]
async fn reload_config_returns_structured_failed_session_state() {
    let (state, dir) = make_test_state_with_tempdir();
    let sock_path = dir.path().join("process.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();

    let server = std::thread::spawn(move || {
        let (mut std_stream, _) = listener.accept().unwrap();
        capsem_core::ipc_handshake::negotiate_responder(&mut std_stream, "capsem-process-test", "")
            .unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let (tx, rx): (Sender<ProcessToService>, Receiver<ServiceToProcess>) =
                    channel_from_std(std_stream).unwrap();
                match rx.recv().await.unwrap() {
                    ServiceToProcess::ReloadConfig => {
                        tx.send(ProcessToService::ReloadConfigResult {
                            success: false,
                            error: Some("reload exploded".into()),
                        })
                        .await
                        .unwrap();
                    }
                    other => panic!("unexpected command: {other:?}"),
                }
            });
    });

    state.instances.lock().unwrap().insert(
        "vm-reload".to_string(),
        InstanceInfo {
            id: "vm-reload".to_string(),
            pid: std::process::id(),
            uds_path: sock_path,
            session_dir: dir.path().join("sessions/vm-reload"),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: None,
            profile_pin: None,
        },
    );

    let (status, Json(body)) = handle_reload_config(State(state)).await.unwrap();

    server.join().unwrap();
    assert_eq!(status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["success"], false);
    assert_eq!(body["reloaded"], 0);
    assert_eq!(body["failed_session_count"], 1);
    assert_eq!(body["failed_session_ids"], serde_json::json!(["vm-reload"]));
    assert_eq!(body["failures"][0]["message"], "reload exploded");
}

// -----------------------------------------------------------------------
// drain_dead_instances: probe-and-evict contract, filesystem work is the
// caller's responsibility. Exists so `cleanup_stale_instances` can release
// the instances mutex BEFORE performing remove_dir_all -- otherwise every
// handler that touches instances.lock() blocks on slow fs I/O.
// -----------------------------------------------------------------------

#[test]
fn drain_dead_instances_returns_only_dead_entries() {
    let state = make_test_state();
    insert_fake_instance(&state, "live", std::process::id());
    insert_fake_instance(&state, "dead", 99999999);

    let evicted = state.drain_dead_instances();

    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].0, "dead");
    let map = state.instances.lock().unwrap();
    assert!(map.contains_key("live"));
    assert!(!map.contains_key("dead"));
}

#[test]
fn drain_dead_instances_empty_when_all_alive() {
    let state = make_test_state();
    insert_fake_instance(&state, "live-1", std::process::id());
    insert_fake_instance(&state, "live-2", std::process::id());

    let evicted = state.drain_dead_instances();

    assert!(evicted.is_empty());
    assert_eq!(state.instances.lock().unwrap().len(), 2);
}

#[test]
fn drain_dead_instances_releases_mutex_before_returning() {
    // Regression guard: the whole point of splitting drain from the
    // filesystem scrub is that the mutex must be FREE by the time
    // drain returns. If this test ever fails, the locking protocol
    // has regressed and concurrent handlers will block on cleanup I/O.
    let state = make_test_state();
    insert_fake_instance(&state, "dead", 99999999);

    let _evicted = state.drain_dead_instances();

    assert!(
        state.instances.try_lock().is_ok(),
        "mutex still held after drain_dead_instances returned"
    );
}

// -----------------------------------------------------------------------
// preserve_failed_session_dir + cull_failed_sessions
//
// The post-mortem pipeline: when any of the three loss paths
// (wait_for_vm_ready timeout, dead-process cleanup, unexpected
// child exit) would have silently `remove_dir_all`'d a session dir,
// it's renamed to a `-failed-*` sibling instead so process.log,
// mcp-aggregator.stderr.log, serial.log, and session.db survive.
// Cap: MAX_FAILED_SESSIONS (5).
// -----------------------------------------------------------------------

fn make_state_in(run_dir: PathBuf) -> Arc<ServiceState> {
    let registry_path = run_dir.join("persistent_registry.json");
    std::fs::create_dir_all(run_dir.join("sessions")).unwrap();
    let assets_dir = PathBuf::from("/nonexistent/assets");
    let current_version = "0.0.0";
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: assets_dir.clone(),
        asset_locations: test_asset_locations(assets_dir.clone()),
        service_settings: test_service_settings(&run_dir),
        run_dir,
        job_counter: AtomicU64::new(1),
        asset_supervisor: test_asset_supervisor(assets_dir),
        current_version: current_version.into(),
        magika: test_magika(),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    })
}

#[test]
fn preserve_renames_session_dir_and_keeps_logs() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("vm-abc");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"boot failed: ...").unwrap();
    std::fs::write(session_dir.join("serial.log"), b"kernel panic").unwrap();

    state.preserve_failed_session_dir(&session_dir, "vm-abc");

    assert!(
        !session_dir.exists(),
        "original dir should have been renamed"
    );
    let entries: Vec<_> = std::fs::read_dir(state.run_dir.join("sessions"))
        .unwrap()
        .flatten()
        .collect();
    let failed = entries
        .iter()
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("vm-abc-failed-")
        })
        .expect("a vm-abc-failed-* dir must exist");
    let preserved = failed.path().join("process.log");
    assert_eq!(std::fs::read(&preserved).unwrap(), b"boot failed: ...");
    let preserved_serial = failed.path().join("serial.log");
    assert_eq!(std::fs::read(&preserved_serial).unwrap(), b"kernel panic");
}

// AB-008: idempotency on the failure-preservation path.
//
// Multiple cleanup paths can race for the same session dir
// (`scrub_dead_process`, the spawn-completion handler, `handle_run` cleanup).
// The previous implementation emitted two scary WARN lines on the second
// call ("logs lost" + "orphaned on disk") even when the first call had
// preserved the dir successfully. The outcome enum lets us assert the
// idempotent shape without capturing tracing output.

#[test]
fn preserve_outcome_preserved_when_dir_exists() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("vm-x");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"x").unwrap();

    let outcome = state.preserve_failed_session_dir_outcome(&session_dir, "vm-x");
    let preserved_path = match outcome {
        PreserveOutcome::Preserved(p) => p,
        other => panic!("expected Preserved, got {other:?}"),
    };
    assert!(preserved_path.exists(), "rename target must exist");
    assert!(!session_dir.exists(), "original must be gone after rename");
    assert!(
        preserved_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("vm-x-failed-")),
        "preserved name must follow `<id>-failed-*` shape: {}",
        preserved_path.display()
    );
}

#[test]
fn preserve_outcome_already_absent_when_dir_does_not_exist() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("vm-gone");
    // Note: we never create session_dir.

    let outcome = state.preserve_failed_session_dir_outcome(&session_dir, "vm-gone");
    assert!(
        matches!(outcome, PreserveOutcome::AlreadyAbsent),
        "expected AlreadyAbsent, got {outcome:?}"
    );
    let entries: Vec<String> = std::fs::read_dir(state.run_dir.join("sessions"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !entries.iter().any(|n| n.contains("-failed-")),
        "must not create a -failed- dir for an absent source: {entries:?}"
    );
}

#[test]
fn preserve_is_idempotent_when_called_twice() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("vm-twice");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"first").unwrap();

    let first = state.preserve_failed_session_dir_outcome(&session_dir, "vm-twice");
    assert!(
        matches!(first, PreserveOutcome::Preserved(_)),
        "first call must preserve, got {first:?}"
    );

    let failed_count_after_first: usize = std::fs::read_dir(state.run_dir.join("sessions"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("-failed-"))
        .count();
    assert_eq!(failed_count_after_first, 1);

    // Second call on the same -- now-absent -- session_dir must be a quiet
    // idempotent no-op, NOT a duplicate -failed- creation, NOT an
    // orphaned-on-disk warning.
    let second = state.preserve_failed_session_dir_outcome(&session_dir, "vm-twice");
    assert!(
        matches!(second, PreserveOutcome::AlreadyAbsent),
        "second call must be idempotent, got {second:?}"
    );

    let failed_count_after_second: usize = std::fs::read_dir(state.run_dir.join("sessions"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("-failed-"))
        .count();
    assert_eq!(
        failed_count_after_second, 1,
        "second call must not create a new -failed- sibling"
    );
}

#[test]
fn cull_keeps_newest_and_prunes_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    // Create MAX_FAILED_SESSIONS + 2 failed dirs with staggered mtimes.
    // Using filetime to set mtime lets us assert deterministically
    // which ones get pruned (oldest) vs kept (newest).
    let total = MAX_FAILED_SESSIONS + 2;
    for i in 0..total {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        let p = sessions.join(&name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("process.log"), format!("run {i}")).unwrap();
        // Older i -> older mtime.
        let when = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(1_700_000_000 + i as u64 * 10);
        filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(when)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    let remaining: std::collections::HashSet<String> = std::fs::read_dir(&sessions)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        remaining.len(),
        MAX_FAILED_SESSIONS,
        "should keep exactly MAX_FAILED_SESSIONS, got {remaining:?}"
    );
    // Oldest two (i=0, i=1) must be pruned; newest MAX_FAILED_SESSIONS kept.
    for i in 0..2 {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        assert!(
            !remaining.contains(&name),
            "oldest dir {name} should have been culled"
        );
    }
    for i in 2..total {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        assert!(
            remaining.contains(&name),
            "newer dir {name} should have been kept"
        );
    }
}

#[test]
fn cull_is_noop_when_under_cap() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    for i in 0..3 {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        std::fs::create_dir_all(sessions.join(&name)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    assert_eq!(std::fs::read_dir(&sessions).unwrap().count(), 3);
}

#[test]
fn cull_ignores_non_failed_dirs() {
    // Running sessions (no `-failed-` in the name) must never be
    // culled. This is the safety property: a misnamed cull is a
    // production outage.
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    std::fs::create_dir_all(sessions.join("vm-alive")).unwrap();
    for i in 0..(MAX_FAILED_SESSIONS + 3) {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        std::fs::create_dir_all(sessions.join(&name)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    assert!(
        sessions.join("vm-alive").exists(),
        "active VM dir must not be culled"
    );
}

// -----------------------------------------------------------------------
// Auto-ID generation format
// -----------------------------------------------------------------------

#[test]
fn auto_id_format() {
    // Verify the auto-ID pattern used in handle_provision
    let id = format!(
        "vm-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    assert!(id.starts_with("vm-"));
    // Should be "vm-" followed by digits
    let suffix = &id[3..];
    assert!(suffix.chars().all(|c| c.is_ascii_digit()));
}

// -----------------------------------------------------------------------
// Input validation edge cases (DTO level)
// -----------------------------------------------------------------------

#[test]
fn provision_request_no_name() {
    let json = serde_json::json!({"ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert!(req.name.is_none());
}

#[test]
fn provision_request_empty_name() {
    let json = serde_json::json!({"name": "", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.name.unwrap(), "");
}

#[test]
fn provision_request_name_with_path_separator() {
    // This is a security edge case -- names with / could create path traversal
    let json = serde_json::json!({"name": "../escape", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.name.unwrap(), "../escape");
    // Note: the service SHOULD reject this, but currently doesn't validate
}

#[test]
fn exec_request_empty_command() {
    let json = serde_json::json!({"command": ""});
    let req: ExecRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.command, "");
}

#[test]
fn exec_request_shell_metacharacters() {
    let json = serde_json::json!({"command": "echo $(whoami) && rm -rf /"});
    let req: ExecRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.command, "echo $(whoami) && rm -rf /");
}

#[test]
fn inspect_request_sql_injection() {
    let json = serde_json::json!({"sql": "SELECT * FROM net_events; DROP TABLE net_events; --"});
    let req: InspectRequest = serde_json::from_value(json).unwrap();
    assert!(req.sql.contains("DROP TABLE"));
    // Note: backend should use read-only DB connection to prevent writes
}

// -----------------------------------------------------------------------
// Asset path resolution
// -----------------------------------------------------------------------

#[test]
fn asset_version_path_construction() {
    let base = PathBuf::from("/home/user/.capsem/assets");
    let version = "0.16.1";
    let v_path = base.join(format!("v{}", version));
    assert_eq!(v_path, PathBuf::from("/home/user/.capsem/assets/v0.16.1"));
}

#[test]
fn arch_detection_aarch64() {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    assert!(arch == "arm64" || arch == "x86_64");
}

// -----------------------------------------------------------------------
// UDS path length validation (macOS 104, Linux 108 including null)
// -----------------------------------------------------------------------

#[test]
fn long_vm_name_falls_back_to_tmp_socket() {
    let state = make_test_state();
    // A 100-char name exceeds SUN_PATH_MAX via run_dir/instances/ path,
    // but instance_socket_path should fall back to /tmp/capsem/.
    let long_name = "a".repeat(100);
    let path = state.instance_socket_path(&long_name);
    assert!(
        path.starts_with("/tmp/capsem/"),
        "expected /tmp/capsem/ fallback, got: {}",
        path.display()
    );
    assert!(
        path.as_os_str().len() < 104,
        "fallback path still too long: {}",
        path.as_os_str().len()
    );
}

#[test]
fn short_vm_name_uses_run_dir() {
    let state = make_test_state();
    let path = state.instance_socket_path("test-vm");
    assert_eq!(path, state.run_dir.join("instances/test-vm.sock"));
}

#[test]
fn provision_accepts_name_just_under_uds_limit() {
    let state = make_test_state();
    let prefix = state.run_dir.join("instances").join("").as_os_str().len();
    let suffix_len = ".sock".len();
    let sun_path_max: usize = if cfg!(target_os = "macos") { 104 } else { 108 };
    // One byte shorter than the limit -- should pass path validation
    let name_len = sun_path_max - prefix - suffix_len - 1;
    let ok_name = "x".repeat(name_len);
    let result = state.provision_sandbox(ProvisionOptions {
        id: &ok_name,
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: false,
        env: None,
        from: None,
        profile_id: None,
        profile_revision: None,
        description: None,
    });
    // Will fail later (missing rootfs), but NOT for path length
    if let Err(e) = &result {
        let msg = e.to_string();
        assert!(
            !msg.contains("socket path"),
            "short name should not hit path limit: {msg}"
        );
    }
}

#[test]
fn provision_short_name_passes_path_check() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "my-vm",
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: false,
        env: None,
        from: None,
        profile_id: None,
        profile_revision: None,
        description: None,
    });
    // Fails for missing assets, not path length
    if let Err(e) = &result {
        let msg = e.to_string();
        assert!(
            !msg.contains("socket path"),
            "normal name should not hit path limit: {msg}"
        );
    }
}

// -----------------------------------------------------------------------
// Provision rejects duplicate persistent VM
// -----------------------------------------------------------------------

#[test]
fn provision_persistent_rejects_duplicate_name() {
    let state = make_test_state();
    // Pre-register a persistent VM directly in the registry data
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "taken".into(),
            PersistentVmEntry {
                name: "taken".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: None,
                profile_pin: None,
                created_at: "0".into(),
                session_dir: PathBuf::from("/tmp/taken"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }
    let result = state.provision_sandbox(ProvisionOptions {
        id: "taken",
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: true,
        env: None,
        from: None,
        profile_id: None,
        profile_revision: None,
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("already exists"),
        "expected duplicate error, got: {err}"
    );
    assert!(err.contains("resume"), "should suggest resume, got: {err}");
}

#[test]
fn provision_persistent_validates_name() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "../evil",
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: true,
        env: None,
        from: None,
        profile_id: None,
        profile_revision: None,
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("must start with") || err.contains("must contain only"),
        "expected name validation error, got: {err}"
    );
}

#[test]
fn provision_from_source_requires_profile_revision_pin() {
    let (state, _dir) = make_test_state_with_tempdir();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        let base_assets = test_saved_vm_base_assets();
        let mut profile_pin = test_saved_vm_profile_pin(base_assets.clone());
        profile_pin.profile_revision = None;
        reg.data.vms.insert(
            "old-source".into(),
            PersistentVmEntry {
                name: "old-source".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: Some(base_assets),
                profile_pin: Some(profile_pin),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/old-source"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let err = state
        .provision_sandbox(ProvisionOptions {
            id: "clone",
            ram_mb: 2048,
            cpus: 2,
            version_override: None,
            persistent: false,
            env: None,
            from: Some("old-source".into()),
            profile_id: None,
            profile_revision: None,
            description: None,
        })
        .unwrap_err();

    assert!(
        format!("{err:#}").contains("required profile revision pin"),
        "unexpected error: {err:#}"
    );
}

// -----------------------------------------------------------------------
// Image handler tests (service-level unit tests)
// -----------------------------------------------------------------------

fn make_test_state_with_tempdir() -> (Arc<ServiceState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let registry_path = dir.path().join("persistent_registry.json");
    let assets_dir = dir.path().join("assets");
    let current_version = "0.0.0";
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: assets_dir.clone(),
        asset_locations: test_asset_locations(assets_dir.clone()),
        service_settings: test_service_settings(dir.path()),
        run_dir: dir.path().to_path_buf(),
        job_counter: AtomicU64::new(1),
        asset_supervisor: test_asset_supervisor(assets_dir),
        current_version: current_version.into(),
        magika: test_magika(),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });
    (state, dir)
}

fn make_test_state_with_profile_assets(base_url: &str) -> (Arc<ServiceState>, tempfile::TempDir) {
    make_test_state_with_profile_assets_and_process(
        base_url,
        PathBuf::from("/nonexistent/capsem-process"),
    )
}

fn make_test_state_with_profile_assets_and_process(
    base_url: &str,
    process_binary: PathBuf,
) -> (Arc<ServiceState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let registry_path = dir.path().join("persistent_registry.json");
    let assets_dir = dir.path().join("assets");
    let current_version = "0.0.0";
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary,
        assets_dir: assets_dir.clone(),
        asset_locations: test_asset_locations(assets_dir.clone()),
        service_settings: test_service_settings(dir.path()),
        run_dir: dir.path().to_path_buf(),
        job_counter: AtomicU64::new(1),
        asset_supervisor: test_profile_asset_supervisor(assets_dir, base_url),
        current_version: current_version.into(),
        magika: test_magika(),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });
    (state, dir)
}

fn write_profile_test_assets(assets_dir: &std::path::Path) {
    let arch_dir = assets_dir.join(host_asset_arch());
    std::fs::create_dir_all(&arch_dir).unwrap();
    for (logical_name, bytes) in [
        ("vmlinuz", b"kernel".as_slice()),
        ("initrd.img", b"initrd".as_slice()),
        ("rootfs.squashfs", b"rootfs".as_slice()),
    ] {
        let hash = blake3::hash(bytes).to_hex().to_string();
        std::fs::write(
            arch_dir.join(capsem_core::asset_manager::hash_filename(
                logical_name,
                &hash,
            )),
            bytes,
        )
        .unwrap();
    }
}

#[tokio::test]
async fn handle_asset_reconcile_downloads_missing_profile_assets() {
    let (base_url, server) = start_test_asset_server().await;
    let (state, _dir) = make_test_state_with_profile_assets(&base_url);

    let Json(result) = handle_asset_reconcile(State(state.clone())).await.unwrap();

    server.abort();
    assert_eq!(result["mode"], serde_json::json!("settings_profiles_v2"));
    assert_eq!(result["outcome"], serde_json::json!("downloaded"));
    assert_eq!(result["health"]["state"], serde_json::json!("ready"));
    assert_eq!(result["health"]["ready"], serde_json::json!(true));
    assert_eq!(
        result["health"]["profile_id"],
        serde_json::json!("everyday-work")
    );
    assert_eq!(
        result["health"]["profile_revision"],
        serde_json::json!("2026.0520.1")
    );
    assert_eq!(
        result["health"]["profile_payload_hash"],
        serde_json::json!(test_profile_payload_hash())
    );
    assert_eq!(
        result["health"]["profile_assets"][0]["logical_name"],
        serde_json::json!("vmlinuz")
    );
    assert!(!result["health"]["profile_assets"][0]["source_url"]
        .as_str()
        .unwrap()
        .contains('?'));
    assert!(state.asset_supervisor.snapshot().ready);
}

#[test]
fn profile_asset_operator_flow_chains_reconcile_status_debug_and_logs() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (base_url, server) = runtime.block_on(start_test_asset_server());
    let (state, dir) = make_test_state_with_profile_assets(&base_url);
    // The process-wide test subscriber below keeps writing after this test's
    // assertions when parallel service tests emit tracing events.
    let _dir = Box::leak(Box::new(dir));
    let log_path = state.run_dir.join("service.log");
    std::fs::create_dir_all(&state.run_dir).unwrap();
    let log_writer_path = log_path.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_writer_path)
                .unwrap()
        })
        .finish();
    let dispatch = tracing::Dispatch::new(subscriber);
    let _ = tracing::dispatcher::set_global_default(dispatch.clone());

    tracing::dispatcher::with_default(&dispatch, || {
        runtime.block_on(async {
            let Json(reconcile) = handle_asset_reconcile(State(state.clone())).await.unwrap();
            assert_eq!(reconcile["outcome"], serde_json::json!("downloaded"));
            assert_eq!(reconcile["health"]["state"], serde_json::json!("ready"));

            let Json(setup_status) = handle_asset_status(State(state.clone())).await;
            assert_eq!(setup_status["ready"], serde_json::json!(true));
            assert_eq!(
                setup_status["profile_payload_hash"],
                serde_json::json!(test_profile_payload_hash())
            );
            assert_eq!(
                setup_status["profile_assets"][0]["source_url"],
                serde_json::json!("http://127.0.0.1/vmlinuz")
            );

            let Json(list) = handle_list(State(state.clone())).await;
            let list_health = list.asset_health.expect("list should include asset health");
            assert!(list_health.ready);
            assert_eq!(
                list_health.profile_payload_hash.as_deref(),
                Some(test_profile_payload_hash().as_str())
            );
            assert_eq!(list_health.profile_assets.len(), 3);

            let Json(debug) = handle_debug_report(State(state.clone())).await.unwrap();
            assert!(debug
                .text
                .contains("profile_asset_profile_payload_hash: blake3:"));
            assert!(debug.text.contains("profile_asset_source: vmlinuz"));

            let expected_events = [
                "profile_asset_check_start",
                "profile_asset_check_finish",
            ];
            let mut service_logs = String::new();
            for _ in 0..50 {
                service_logs = handle_service_logs(State(state.clone())).await.unwrap();
                if expected_events
                    .iter()
                    .all(|event| service_logs.contains(event))
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            server.abort();

            for event in expected_events {
                assert!(
                    service_logs.contains(event),
                    "service logs should include {event}; logs were:\n{service_logs}"
                );
            }
            assert!(
                service_logs.contains("profile_asset_download_start")
                    || service_logs.contains("profile_asset_download_progress")
                    || service_logs.contains("profile_asset_verify_ok"),
                "service logs should include a profile asset download event; logs were:\n{service_logs}"
            );
        });
    });
}

#[tokio::test]
async fn handle_asset_reconcile_reports_already_ready() {
    let (state, _dir) = make_test_state_with_profile_assets("https://assets.example.test");
    write_profile_test_assets(&state.assets_dir);
    state.asset_supervisor.refresh_local_state();

    let Json(result) = handle_asset_reconcile(State(state)).await.unwrap();

    assert_eq!(result["outcome"], serde_json::json!("already_ready"));
    assert_eq!(result["health"]["state"], serde_json::json!("ready"));
}

#[tokio::test]
async fn handle_asset_reconcile_concurrent_calls_share_one_download_run() {
    let (base_url, server, request_count, first_request_seen, release_first_response) =
        start_counted_blocking_asset_server().await;
    let (state, _dir) = make_test_state_with_profile_assets(&base_url);

    let first = tokio::spawn(handle_asset_reconcile(State(state.clone())));
    let second = tokio::spawn(handle_asset_reconcile(State(state.clone())));

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        first_request_seen.notified(),
    )
    .await
    .expect("first download request should start");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "second reconcile must wait on the supervisor run lock instead of starting a duplicate GET"
    );

    release_first_response.notify_waiters();
    let first = first.await.unwrap().unwrap().0;
    let second = second.await.unwrap().unwrap().0;
    server.abort();

    assert_eq!(first["health"]["state"], serde_json::json!("ready"));
    assert_eq!(second["health"]["state"], serde_json::json!("ready"));
    assert!(state.asset_supervisor.snapshot().ready);
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        3,
        "exactly one GET per required profile asset should be issued"
    );
}

#[tokio::test]
async fn handle_asset_cleanup_refuses_during_active_profile_download() {
    let (base_url, server, _request_count, first_request_seen, release_first_response) =
        start_counted_blocking_asset_server().await;
    let (state, _dir) = make_test_state_with_profile_assets(&base_url);
    let stale = state.assets_dir.join("rootfs-9999999999999999.squashfs");
    std::fs::create_dir_all(&state.assets_dir).unwrap();
    std::fs::write(&stale, b"stale rootfs").unwrap();

    let reconcile = tokio::spawn(handle_asset_reconcile(State(state.clone())));
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        first_request_seen.notified(),
    )
    .await
    .expect("download should be in progress before cleanup");

    let err = handle_asset_cleanup(State(state.clone()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err
        .1
        .contains("asset cleanup is blocked while assets are updating"));
    assert!(stale.exists());

    release_first_response.notify_waiters();
    let result = reconcile.await.unwrap().unwrap().0;
    server.abort();
    assert_eq!(result["health"]["state"], serde_json::json!("ready"));
}

#[tokio::test]
async fn provision_attempt_reconciles_profile_assets_on_first_use_create() {
    let (base_url, server) = start_test_asset_server().await;
    let (state, _dir) =
        make_test_state_with_profile_assets_and_process(&base_url, PathBuf::from("/bin/false"));

    assert!(!state.asset_supervisor.snapshot().ready);

    let outcome = provision_attempt(
        &state,
        "first-use-create",
        2048,
        2,
        false,
        None,
        None,
        None,
        None,
    )
    .await;

    server.abort();
    match outcome {
        ProvisionAttemptOutcome::BootCrash { .. } | ProvisionAttemptOutcome::ProvisionError(_) => {}
        other => panic!("expected spawn failure after asset reconcile, got {other:?}"),
    }
    let health = state.asset_supervisor.snapshot();
    assert!(health.ready);
    assert_eq!(health.profile_id.as_deref(), Some("everyday-work"));
    assert_eq!(health.profile_revision.as_deref(), Some("2026.0520.1"));
    let resolved = state.resolve_asset_paths().unwrap();
    assert!(resolved.kernel.exists());
    assert!(resolved.initrd.exists());
    assert!(resolved.rootfs.exists());
}

#[tokio::test]
async fn provision_attempt_reconciles_selected_profile_assets_and_attachment() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let false_binary = ["/bin/false", "/usr/bin/false"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("/bin/false"));
    let (state, dir) = make_test_state_with_profile_assets_and_process(
        "https://assets.example.test",
        false_binary,
    );
    let _env_guard = SettingsEnvGuard {
        previous_capsem_home: std::env::var_os("CAPSEM_HOME"),
    };
    std::env::set_var("CAPSEM_HOME", &state.run_dir);
    capsem_core::settings_profiles::write_service_settings(
        state.run_dir.join("service.toml"),
        &state.service_settings,
    )
    .unwrap();
    let corp_dir = dir.path().join("profiles/corp");
    let source_dir = dir.path().join("selected-profile-assets");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(source_dir.join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(source_dir.join("initrd.img"), b"initrd").unwrap();
    std::fs::write(source_dir.join("rootfs.squashfs"), b"rootfs").unwrap();
    let revision_dir = corp_dir.join(".catalog/profiles/coding/2026.0520.1");
    std::fs::create_dir_all(&revision_dir).unwrap();
    let arch = host_asset_arch();
    std::fs::write(
        corp_dir.join("coding.toml"),
        format!(
            r#"
version = 1
id = "coding"
name = "Coding"
best_for = "Development sessions."
profile_type = "coding"

[vm.assets.{arch}.kernel]
url = "file://{}"
hash = "blake3:{}"
signature_url = "file://{}/vmlinuz.minisig"
size = 6
content_type = "application/octet-stream"

[vm.assets.{arch}.initrd]
url = "file://{}"
hash = "blake3:{}"
signature_url = "file://{}/initrd.img.minisig"
size = 6
content_type = "application/octet-stream"

[vm.assets.{arch}.rootfs]
url = "file://{}"
hash = "blake3:{}"
signature_url = "file://{}/rootfs.squashfs.minisig"
size = 6
content_type = "application/octet-stream"
"#,
            source_dir.join("vmlinuz").display(),
            blake3::hash(b"kernel").to_hex(),
            source_dir.display(),
            source_dir.join("initrd.img").display(),
            blake3::hash(b"initrd").to_hex(),
            source_dir.display(),
            source_dir.join("rootfs.squashfs").display(),
            blake3::hash(b"rootfs").to_hex(),
            source_dir.display(),
        ),
    )
    .unwrap();
    let payload = br#"{"id":"coding"}"#;
    std::fs::write(revision_dir.join("profile.json"), payload).unwrap();
    let payload_hash = format!("blake3:{}", blake3::hash(payload).to_hex());
    std::fs::write(
        corp_dir.join(".catalog/profiles/coding/current.json"),
        format!(
            r#"{{
          "profile_id": "coding",
          "revision": "2026.0520.1",
          "payload_hash": "{payload_hash}"
        }}"#,
        ),
    )
    .unwrap();

    let outcome = provision_attempt(
        &state,
        "selected-profile-create",
        2048,
        2,
        false,
        None,
        None,
        Some("coding".to_string()),
        Some("2026.0520.1".to_string()),
    )
    .await;

    match outcome {
        ProvisionAttemptOutcome::BootCrash { .. } => {}
        ProvisionAttemptOutcome::ProvisionError(error) => {
            panic!("selected profile create should reach process spawn, got: {error:#}");
        }
        other => panic!("expected spawn failure after selected asset reconcile, got {other:?}"),
    }
    for (logical_name, bytes) in [
        ("vmlinuz", b"kernel".as_slice()),
        ("initrd.img", b"initrd".as_slice()),
        ("rootfs.squashfs", b"rootfs".as_slice()),
    ] {
        let hash = blake3::hash(bytes).to_hex().to_string();
        assert!(state
            .assets_dir
            .join(arch)
            .join(capsem_core::asset_manager::hash_filename(
                logical_name,
                &hash
            ))
            .exists());
    }
    let failed_dir = find_failed_session_dir(&state.run_dir, "selected-profile-create")
        .expect("failed selected-create session should be preserved");
    let effective = capsem_core::settings_profiles::load_vm_effective_settings(&failed_dir)
        .expect("selected create should attach VM-effective settings");
    assert_eq!(effective.profile_id, "coding");
}

#[tokio::test]
async fn telemetry_identity_env_uses_attached_profile_and_user_id() {
    let _guard = SETTINGS_ENV_LOCK.lock().await;
    let previous_user = std::env::var(capsem_core::telemetry::CAPSEM_USER_ID_ENV).ok();
    std::env::set_var(capsem_core::telemetry::CAPSEM_USER_ID_ENV, "corp-user");

    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions/vm-ident");
    std::fs::create_dir_all(&session_dir).unwrap();
    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let env = state
        .telemetry_identity_env("vm-ident", &session_dir)
        .unwrap();

    match previous_user {
        Some(value) => std::env::set_var(capsem_core::telemetry::CAPSEM_USER_ID_ENV, value),
        None => std::env::remove_var(capsem_core::telemetry::CAPSEM_USER_ID_ENV),
    }

    assert!(env
        .iter()
        .any(|(k, v)| { k == capsem_core::telemetry::CAPSEM_VM_ID_ENV && v == "vm-ident" }));
    assert!(env.iter().any(|(k, v)| {
        k == capsem_core::telemetry::CAPSEM_PROFILE_ID_ENV && v == "everyday-work"
    }));
    assert!(env
        .iter()
        .any(|(k, v)| { k == capsem_core::telemetry::CAPSEM_USER_ID_ENV && v == "corp-user" }));
}

#[tokio::test]
async fn handle_fork_creates_persistent_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    // Create a real session dir for the fake instance
    let session_dir = state.run_dir.join("sessions/fork-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let base_assets = test_saved_vm_base_assets();
    let source_profile_pin = state
        .vm_profile_pin(
            &session_dir,
            Some("2026.0520.1".into()),
            Some(test_profile_payload_hash()),
            Some(base_assets.clone()),
        )
        .unwrap();
    state.instances.lock().unwrap().insert(
        "fork-src".into(),
        InstanceInfo {
            id: "fork-src".into(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/fork-src.sock"),
            session_dir: session_dir.clone(),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: Some(base_assets.clone()),
            profile_pin: Some(source_profile_pin),
        },
    );
    let result = handle_fork(
        State(state.clone()),
        Path("fork-src".into()),
        Json(ForkRequest {
            name: "my-fork".into(),
            description: Some("test".into()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(result.0.name, "my-fork");
    assert!(result.0.size_bytes > 0);
    // Verify fork created a persistent sandbox entry in the registry
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("my-fork").unwrap();
    assert_eq!(entry.forked_from, Some("fork-src".into()));
    assert_eq!(entry.description, Some("test".into()));
    assert_eq!(entry.base_version, "0.0.0");
    assert_eq!(entry.base_assets, Some(base_assets));
    let pin = entry.profile_pin.as_ref().expect("fork must pin profile");
    assert_eq!(pin.profile_id, "everyday-work");
    assert_eq!(pin.profile_revision.as_deref(), Some("2026.0520.1"));
    assert!(pin.package_contract_hash.starts_with("blake3:"));
    assert_eq!(pin.base_assets, entry.base_assets);
}

#[tokio::test]
async fn handle_fork_preserves_profile_and_fork_exec_works() {
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/fork-exec-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let base_assets = test_saved_vm_base_assets();
    let source_profile_pin = state
        .vm_profile_pin(
            &session_dir,
            Some("2026.0520.1".into()),
            Some(test_profile_payload_hash()),
            Some(base_assets.clone()),
        )
        .unwrap();
    state.instances.lock().unwrap().insert(
        "fork-exec-src".into(),
        InstanceInfo {
            id: "fork-exec-src".into(),
            pid: std::process::id(),
            uds_path: dir.path().join("fork-exec-src.sock"),
            session_dir: session_dir.clone(),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: Some(base_assets.clone()),
            profile_pin: Some(source_profile_pin.clone()),
        },
    );

    let Json(fork_response) = handle_fork(
        State(state.clone()),
        Path("fork-exec-src".into()),
        Json(ForkRequest {
            name: "fork-exec".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(fork_response.name, "fork-exec");

    let fork_entry = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("fork-exec")
        .cloned()
        .unwrap();
    let fork_pin = fork_entry.profile_pin.as_ref().unwrap();
    assert_eq!(fork_pin.profile_id, source_profile_pin.profile_id);
    assert_eq!(
        fork_pin.profile_revision,
        source_profile_pin.profile_revision
    );
    assert_eq!(
        fork_pin.profile_payload_hash,
        source_profile_pin.profile_payload_hash
    );
    assert_eq!(
        fork_pin.package_contract_hash,
        source_profile_pin.package_contract_hash
    );
    assert_eq!(fork_pin.base_assets, source_profile_pin.base_assets);
    let fork_effective =
        capsem_core::settings_profiles::load_vm_effective_settings(&fork_entry.session_dir)
            .unwrap();
    assert_eq!(fork_effective.profile_id, source_profile_pin.profile_id);

    let fork_sock = dir.path().join("fork-exec.sock");
    let server = spawn_single_exec_server(fork_sock.clone(), b"fork-ok\n");
    state.instances.lock().unwrap().insert(
        "fork-exec".into(),
        InstanceInfo {
            id: "fork-exec".into(),
            pid: std::process::id(),
            uds_path: fork_sock,
            session_dir: fork_entry.session_dir,
            ram_mb: fork_entry.ram_mb,
            cpus: fork_entry.cpus,
            start_time: std::time::Instant::now(),
            base_version: fork_entry.base_version,
            persistent: true,
            env: None,
            forked_from: fork_entry.forked_from,
            base_assets: fork_entry.base_assets,
            profile_pin: fork_entry.profile_pin,
        },
    );

    let Json(exec) = handle_exec(
        State(state),
        Path("fork-exec".into()),
        Json(ExecRequest {
            command: "echo fork-ok".into(),
            timeout_secs: Some(5),
        }),
    )
    .await
    .unwrap();

    server.join().unwrap();
    assert_eq!(exec.stdout, "fork-ok\n");
    assert_eq!(exec.stderr, "");
    assert_eq!(exec.exit_code, 0);
}

#[tokio::test]
async fn handle_fork_rejects_profile_string_drift_after_clone() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/fork-profile-drift");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let base_assets = test_saved_vm_base_assets();
    let source_profile_pin = state
        .vm_profile_pin(
            &session_dir,
            Some("2026.0520.1".into()),
            Some(test_profile_payload_hash()),
            Some(base_assets.clone()),
        )
        .unwrap();
    let mut effective =
        capsem_core::settings_profiles::load_vm_effective_settings(&session_dir).unwrap();
    effective.profile_id = "tampered-profile".into();
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();
    state.instances.lock().unwrap().insert(
        "fork-profile-drift".into(),
        InstanceInfo {
            id: "fork-profile-drift".into(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/fork-profile-drift.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: Some(base_assets),
            profile_pin: Some(source_profile_pin),
        },
    );

    let err = handle_fork(
        State(state.clone()),
        Path("fork-profile-drift".into()),
        Json(ForkRequest {
            name: "drifted-fork".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1.contains("profile drift"),
        "unexpected error: {}",
        err.1
    );
    assert!(
        state
            .persistent_registry
            .lock()
            .unwrap()
            .get("drifted-fork")
            .is_none(),
        "profile drift must not register a persistent fork"
    );
}

#[tokio::test]
async fn handle_fork_rejects_source_without_profile_revision_pin() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/fork-src-no-pin");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    let base_assets = test_saved_vm_base_assets();
    state.instances.lock().unwrap().insert(
        "fork-src-no-pin".into(),
        InstanceInfo {
            id: "fork-src-no-pin".into(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/fork-src-no-pin.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: Some(base_assets),
            profile_pin: None,
        },
    );

    let err = handle_fork(
        State(state),
        Path("fork-src-no-pin".into()),
        Json(ForkRequest {
            name: "bad-fork".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1.contains("required profile revision pin"),
        "unexpected error: {}",
        err.1
    );
}

#[tokio::test]
async fn handle_fork_not_found() {
    let (state, _dir) = make_test_state_with_tempdir();
    // state is already Arc<ServiceState> from make_test_state*
    let err = handle_fork(
        State(state),
        Path("ghost".into()),
        Json(ForkRequest {
            name: "img".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handle_fork_duplicate_returns_conflict() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/dup-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let base_assets = test_saved_vm_base_assets();
    let source_profile_pin = state
        .vm_profile_pin(
            &session_dir,
            Some("2026.0520.1".into()),
            Some(test_profile_payload_hash()),
            Some(base_assets.clone()),
        )
        .unwrap();
    state.instances.lock().unwrap().insert(
        "dup-src".into(),
        InstanceInfo {
            id: "dup-src".into(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/dup-src.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: Some(base_assets),
            profile_pin: Some(source_profile_pin),
        },
    );
    // state is already Arc<ServiceState> from make_test_state*
    // First fork succeeds
    let _ = handle_fork(
        State(state.clone()),
        Path("dup-src".into()),
        Json(ForkRequest {
            name: "same-name".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    // Second fork with same name returns CONFLICT
    let err = handle_fork(
        State(state),
        Path("dup-src".into()),
        Json(ForkRequest {
            name: "same-name".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn handle_fork_from_persistent_registry() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/pers-vm");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    let (effective, trace) =
        capsem_core::settings_profiles::resolve_effective_vm_settings_with_trace(
            &capsem_core::settings_profiles::ProfileRootSettings::default(),
            None,
        )
        .unwrap();
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();
    capsem_core::settings_profiles::write_vm_effective_trace(&session_dir, &trace).unwrap();
    let base_assets = test_saved_vm_base_assets();
    let source_profile_pin = state
        .vm_profile_pin(
            &session_dir,
            Some("2026.0518.1".to_string()),
            Some(test_profile_payload_hash()),
            Some(base_assets.clone()),
        )
        .unwrap();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "pers-vm".into(),
            PersistentVmEntry {
                name: "pers-vm".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: Some(base_assets.clone()),
                profile_pin: Some(source_profile_pin.clone()),
                created_at: "2026-01-01T00:00:00Z".into(),
                session_dir: session_dir.clone(),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }
    // state is already Arc<ServiceState> from make_test_state*
    let result = handle_fork(
        State(state.clone()),
        Path("pers-vm".into()),
        Json(ForkRequest {
            name: "from-pers".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(result.0.name, "from-pers");
    let registry = state.persistent_registry.lock().unwrap();
    assert_eq!(
        registry.get("from-pers").unwrap().base_assets,
        Some(base_assets)
    );
    let fork_pin = registry
        .get("from-pers")
        .unwrap()
        .profile_pin
        .as_ref()
        .expect("forked persistent VM should preserve a profile pin");
    assert_eq!(fork_pin.profile_id, source_profile_pin.profile_id);
    assert_eq!(
        fork_pin.profile_revision,
        source_profile_pin.profile_revision
    );
    assert_eq!(
        fork_pin.profile_payload_hash,
        source_profile_pin.profile_payload_hash
    );
    assert_eq!(
        fork_pin.package_contract_hash,
        source_profile_pin.package_contract_hash
    );
    assert_eq!(fork_pin.base_assets, source_profile_pin.base_assets);
}

#[tokio::test]
async fn handle_fork_uses_profile_pin_assets_when_registry_side_field_is_absent() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/pers-pin-only");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.ensure_vm_effective_settings(&session_dir).unwrap();
    let base_assets = test_saved_vm_base_assets();
    let source_profile_pin = state
        .vm_profile_pin(
            &session_dir,
            Some("2026.0520.1".to_string()),
            Some(test_profile_payload_hash()),
            Some(base_assets.clone()),
        )
        .unwrap();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "pers-pin-only".into(),
            PersistentVmEntry {
                name: "pers-pin-only".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: None,
                profile_pin: Some(source_profile_pin.clone()),
                created_at: "0".into(),
                session_dir,
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let Json(result) = handle_fork(
        State(state.clone()),
        Path("pers-pin-only".into()),
        Json(ForkRequest {
            name: "pin-only-fork".into(),
            description: None,
        }),
    )
    .await
    .unwrap();

    assert_eq!(result.name, "pin-only-fork");
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("pin-only-fork").unwrap();
    assert_eq!(entry.base_assets, Some(base_assets));
    assert_eq!(
        entry.profile_pin.as_ref().unwrap().base_assets,
        source_profile_pin.base_assets
    );
}

#[tokio::test]
async fn handle_persist_rejects_running_vm_without_profile_revision_pin() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/persist-no-pin");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    let base_assets = test_saved_vm_base_assets();
    let mut profile_pin = test_saved_vm_profile_pin(base_assets.clone());
    profile_pin.profile_revision = None;
    state.instances.lock().unwrap().insert(
        "persist-no-pin".into(),
        InstanceInfo {
            id: "persist-no-pin".into(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/persist-no-pin.sock"),
            session_dir: session_dir.clone(),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: Some(base_assets),
            profile_pin: Some(profile_pin),
        },
    );

    let err = handle_persist(
        State(state.clone()),
        Path("persist-no-pin".into()),
        Json(PersistRequest {
            name: "persisted-no-pin".into(),
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1.contains("required profile revision pin"),
        "unexpected error: {}",
        err.1
    );
    assert!(
        session_dir.exists(),
        "failed persist must not move session dir"
    );
    assert!(
        state
            .persistent_registry
            .lock()
            .unwrap()
            .get("persisted-no-pin")
            .is_none(),
        "failed persist must not create persistent registry entry"
    );
}

#[test]
fn provision_rejects_nonexistent_source_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "vm1",
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: false,
        env: None,
        from: Some("ghost-sandbox".into()),
        profile_id: None,
        profile_revision: None,
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found"),
        "expected sandbox not found, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Suspend/resume registry fixes (issues #4-8)
// -----------------------------------------------------------------------

#[tokio::test]
async fn handle_list_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();

    // Register a suspended persistent VM
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "susp-vm".into(),
            PersistentVmEntry {
                name: "susp-vm".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: None,
                profile_pin: None,
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/susp-vm"),
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    // Register a stopped (not suspended) persistent VM
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "stop-vm".into(),
            PersistentVmEntry {
                name: "stop-vm".into(),
                ram_mb: 1024,
                cpus: 1,
                base_version: "0.0.0".into(),
                base_assets: None,
                profile_pin: None,
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/stop-vm"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let Json(list) = handle_list(State(state)).await;

    let susp = list.sandboxes.iter().find(|s| s.id == "susp-vm").unwrap();
    assert_eq!(
        susp.status, "Suspended",
        "suspended VM should show Suspended status"
    );

    let stop = list.sandboxes.iter().find(|s| s.id == "stop-vm").unwrap();
    assert_eq!(
        stop.status, "Stopped",
        "non-suspended VM should show Stopped status"
    );
}

#[tokio::test]
async fn handle_info_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();

    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "info-susp".into(),
            PersistentVmEntry {
                name: "info-susp".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: None,
                profile_pin: None,
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/info-susp"),
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    let result = handle_info(State(state), Path("info-susp".into())).await;
    let Json(info) = result.unwrap();
    assert_eq!(info.status, "Suspended");
}

#[tokio::test]
async fn handle_suspend_rejects_ephemeral_vm() {
    let (state, _dir) = make_test_state_with_tempdir();

    // Insert an ephemeral VM in instances
    {
        let mut instances = state.instances.lock().unwrap();
        instances.insert(
            "eph-vm".into(),
            InstanceInfo {
                id: "eph-vm".into(),
                pid: 0,
                uds_path: state.run_dir.join("instances/eph-vm.sock"),
                session_dir: state.run_dir.join("sessions/eph-vm"),
                ram_mb: 2048,
                cpus: 2,
                start_time: std::time::Instant::now(),
                base_version: "0.0.0".into(),
                persistent: false,
                env: None,
                forked_from: None,
                base_assets: None,
                profile_pin: None,
            },
        );
    }

    let result = handle_suspend(State(state), Path("eph-vm".into())).await;
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("ephemeral"));
}

#[tokio::test]
async fn handle_suspend_returns_not_found_for_missing_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = handle_suspend(State(state), Path("nonexistent".into())).await;
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[test]
fn archive_failed_restore_checkpoint_moves_checkpoint_aside() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/resume-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let checkpoint = session_dir.join("checkpoint.vzsave");
    std::fs::write(&checkpoint, b"bad checkpoint").unwrap();

    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "resume-vm".into(),
            PersistentVmEntry {
                name: "resume-vm".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: None,
                profile_pin: None,
                created_at: "0".into(),
                session_dir: session_dir.clone(),
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    let archived = state
        .archive_failed_restore_checkpoint("resume-vm")
        .expect("checkpoint should be archived");

    assert!(!checkpoint.exists(), "original checkpoint must be moved");
    assert!(
        archived.exists(),
        "archived checkpoint should exist: {}",
        archived.display()
    );
    assert!(archived
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("checkpoint.vzsave.failed-restore-"));
}

// -----------------------------------------------------------------------
// main_db_path
// -----------------------------------------------------------------------

#[test]
fn main_db_path_resolves_to_sessions_dir() {
    let state = make_test_state();
    // run_dir = /tmp/capsem-test-svc => parent = /tmp => main.db = /tmp/sessions/main.db
    let path = state.main_db_path();
    assert!(
        path.ends_with("sessions/main.db"),
        "got: {}",
        path.display()
    );
}

// -----------------------------------------------------------------------
// SandboxInfo::new
// -----------------------------------------------------------------------

#[test]
fn sandbox_info_new_defaults_telemetry_to_none() {
    let info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
    assert_eq!(info.id, "test");
    assert_eq!(info.pid, 1);
    assert!(!info.persistent);
    assert!(info.vm_id.is_none());
    assert!(info.profile_id.is_none());
    assert!(info.user_id.is_none());
    assert!(info.total_input_tokens.is_none());
    assert!(info.total_estimated_cost.is_none());
    assert!(info.model_call_count.is_none());
    assert!(info.created_at.is_none());
    assert!(info.uptime_secs.is_none());
}

#[test]
fn sandbox_info_telemetry_fields_serialize_when_present() {
    let mut info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
    info.vm_id = Some("test".into());
    info.profile_id = Some("everyday-work".into());
    info.user_id = Some("elie".into());
    info.profile_pin = Some(capsem_service::registry::SavedVmProfilePin {
        profile_id: "everyday-work".into(),
        profile_revision: Some("2026.0518.1".into()),
        profile_payload_hash: Some(
            "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
        ),
        package_contract_hash:
            "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into(),
        base_assets: None,
    });
    info.total_input_tokens = Some(1000);
    info.total_estimated_cost = Some(0.42);
    info.model_call_count = Some(5);
    let json = serde_json::to_string(&info).unwrap();
    assert!(json.contains("\"vm_id\":\"test\""));
    assert!(json.contains("\"profile_id\":\"everyday-work\""));
    assert!(json.contains("\"user_id\":\"elie\""));
    assert!(json.contains("\"profile_pin\""));
    assert!(json.contains("\"profile_revision\":\"2026.0518.1\""));
    assert!(json.contains("\"profile_payload_hash\""));
    assert!(json.contains("\"total_input_tokens\":1000"));
    assert!(json.contains("\"total_estimated_cost\":0.42"));
    assert!(json.contains("\"model_call_count\":5"));
}

#[test]
fn sandbox_info_telemetry_fields_omitted_when_none() {
    let info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
    let json = serde_json::to_string(&info).unwrap();
    assert!(!json.contains("total_input_tokens"));
    assert!(!json.contains("total_estimated_cost"));
    assert!(!json.contains("model_call_count"));
    assert!(!json.contains("uptime_secs"));
    assert!(!json.contains("profile_id"));
    assert!(!json.contains("profile_pin"));
    assert!(!json.contains("user_id"));
}

#[test]
fn sandbox_info_backwards_compatible_deserialization() {
    // Old JSON without telemetry fields should still deserialize
    let json = r#"{"id":"x","pid":1,"status":"Running","persistent":false}"#;
    let info: SandboxInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.id, "x");
    assert!(info.total_input_tokens.is_none());
    assert!(info.profile_id.is_none());
}

#[test]
fn enrich_telemetry_from_session_db_attaches_identity() {
    let dir = tempfile::tempdir().unwrap();
    {
        let writer = capsem_logger::DbWriter::open(&dir.path().join("session.db"), 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(capsem_logger::WriteOp::TelemetryIdentity(
                    capsem_logger::TelemetryIdentity {
                        timestamp: std::time::SystemTime::now(),
                        vm_id: "vm-ident".to_string(),
                        profile_id: "everyday-work".to_string(),
                        user_id: "elie".to_string(),
                    },
                ))
                .await;
        });
    }

    let mut info = SandboxInfo::new("vm-ident".into(), 1, "Running".into(), false);
    enrich_telemetry_from_session_db(&mut info, dir.path());
    assert_eq!(info.vm_id.as_deref(), Some("vm-ident"));
    assert_eq!(info.profile_id.as_deref(), Some("everyday-work"));
    assert_eq!(info.user_id.as_deref(), Some("elie"));
}

// -----------------------------------------------------------------------
// StatsResponse
// -----------------------------------------------------------------------

#[test]
fn stats_response_serializes() {
    let resp = StatsResponse {
        global: capsem_core::session::GlobalStats {
            total_sessions: 10,
            total_input_tokens: 5000,
            total_output_tokens: 2000,
            total_estimated_cost: 1.50,
            total_tool_calls: 100,
            total_mcp_calls: 20,
            total_file_events: 300,
            total_requests: 400,
            total_allowed: 380,
            total_denied: 20,
        },
        sessions: vec![],
        top_providers: vec![],
        top_tools: vec![],
        top_mcp_tools: vec![],
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"total_sessions\":10"));
    assert!(json.contains("\"total_estimated_cost\":1.5"));
    assert!(json.contains("\"top_providers\":[]"));
}

// -----------------------------------------------------------------------
// handle_list includes uptime_secs for running VMs
// -----------------------------------------------------------------------

#[tokio::test]
async fn handle_list_includes_uptime_for_running_vms() {
    let state = make_test_state();
    insert_fake_instance(&state, "vm-1", 100);
    let resp = handle_list(State(state)).await;
    let list = resp.0;
    assert_eq!(list.sandboxes.len(), 1);
    assert!(list.sandboxes[0].uptime_secs.is_some());
}

#[tokio::test]
async fn handle_list_does_not_scan_session_db_hot_path() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/list-hotpath");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    drop(writer);

    state.instances.lock().unwrap().insert(
        "list-hotpath".into(),
        InstanceInfo {
            id: "list-hotpath".into(),
            pid: std::process::id(),
            uds_path: state.run_dir.join("instances/list-hotpath.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: None,
            profile_pin: None,
        },
    );

    let Json(list) = handle_list(State(state)).await;
    let vm = list
        .sandboxes
        .iter()
        .find(|sandbox| sandbox.id == "list-hotpath")
        .expect("running VM should be listed");

    assert!(
        vm.total_requests.is_none(),
        "/list must not populate SQLite-backed network counters"
    );
    assert!(
        vm.model_call_count.is_none(),
        "/list must not populate SQLite-backed model counters"
    );
    assert!(
        vm.total_mcp_calls.is_none(),
        "/list must not populate SQLite-backed MCP counters"
    );
    assert!(
        vm.total_file_events.is_none(),
        "/list must not populate SQLite-backed file counters"
    );
}

// -----------------------------------------------------------------------
// handle_stats with tempdir
// -----------------------------------------------------------------------

#[tokio::test]
async fn handle_stats_returns_global_data() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();

    // Create main.db with a test session
    let idx = capsem_core::session::SessionIndex::open(&sessions_dir.join("main.db")).unwrap();
    let record = capsem_core::session::SessionRecord {
        id: "20260412-120000-abcd".into(),
        mode: "virtiofs".into(),
        command: Some("echo hello".into()),
        status: "stopped".into(),
        created_at: "2026-04-12T12:00:00Z".into(),
        stopped_at: Some("2026-04-12T12:05:00Z".into()),
        scratch_disk_size_gb: 16,
        ram_bytes: 4294967296,
        total_requests: 50,
        allowed_requests: 45,
        denied_requests: 5,
        total_input_tokens: 10000,
        total_output_tokens: 3000,
        total_estimated_cost: 0.42,
        total_tool_calls: 25,
        total_mcp_calls: 5,
        total_file_events: 100,
        compressed_size_bytes: None,
        vacuumed_at: None,
        storage_mode: "virtiofs".into(),
        rootfs_hash: None,
        rootfs_version: None,
        forked_from: None,
        persistent: false,
        exec_count: 0,
        audit_event_count: 0,
    };
    idx.create_session(&record).unwrap();
    drop(idx);

    let (state, _dir) = make_test_state_with_tempdir_at(dir);
    let result = handle_stats(State(state)).await;
    assert!(result.is_ok());
    let resp = result.unwrap().0;
    assert_eq!(resp.global.total_sessions, 1);
    assert_eq!(resp.global.total_input_tokens, 10000);
    assert_eq!(resp.global.total_estimated_cost, 0.42);
    assert_eq!(resp.sessions.len(), 1);
    assert_eq!(resp.sessions[0].id, "20260412-120000-abcd");
}

// -----------------------------------------------------------------------
// Settings handler tests
// -----------------------------------------------------------------------

struct SettingsEnvGuard {
    previous_capsem_home: Option<std::ffi::OsString>,
}

impl Drop for SettingsEnvGuard {
    fn drop(&mut self) {
        if let Some(previous_capsem_home) = self.previous_capsem_home.take() {
            std::env::set_var("CAPSEM_HOME", previous_capsem_home);
        } else {
            std::env::remove_var("CAPSEM_HOME");
        }
    }
}

fn install_settings_profiles_env(dir: &tempfile::TempDir) -> (SettingsEnvGuard, PathBuf, PathBuf) {
    let capsem_home = dir.path().join("home");
    let settings_path = capsem_home.join("service.toml");
    let base_dir = capsem_home.join("profiles").join("base");
    let corp_dir = capsem_home.join("profiles").join("corp");
    let user_dir = capsem_home.join("profiles").join("user");
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::create_dir_all(&corp_dir).unwrap();
    std::fs::create_dir_all(&user_dir).unwrap();

    let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
    settings.profiles.base_dirs = vec![base_dir];
    settings.profiles.corp_dirs = vec![corp_dir];
    settings.profiles.user_dirs = vec![user_dir.clone()];
    settings.profiles.default_profile =
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID.to_string();
    capsem_core::settings_profiles::write_service_settings(&settings_path, &settings).unwrap();

    let user_profile_path = user_dir.join(format!(
        "{}.toml",
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID
    ));

    let guard = SettingsEnvGuard {
        previous_capsem_home: std::env::var_os("CAPSEM_HOME"),
    };
    std::env::set_var("CAPSEM_HOME", &capsem_home);
    (guard, settings_path, user_profile_path)
}

#[tokio::test]
async fn handle_get_settings_returns_typed_payload() {
    let Json(val) = handle_get_settings().await;
    assert!(
        val.get("profile_presets").is_some(),
        "response must have 'profile_presets'"
    );
    assert!(
        val.get("effective_rules").is_some(),
        "response must have 'effective_rules'"
    );
    assert!(val.get("settings_profiles").is_some());
    assert_eq!(val["mode"], serde_json::json!("settings_profiles_v2"));
    assert!(val["profile_presets"].is_array());
    assert!(val["effective_rules"].is_object());
}

#[tokio::test]
async fn handle_policy_hook_spec_exports_spec0_contract() {
    let Json(val) = handle_policy_hook_spec().await;
    assert_eq!(val["openapi"], "3.1.0");
    assert_eq!(
        val["info"]["version"],
        capsem_core::net::policy_hook_spec::POLICY_HOOK_SPEC_VERSION
    );
    assert!(val["paths"].get("/v1/policy/decision").is_some());
    assert!(val["components"]["schemas"]
        .get("HookDecisionRequest")
        .is_some());
}

#[tokio::test]
async fn handle_get_presets_returns_list() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(val) = handle_get_presets().await;
    let arr = val.as_array().expect("presets should be an array");
    assert!(!arr.is_empty(), "should have at least one preset");
    assert!(arr[0].get("id").is_some());
    assert!(arr[0].get("name").is_some());
    assert!(arr[0].get("settings").is_some());
}

#[tokio::test]
async fn handle_list_profiles_returns_catalog_with_default_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(val) = handle_list_profiles().await.unwrap();
    assert_eq!(
        val["default_profile"],
        serde_json::json!(capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID)
    );
    let profiles = val["profiles"].as_array().expect("profiles array");
    assert!(
        profiles.iter().any(|profile| {
            profile["profile"]["id"]
                == serde_json::json!(capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID)
        }),
        "catalog should include the selected everyday-work profile"
    );
}

#[tokio::test]
async fn handle_profile_catalog_reports_manifest_and_installed_revisions() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);
    let home = dir.path().join("home");
    let corp_dir = home.join("profiles").join("corp");
    let manifest_json = r#"{
      "format": 1,
      "profiles": {
        "everyday-work": {
          "current_revision": "2026.0520.2",
          "revisions": {
            "2026.0520.1": {
              "status": "deprecated",
              "min_binary": "1.0.0",
              "profile_url": "file:///profiles/everyday-work/2026.0520.1/profile.json",
              "profile_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
              "profile_signature_url": "file:///profiles/everyday-work/2026.0520.1/profile.json.minisig"
            },
            "2026.0520.2": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///profiles/everyday-work/2026.0520.2/profile.json",
              "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
              "profile_signature_url": "file:///profiles/everyday-work/2026.0520.2/profile.json.minisig"
            }
          }
        }
      }
    }"#;
    std::fs::create_dir_all(corp_dir.join(".catalog/profiles/everyday-work")).unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profile-manifest.json"),
        manifest_json,
    )
    .unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profiles/everyday-work/current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.2",
          "payload_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        }"#,
    )
    .unwrap();

    let Json(val) = handle_profile_catalog().await.unwrap();

    assert_eq!(val["mode"], serde_json::json!("settings_profiles_v2"));
    assert_eq!(val["manifest_present"], serde_json::json!(true));
    assert_eq!(
        val["profiles"][0]["profile_id"],
        serde_json::json!("everyday-work")
    );
    assert_eq!(
        val["profiles"][0]["current_revision"],
        serde_json::json!("2026.0520.2")
    );
    assert_eq!(
        val["profiles"][0]["installed_revision"],
        serde_json::json!("2026.0520.2")
    );
    assert_eq!(val["profiles"][0]["revisions"][0]["status"], "deprecated");
    assert_eq!(
        val["profiles"][0]["revisions"][1]["installed"],
        serde_json::json!(true)
    );
}

#[tokio::test]
async fn handle_profile_catalog_reports_empty_state_without_manifest() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(val) = handle_profile_catalog().await.unwrap();

    assert_eq!(val["manifest_present"], serde_json::json!(false));
    assert_eq!(val["profiles"], serde_json::json!([]));
}

#[tokio::test]
async fn handle_profile_revisions_reports_current_and_installed_revision() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);
    let home = dir.path().join("home");
    let corp_dir = home.join("profiles").join("corp");
    let manifest_json = r#"{
      "format": 1,
      "profiles": {
        "everyday-work": {
          "current_revision": "2026.0520.2",
          "revisions": {
            "2026.0520.1": {
              "status": "deprecated",
              "min_binary": "1.0.0",
              "profile_url": "file:///profiles/everyday-work/2026.0520.1/profile.json",
              "profile_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
              "profile_signature_url": "file:///profiles/everyday-work/2026.0520.1/profile.json.minisig"
            },
            "2026.0520.2": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///profiles/everyday-work/2026.0520.2/profile.json",
              "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
              "profile_signature_url": "file:///profiles/everyday-work/2026.0520.2/profile.json.minisig"
            }
          }
        }
      }
    }"#;
    std::fs::create_dir_all(corp_dir.join(".catalog/profiles/everyday-work")).unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profile-manifest.json"),
        manifest_json,
    )
    .unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profiles/everyday-work/current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.2",
          "payload_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        }"#,
    )
    .unwrap();

    let Json(val) = handle_profile_revisions(Path("everyday-work".to_string()))
        .await
        .unwrap();

    assert_eq!(val["mode"], serde_json::json!("settings_profiles_v2"));
    assert_eq!(val["profile_id"], serde_json::json!("everyday-work"));
    assert_eq!(val["current_revision"], serde_json::json!("2026.0520.2"));
    assert_eq!(val["installed_revision"], serde_json::json!("2026.0520.2"));
    assert_eq!(val["revisions"][0]["status"], "deprecated");
    assert_eq!(val["revisions"][1]["status"], "active");
    assert_eq!(val["revisions"][1]["current"], serde_json::json!(true));
    assert_eq!(val["revisions"][1]["installed"], serde_json::json!(true));
}

#[tokio::test]
async fn handle_profile_revisions_returns_not_found_without_manifest() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let err = handle_profile_revisions(Path("everyday-work".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("profile catalog manifest is not present"));
}

#[tokio::test]
async fn handle_profile_revisions_returns_not_found_for_unknown_catalog_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);
    let home = dir.path().join("home");
    let corp_dir = home.join("profiles").join("corp");
    let manifest_json = r#"{
      "format": 1,
      "profiles": {
        "everyday-work": {
          "current_revision": "2026.0520.2",
          "revisions": {
            "2026.0520.2": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///profiles/everyday-work/2026.0520.2/profile.json",
              "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
              "profile_signature_url": "file:///profiles/everyday-work/2026.0520.2/profile.json.minisig"
            }
          }
        }
      }
    }"#;
    std::fs::create_dir_all(corp_dir.join(".catalog")).unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profile-manifest.json"),
        manifest_json,
    )
    .unwrap();

    let err = handle_profile_revisions(Path("missing-profile".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err
        .1
        .contains("profile catalog entry 'missing-profile' not found"));
}

fn write_profile_revision_action_manifest(
    dir: &tempfile::TempDir,
    settings_path: &std::path::Path,
    manifest_json: &str,
) {
    let pubkey = include_str!("../../../schemas/fixtures/profile-v2-test.pub");
    let mut settings =
        capsem_core::settings_profiles::load_service_settings_or_default(settings_path).unwrap();
    settings.profile_catalog.manifest_url =
        Some("https://profiles.example.test/profile-manifest.json".to_string());
    settings.profile_catalog.profile_payload_pubkey = Some(pubkey.to_string());
    capsem_core::settings_profiles::write_service_settings(settings_path, &settings).unwrap();
    std::fs::create_dir_all(
        dir.path()
            .join("home")
            .join("profiles")
            .join("corp")
            .join(".catalog"),
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join("home")
            .join("profiles")
            .join("corp")
            .join(".catalog")
            .join("profile-manifest.json"),
        manifest_json,
    )
    .unwrap();
}

fn signed_profile_revision_manifest(
    payload_path: &std::path::Path,
    signature_path: &std::path::Path,
    profile_hash: &str,
) -> String {
    format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "file://{}",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "file://{}"
                }},
                "2026.0520.2": {{
                  "status": "revoked",
                  "min_binary": "1.0.0",
                  "profile_url": "file://{}",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "file://{}"
                }}
              }}
            }}
          }}
        }}"#,
        payload_path.display(),
        signature_path.display(),
        payload_path.display(),
        signature_path.display(),
    )
}

#[tokio::test]
async fn handle_install_profile_revision_installs_active_current_revision() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, settings_path, _) = install_settings_profiles_env(&dir);
    let payload_path = dir.path().join("profile.json");
    let signature_path = dir.path().join("profile.json.minisig");
    let payload = include_str!("../../../schemas/fixtures/profile-v2-valid.json");
    let signature = include_str!("../../../schemas/fixtures/profile-v2-valid.json.minisig");
    std::fs::write(&payload_path, payload).unwrap();
    std::fs::write(&signature_path, signature).unwrap();
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest_json =
        signed_profile_revision_manifest(&payload_path, &signature_path, &profile_hash);
    write_profile_revision_action_manifest(&dir, &settings_path, &manifest_json);

    let Json(val) = handle_install_profile_revision(
        Path("everyday-work".to_string()),
        Json(ProfileRevisionActionRequest { revision: None }),
    )
    .await
    .unwrap();

    assert_eq!(val["action"], serde_json::json!("install"));
    assert_eq!(val["selected_revision"], serde_json::json!("2026.0520.1"));
    assert_eq!(val["outcome"]["outcome"], serde_json::json!("installed"));
    assert_eq!(
        val["outcome"]["payload_hash"],
        serde_json::json!(profile_hash)
    );
}

#[tokio::test]
async fn handle_install_profile_revision_rejects_revoked_revision() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, settings_path, _) = install_settings_profiles_env(&dir);
    let payload_path = dir.path().join("profile.json");
    let signature_path = dir.path().join("profile.json.minisig");
    let payload = include_str!("../../../schemas/fixtures/profile-v2-valid.json");
    std::fs::write(&payload_path, payload).unwrap();
    std::fs::write(
        &signature_path,
        include_str!("../../../schemas/fixtures/profile-v2-valid.json.minisig"),
    )
    .unwrap();
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest_json =
        signed_profile_revision_manifest(&payload_path, &signature_path, &profile_hash);
    write_profile_revision_action_manifest(&dir, &settings_path, &manifest_json);

    let err = handle_install_profile_revision(
        Path("everyday-work".to_string()),
        Json(ProfileRevisionActionRequest {
            revision: Some("2026.0520.2".to_string()),
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("only active revisions can be installed"));
}

#[tokio::test]
async fn handle_update_profile_revision_removes_revoked_installed_revision() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, settings_path, _) = install_settings_profiles_env(&dir);
    let payload_path = dir.path().join("profile.json");
    let signature_path = dir.path().join("profile.json.minisig");
    let payload = include_str!("../../../schemas/fixtures/profile-v2-valid.json");
    std::fs::write(&payload_path, payload).unwrap();
    std::fs::write(
        &signature_path,
        include_str!("../../../schemas/fixtures/profile-v2-valid.json.minisig"),
    )
    .unwrap();
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest_json =
        signed_profile_revision_manifest(&payload_path, &signature_path, &profile_hash);
    write_profile_revision_action_manifest(&dir, &settings_path, &manifest_json);
    let corp_dir = dir.path().join("home").join("profiles").join("corp");
    std::fs::create_dir_all(corp_dir.join(".catalog/profiles/everyday-work")).unwrap();
    std::fs::write(
        corp_dir.join("everyday-work.toml"),
        "id = \"everyday-work\"\n",
    )
    .unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profiles/everyday-work/current.json"),
        format!(
            r#"{{
              "profile_id": "everyday-work",
              "revision": "2026.0520.2",
              "payload_hash": "{profile_hash}"
            }}"#
        ),
    )
    .unwrap();

    let Json(val) = handle_update_profile_revision_lifecycle(
        Path("everyday-work".to_string()),
        Json(ProfileRevisionActionRequest {
            revision: Some("2026.0520.2".to_string()),
        }),
    )
    .await
    .unwrap();

    assert_eq!(val["action"], serde_json::json!("update"));
    assert_eq!(
        val["outcome"]["outcome"],
        serde_json::json!("revoked_removed")
    );
    assert!(!corp_dir.join("everyday-work.toml").exists());
    assert!(!corp_dir
        .join(".catalog/profiles/everyday-work/current.json")
        .exists());
}

#[tokio::test]
async fn handle_remove_profile_revision_removes_launchable_state() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);
    let corp_dir = dir.path().join("home").join("profiles").join("corp");
    std::fs::create_dir_all(corp_dir.join(".catalog/profiles/everyday-work/2026.0520.2")).unwrap();
    std::fs::write(
        corp_dir.join("everyday-work.toml"),
        "id = \"everyday-work\"\n",
    )
    .unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profiles/everyday-work/2026.0520.2/profile.json"),
        "{}",
    )
    .unwrap();
    std::fs::write(
        corp_dir.join(".catalog/profiles/everyday-work/current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.2",
          "payload_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        }"#,
    )
    .unwrap();

    let Json(val) = handle_remove_profile_revision(
        Path("everyday-work".to_string()),
        Json(ProfileRevisionActionRequest { revision: None }),
    )
    .await
    .unwrap();

    assert_eq!(val["action"], serde_json::json!("remove"));
    assert_eq!(val["selected_revision"], serde_json::json!("2026.0520.2"));
    assert_eq!(val["outcome"]["outcome"], serde_json::json!("removed"));
    assert!(!corp_dir.join("everyday-work.toml").exists());
    assert!(!corp_dir
        .join(".catalog/profiles/everyday-work/current.json")
        .exists());
    assert!(corp_dir
        .join(".catalog/profiles/everyday-work/2026.0520.2/profile.json")
        .exists());
}

#[tokio::test]
async fn handle_get_profile_returns_profile_record() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(val) = handle_get_profile(Path(
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID.to_string(),
    ))
    .await
    .unwrap();

    assert_eq!(
        val["profile"]["id"],
        serde_json::json!(capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID)
    );
    assert!(val["source"].is_string());
    assert!(val["locked"].is_boolean());
}

#[tokio::test]
async fn handle_get_profile_returns_not_found_for_unknown_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let err = handle_get_profile(Path("missing-profile".to_string()))
        .await
        .expect_err("unknown profile should return typed not-found error");

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("missing-profile"));
}

#[tokio::test]
async fn handle_resolve_profile_returns_effective_settings_and_trace() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(val) = handle_resolve_profile(Path(
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID.to_string(),
    ))
    .await
    .unwrap();

    assert_eq!(
        val["profile_id"],
        serde_json::json!(capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID)
    );
    assert_eq!(
        val["effective"]["profile_id"],
        serde_json::json!(capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID)
    );
    assert!(val["resolver_trace"]["events"].is_array());
}

#[tokio::test]
async fn handle_reconcile_profile_catalog_installs_current_active_revision() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);
    let payload_path = dir.path().join("profile.json");
    let signature_path = dir.path().join("profile.json.minisig");
    let payload = include_str!("../../../schemas/fixtures/profile-v2-valid.json");
    let signature = include_str!("../../../schemas/fixtures/profile-v2-valid.json.minisig");
    let pubkey = include_str!("../../../schemas/fixtures/profile-v2-test.pub");
    std::fs::write(&payload_path, payload).unwrap();
    std::fs::write(&signature_path, signature).unwrap();
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest_json = format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "file://{}",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "file://{}"
                }}
              }}
            }}
          }}
        }}"#,
        payload_path.display(),
        signature_path.display(),
    );

    let Json(val) = handle_reconcile_profile_catalog(Json(ProfileCatalogReconcileRequest {
        manifest_json: manifest_json.clone(),
        profile_payload_pubkey: pubkey.to_string(),
    }))
    .await
    .unwrap();

    assert_eq!(val["mode"], serde_json::json!("settings_profiles_v2"));
    assert_eq!(val["summary"]["installed"], serde_json::json!(1));
    assert_eq!(val["summary"]["errors"], serde_json::json!(0));
    assert_eq!(
        val["outcomes"][0]["outcome"],
        serde_json::json!("installed")
    );
    assert_eq!(
        val["outcomes"][0]["profile_id"],
        serde_json::json!("everyday-work")
    );
    assert_eq!(
        val["outcomes"][0]["revision"],
        serde_json::json!("2026.0520.1")
    );
    assert_eq!(
        val["outcomes"][0]["payload_hash"],
        serde_json::json!(profile_hash)
    );

    let installed = capsem_core::settings_profiles::load_installed_profile_revision(
        &capsem_core::settings_profiles::load_service_settings_or_default(
            dir.path().join("home").join("service.toml"),
        )
        .unwrap()
        .profiles,
        "everyday-work",
    )
    .unwrap()
    .expect("catalog reconcile should install current revision");
    assert_eq!(installed.revision, "2026.0520.1");
    assert_eq!(installed.payload_hash, profile_hash);
    let stored_manifest = std::fs::read_to_string(
        dir.path()
            .join("home")
            .join("profiles")
            .join("corp")
            .join(".catalog")
            .join("profile-manifest.json"),
    )
    .unwrap();
    assert_eq!(stored_manifest, manifest_json);
}

#[tokio::test]
async fn reconcile_configured_profile_catalog_fetches_manifest_source() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, settings_path, _) = install_settings_profiles_env(&dir);
    let payload_path = dir.path().join("profile.json");
    let signature_path = dir.path().join("profile.json.minisig");
    let payload = include_str!("../../../schemas/fixtures/profile-v2-valid.json");
    let signature = include_str!("../../../schemas/fixtures/profile-v2-valid.json.minisig");
    let pubkey = include_str!("../../../schemas/fixtures/profile-v2-test.pub");
    std::fs::write(&payload_path, payload).unwrap();
    std::fs::write(&signature_path, signature).unwrap();
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest_json = format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "file://{}",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "file://{}"
                }}
              }}
            }}
          }}
        }}"#,
        payload_path.display(),
        signature_path.display(),
    );
    let (manifest_url, server) = start_profile_catalog_manifest_server(manifest_json.clone()).await;
    let mut settings =
        capsem_core::settings_profiles::load_service_settings_or_default(&settings_path).unwrap();
    settings.profile_catalog.manifest_url = Some(manifest_url);
    settings.profile_catalog.profile_payload_pubkey = Some(pubkey.to_string());

    let val = reconcile_configured_profile_catalog(&settings)
        .await
        .unwrap();

    server.abort();
    assert_eq!(val["summary"]["installed"], serde_json::json!(1));
    assert_eq!(val["summary"]["errors"], serde_json::json!(0));
    let installed = capsem_core::settings_profiles::load_installed_profile_revision(
        &settings.profiles,
        "everyday-work",
    )
    .unwrap()
    .expect("configured catalog reconcile should install current revision");
    assert_eq!(installed.revision, "2026.0520.1");
    assert_eq!(installed.payload_hash, profile_hash);
    let stored_manifest = std::fs::read_to_string(
        dir.path()
            .join("home")
            .join("profiles")
            .join("corp")
            .join(".catalog")
            .join("profile-manifest.json"),
    )
    .unwrap();
    assert_eq!(stored_manifest, manifest_json);
}

#[tokio::test]
async fn handle_reconcile_profile_catalog_removes_revoked_installed_revision() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);
    let home = dir.path().join("home");
    let corp_dir = home.join("profiles").join("corp");
    std::fs::write(corp_dir.join("everyday-work.toml"), "runtime profile").unwrap();
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    std::fs::create_dir_all(&record_dir).unwrap();
    std::fs::write(
        record_dir.join("current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        }"#,
    )
    .unwrap();
    let manifest_json = r#"{
      "format": 1,
      "profiles": {
        "everyday-work": {
          "current_revision": "2026.0520.2",
          "revisions": {
            "2026.0520.1": {
              "status": "revoked",
              "min_binary": "1.0.0",
              "profile_url": "file:///definitely/not/read/profile.json",
              "profile_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
              "profile_signature_url": "file:///definitely/not/read/profile.json.minisig"
            },
            "2026.0520.2": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///definitely/not/read/profile.json",
              "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
              "profile_signature_url": "file:///definitely/not/read/profile.json.minisig"
            }
          }
        }
      }
    }"#;

    let Json(val) = handle_reconcile_profile_catalog(Json(ProfileCatalogReconcileRequest {
        manifest_json: manifest_json.to_string(),
        profile_payload_pubkey: "unused".to_string(),
    }))
    .await
    .unwrap();

    assert_eq!(val["summary"]["revoked_removed"], serde_json::json!(1));
    assert_eq!(val["summary"]["errors"], serde_json::json!(1));
    assert!(val["outcomes"].as_array().unwrap().iter().any(|outcome| {
        outcome["outcome"] == serde_json::json!("revoked_removed")
            && outcome["revision"] == serde_json::json!("2026.0520.1")
    }));
    assert!(
        val["outcomes"].as_array().unwrap().iter().any(|outcome| {
            outcome["outcome"] == serde_json::json!("error")
                && outcome["revision"] == serde_json::json!("2026.0520.2")
        }),
        "current active revision should report download/signature errors without hiding revoke result"
    );
    assert!(!corp_dir.join("everyday-work.toml").exists());
    assert!(!record_dir.join("current.json").exists());
}

#[tokio::test]
async fn handle_reconcile_profile_catalog_removes_absent_installed_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);
    let home = dir.path().join("home");
    let corp_dir = home.join("profiles").join("corp");
    std::fs::write(corp_dir.join("everyday-work.toml"), "runtime profile").unwrap();
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    std::fs::create_dir_all(record_dir.join("2026.0520.1")).unwrap();
    std::fs::write(record_dir.join("2026.0520.1").join("profile.json"), "{}").unwrap();
    std::fs::write(
        record_dir.join("current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        }"#,
    )
    .unwrap();
    let manifest_json = r#"{
      "format": 1,
      "profiles": {
        "coding": {
          "current_revision": "2026.0520.1",
          "revisions": {
            "2026.0520.1": {
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "file:///definitely/not/read/profile.json",
              "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
              "profile_signature_url": "file:///definitely/not/read/profile.json.minisig"
            }
          }
        }
      }
    }"#;

    let Json(val) = handle_reconcile_profile_catalog(Json(ProfileCatalogReconcileRequest {
        manifest_json: manifest_json.to_string(),
        profile_payload_pubkey: "unused".to_string(),
    }))
    .await
    .unwrap();

    assert_eq!(val["summary"]["absent_removed"], serde_json::json!(1));
    assert_eq!(val["summary"]["errors"], serde_json::json!(1));
    assert!(val["outcomes"].as_array().unwrap().iter().any(|outcome| {
        outcome["outcome"] == serde_json::json!("absent_removed")
            && outcome["profile_id"] == serde_json::json!("everyday-work")
            && outcome["revision"] == serde_json::json!("2026.0520.1")
    }));
    assert!(!corp_dir.join("everyday-work.toml").exists());
    assert!(!record_dir.join("current.json").exists());
    assert!(record_dir.join("2026.0520.1").join("profile.json").exists());
}

fn custom_profile(id: &str, name: &str) -> capsem_core::settings_profiles::Profile {
    let mut profile = capsem_core::settings_profiles::Profile::everyday_work();
    profile.id = id.to_string();
    profile.name = name.to_string();
    profile.description = format!("{name} description");
    profile.best_for = format!("{name} work");
    profile.profile_type = capsem_core::settings_profiles::ProfileType::Coding;
    profile
}

fn write_profile_fixture(path: &std::path::Path, id: &str, name: &str) {
    std::fs::write(
        path,
        format!(
            r#"
version = 1
id = "{id}"
name = "{name}"
best_for = "{name} sessions."
profile_type = "coding"
"#
        ),
    )
    .unwrap();
}

fn test_profile_rule(
    callback: &str,
    condition: &str,
    decision: capsem_core::settings_profiles::RuleDecision,
    priority: i32,
    reason: &str,
) -> capsem_core::settings_profiles::ProfileRule {
    capsem_core::settings_profiles::ProfileRule {
        callback: callback.to_string(),
        condition: condition.to_string(),
        decision,
        priority,
        reason: Some(reason.to_string()),
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
    }
}

fn test_mcp_connector() -> capsem_core::settings_profiles::McpConnectorConfig {
    capsem_core::settings_profiles::McpConnectorConfig {
        enabled: true,
        server_type: Some("stdio".to_string()),
        command: Some("npx".to_string()),
        args: vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-github".to_string(),
        ],
        env: std::collections::BTreeMap::new(),
        url: None,
        headers: std::collections::BTreeMap::new(),
        bearer_token: None,
        pool_size: None,
        pool_safe_tools: Vec::new(),
        capsem: capsem_core::settings_profiles::McpConnectorCapsemMetadata {
            credential_refs: vec!["github-token".to_string()],
            allowed_tools: vec!["repo.read".to_string()],
            rules: capsem_core::settings_profiles::SecurityRules::default(),
        },
    }
}

#[tokio::test]
async fn handle_create_profile_persists_user_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(val) = handle_create_profile(Json(custom_profile("custom", "Custom")))
        .await
        .unwrap();

    assert_eq!(val["profile"]["id"], serde_json::json!("custom"));
    assert_eq!(val["source"], serde_json::json!("user"));
    assert_eq!(val["locked"], serde_json::json!(false));

    let Json(list) = handle_list_profiles().await.unwrap();
    assert!(list["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["profile"]["id"] == serde_json::json!("custom")));
}

#[tokio::test]
async fn handle_create_profile_rejects_existing_builtin_profile_id() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, user_profile_path) = install_settings_profiles_env(&dir);

    let err = handle_create_profile(Json(custom_profile(
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID,
        "Builtin Shadow",
    )))
    .await
    .expect_err("create route must not shadow locked built-in profiles");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("already exists") || err.1.contains("locked"));
    assert!(
        !user_profile_path.exists(),
        "rejected profile create must not write a built-in shadow file"
    );
}

#[tokio::test]
async fn handle_create_profile_rejects_existing_base_profile_id() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);
    let base_profile_path = dir
        .path()
        .join("home")
        .join("profiles")
        .join("base")
        .join("base-locked.toml");
    write_profile_fixture(&base_profile_path, "base-locked", "Base Locked");

    let err = handle_create_profile(Json(custom_profile("base-locked", "User Shadow")))
        .await
        .expect_err("create route must not shadow base profiles");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("already exists") || err.1.contains("locked"));
    assert!(
        !dir.path()
            .join("home")
            .join("profiles")
            .join("user")
            .join("base-locked.toml")
            .exists(),
        "rejected profile create must not write a base shadow file"
    );
}

#[tokio::test]
async fn handle_update_profile_rejects_path_body_id_mismatch() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let err = handle_update_profile(
        Path("path-id".to_string()),
        Json(custom_profile("body-id", "Body")),
    )
    .await
    .expect_err("route id/body id mismatch should fail closed");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("does not match"));
}

#[tokio::test]
async fn handle_update_profile_persists_existing_user_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let _ = handle_create_profile(Json(custom_profile("custom", "Custom")))
        .await
        .unwrap();
    let mut updated = custom_profile("custom", "Custom Updated");
    updated.best_for = "Updated work".to_string();

    let Json(val) = handle_update_profile(Path("custom".to_string()), Json(updated))
        .await
        .unwrap();

    assert_eq!(val["profile"]["name"], serde_json::json!("Custom Updated"));
    assert_eq!(
        val["profile"]["best_for"],
        serde_json::json!("Updated work")
    );
}

#[tokio::test]
async fn profile_section_locks_allow_skills_and_mcp_but_block_ai_and_rules() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("section-locks", "Section Locks");
    profile.editable.ai = false;
    profile.editable.security_rules = false;
    profile.editable.skills = true;
    profile.editable.mcp_servers = true;
    let _ = handle_create_profile(Json(profile)).await.unwrap();

    let Json(skill) = handle_create_skill(Json(SkillMutationRequest {
        profile: Some("section-locks".to_string()),
        id: "dev-sprint".to_string(),
        kind: SkillKind::Enabled,
    }))
    .await
    .unwrap();
    assert_eq!(skill["editable"], serde_json::json!(true));

    let Json(server) = handle_create_mcp_connector(Json(McpConnectorMutationRequest {
        profile: Some("section-locks".to_string()),
        id: "github".to_string(),
        connector: test_mcp_connector(),
    }))
    .await
    .unwrap();
    assert_eq!(server["editable"], serde_json::json!(true));

    let err = handle_create_rule(Json(RuleCreateRequest {
        profile: Some("section-locks".to_string()),
        id: "security.rules.http.ask_probe".to_string(),
        update: PolicyRuleUpdate {
            callback: "http.request".to_string(),
            condition: "request.host == 'probe.example.com'".to_string(),
            decision: capsem_core::settings_profiles::RuleDecision::Ask,
            priority: 20,
            reason: Some("section lock proof".to_string()),
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    }))
    .await
    .expect_err("security.rules lock must block rule creation");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("profile_section_locked"));
    assert!(err.1.contains("security.rules"));

    let mut updated = handle_get_profile(Path("section-locks".to_string()))
        .await
        .unwrap()
        .0["profile"]
        .clone();
    updated["ai"]["providers"]["openai"] = serde_json::json!({
        "enabled": true,
        "model": "gpt-5.2",
        "base_url": "https://api.openai.com/v1"
    });
    let updated: capsem_core::settings_profiles::Profile = serde_json::from_value(updated).unwrap();
    let err = handle_update_profile(Path("section-locks".to_string()), Json(updated))
        .await
        .expect_err("ai lock must block whole-profile update smuggling");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("profile_section_locked"));
    assert!(err.1.contains("ai"));

    let mut updated = handle_get_profile(Path("section-locks".to_string()))
        .await
        .unwrap()
        .0["profile"]
        .clone();
    updated["editable"]["security_rules"] = serde_json::json!(true);
    let updated: capsem_core::settings_profiles::Profile = serde_json::from_value(updated).unwrap();
    let err = handle_update_profile(Path("section-locks".to_string()), Json(updated))
        .await
        .expect_err("editable lock map must not be mutable through whole-profile update");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("profile_section_locked"));
    assert!(err.1.contains("editable"));
}

#[tokio::test]
async fn handle_fork_profile_creates_user_copy() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(val) = handle_fork_profile(
        Path(capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID.to_string()),
        Json(ProfileForkRequest {
            id: "daily-strict".to_string(),
            name: "Daily Strict".to_string(),
        }),
    )
    .await
    .unwrap();

    assert_eq!(val["profile"]["id"], serde_json::json!("daily-strict"));
    assert_eq!(val["profile"]["name"], serde_json::json!("Daily Strict"));
    assert_eq!(val["source"], serde_json::json!("user"));
}

#[tokio::test]
async fn handle_fork_profile_propagates_section_locks() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut source = custom_profile("locked-source", "Locked Source");
    source.editable.skills = false;
    source.editable.mcp_servers = true;
    let _ = handle_create_profile(Json(source)).await.unwrap();

    let Json(forked) = handle_fork_profile(
        Path("locked-source".to_string()),
        Json(ProfileForkRequest {
            id: "locked-fork".to_string(),
            name: "Locked Fork".to_string(),
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        forked["profile"]["editable"]["skills"],
        serde_json::json!(false)
    );
    assert_eq!(
        forked["profile"]["editable"]["mcpServers"],
        serde_json::json!(true)
    );

    let err = handle_create_skill(Json(SkillMutationRequest {
        profile: Some("locked-fork".to_string()),
        id: "dev-sprint".to_string(),
        kind: SkillKind::Enabled,
    }))
    .await
    .expect_err("forked profile must preserve skills section lock");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("profile_section_locked"));
    assert!(err.1.contains("skills"));

    let Json(server) = handle_create_mcp_connector(Json(McpConnectorMutationRequest {
        profile: Some("locked-fork".to_string()),
        id: "github".to_string(),
        connector: test_mcp_connector(),
    }))
    .await
    .unwrap();
    assert_eq!(server["editable"], serde_json::json!(true));
}

#[tokio::test]
async fn handle_delete_profile_removes_user_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let _ = handle_create_profile(Json(custom_profile("custom", "Custom")))
        .await
        .unwrap();
    let Json(val) = handle_delete_profile(Path("custom".to_string()))
        .await
        .unwrap();

    assert_eq!(val["deleted"], serde_json::json!("custom"));
    let err = handle_get_profile(Path("custom".to_string()))
        .await
        .expect_err("deleted profile should no longer be discoverable");
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handle_delete_profile_rejects_locked_builtin_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let err = handle_delete_profile(Path(
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID.to_string(),
    ))
    .await
    .expect_err("built-in profile deletes should fail closed");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("locked"));
}

#[tokio::test]
async fn settings_save_updates_selected_user_profile_after_preset_switch() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, builtin_override_path) = install_settings_profiles_env(&dir);

    let _ = handle_create_profile(Json(custom_profile("custom", "Custom")))
        .await
        .unwrap();
    let Json(selected) = handle_select_profile_preset(Path("custom".to_string()))
        .await
        .unwrap();
    assert_eq!(
        selected["settings_profiles"]["selected_profile_id"],
        serde_json::json!("custom")
    );

    let mut changes = HashMap::new();
    changes.insert(
        "policy.http.block_custom".into(),
        serde_json::json!({
            "on": "http.request",
            "if": "request.host == 'custom.example.com'",
            "decision": "block",
            "priority": 10,
            "reason": "selected profile rule"
        }),
    );

    let Json(val) = handle_save_settings(Json(changes)).await.unwrap();

    assert_eq!(
        val["settings_profiles"]["selected_profile_id"],
        serde_json::json!("custom")
    );
    assert_eq!(
        val["settings_profiles"]["effective"]["profile_id"],
        serde_json::json!("custom")
    );
    let custom_profile_path = dir
        .path()
        .join("home")
        .join("profiles")
        .join("user")
        .join("custom.toml");
    let custom_text = std::fs::read_to_string(custom_profile_path).unwrap();
    assert!(custom_text.contains("[security.rules.http.block_custom]"));
    assert!(
        !builtin_override_path.exists(),
        "saving settings for selected user profile must not create a built-in default override"
    );
}

#[tokio::test]
async fn handle_list_rules_returns_effective_rules_with_canonical_ids() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("custom", "Custom");
    profile.security.rules.http.insert(
        "block_openai".to_string(),
        test_profile_rule(
            "http.request",
            "request.host == 'api.openai.com'",
            capsem_core::settings_profiles::RuleDecision::Block,
            25,
            "test block",
        ),
    );
    let _ = handle_create_profile(Json(profile)).await.unwrap();

    let Json(val) = handle_list_rules(Query(RulesQuery {
        profile: Some("custom".to_string()),
        callback: Some("http.request".to_string()),
    }))
    .await
    .unwrap();

    assert_eq!(val["mode"], serde_json::json!("settings_profiles_v2"));
    assert_eq!(val["profile_id"], serde_json::json!("custom"));
    let rules = val["rules"].as_array().expect("rules array");
    let rule = rules
        .iter()
        .find(|rule| rule["id"] == serde_json::json!("security.rules.http.block_openai"))
        .expect("custom HTTP rule should be listed by canonical id");
    assert_eq!(rule["effective_id"], serde_json::json!("http.block_openai"));
    assert_eq!(rule["source_profile"], serde_json::json!("custom"));
    assert_eq!(rule["rule"]["on"], serde_json::json!("http.request"));
    assert_eq!(
        rule["rule"]["if"],
        serde_json::json!("request.host == 'api.openai.com'")
    );
    assert_eq!(rule["rule"]["priority"], serde_json::json!(25));
    assert_eq!(rule["editable"], serde_json::json!(true));
}

#[tokio::test]
async fn mcp_connectors_api_create_list_delete_roundtrip_updates_user_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let _ = handle_create_profile(Json(custom_profile("mcp-user", "MCP User")))
        .await
        .unwrap();

    let Json(created) = handle_create_mcp_connector(Json(McpConnectorMutationRequest {
        profile: Some("mcp-user".to_string()),
        id: "github".to_string(),
        connector: test_mcp_connector(),
    }))
    .await
    .unwrap();

    assert_eq!(created["id"], serde_json::json!("github"));
    assert_eq!(created["source_profile"], serde_json::json!("mcp-user"));
    assert_eq!(created["editable"], serde_json::json!(true));
    assert_eq!(
        created["server"]["capsem"]["allowed_tools"],
        serde_json::json!(["repo.read"])
    );

    let Json(listed) = handle_mcp_connectors(Query(McpConnectorsQuery {
        profile: Some("mcp-user".to_string()),
    }))
    .await
    .unwrap();
    assert!(listed["servers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|server| server["id"] == serde_json::json!("github")));

    let Json(deleted) = handle_delete_mcp_connector(
        Path("github".to_string()),
        Query(McpConnectorsQuery {
            profile: Some("mcp-user".to_string()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(deleted["server_id"], serde_json::json!("github"));
    assert_eq!(deleted["removed"], serde_json::json!(true));

    let Json(after_delete) = handle_mcp_connectors(Query(McpConnectorsQuery {
        profile: Some("mcp-user".to_string()),
    }))
    .await
    .unwrap();
    assert!(after_delete["servers"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn handle_create_mcp_connector_materializes_default_builtin_profile_override() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, user_profile_path) = install_settings_profiles_env(&dir);

    assert!(!user_profile_path.exists());
    let Json(created) = handle_create_mcp_connector(Json(McpConnectorMutationRequest {
        profile: None,
        id: "github".to_string(),
        connector: test_mcp_connector(),
    }))
    .await
    .unwrap();

    assert_eq!(created["id"], serde_json::json!("github"));
    assert!(user_profile_path.exists());
    let text = std::fs::read_to_string(user_profile_path).unwrap();
    assert!(text.contains("[mcpServers.github]"));
    assert!(text.contains("command = \"npx\""));
    assert!(text.contains("[mcpServers.github.capsem]"));
    assert!(text.contains("allowed_tools = [\"repo.read\"]"));
}

#[tokio::test]
async fn handle_create_mcp_connector_rejects_duplicate_direct_connector() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("mcp-user", "MCP User");
    profile
        .mcp
        .connectors
        .insert("github".to_string(), test_mcp_connector());
    let _ = handle_create_profile(Json(profile)).await.unwrap();

    let err = handle_create_mcp_connector(Json(McpConnectorMutationRequest {
        profile: Some("mcp-user".to_string()),
        id: "github".to_string(),
        connector: test_mcp_connector(),
    }))
    .await
    .expect_err("duplicate MCP server create should fail closed");

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("server_exists"));
}

#[tokio::test]
async fn skills_api_create_list_delete_roundtrip_updates_user_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let _ = handle_create_profile(Json(custom_profile("skills-user", "Skills User")))
        .await
        .unwrap();

    let Json(created) = handle_create_skill(Json(SkillMutationRequest {
        profile: Some("skills-user".to_string()),
        id: "dev-sprint".to_string(),
        kind: SkillKind::Enabled,
    }))
    .await
    .unwrap();

    assert_eq!(created["id"], serde_json::json!("dev-sprint"));
    assert_eq!(created["kind"], serde_json::json!("enabled"));
    assert_eq!(created["source_profile"], serde_json::json!("skills-user"));
    assert_eq!(created["editable"], serde_json::json!(true));

    let Json(listed) = handle_list_skills(Query(SkillsQuery {
        profile: Some("skills-user".to_string()),
        kind: Some(SkillKind::Enabled),
    }))
    .await
    .unwrap();
    assert!(listed["enabled"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("dev-sprint")));
    assert!(listed["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|skill| skill["id"] == serde_json::json!("dev-sprint")
            && skill["kind"] == serde_json::json!("enabled")));

    let Json(deleted) = handle_delete_skill(
        Path("dev-sprint".to_string()),
        Query(SkillsQuery {
            profile: Some("skills-user".to_string()),
            kind: Some(SkillKind::Enabled),
        }),
    )
    .await
    .unwrap();
    assert_eq!(deleted["skill_id"], serde_json::json!("dev-sprint"));
    assert_eq!(deleted["kind"], serde_json::json!("enabled"));
    assert_eq!(deleted["removed"], serde_json::json!(true));

    let Json(after_delete) = handle_list_skills(Query(SkillsQuery {
        profile: Some("skills-user".to_string()),
        kind: Some(SkillKind::Enabled),
    }))
    .await
    .unwrap();
    assert!(after_delete["enabled"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn handle_create_skill_rejects_duplicate_direct_skill() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let _ = handle_create_profile(Json(custom_profile("skills-user", "Skills User")))
        .await
        .unwrap();
    let request = SkillMutationRequest {
        profile: Some("skills-user".to_string()),
        id: "dev-sprint".to_string(),
        kind: SkillKind::Enabled,
    };
    let _ = handle_create_skill(Json(request.clone())).await.unwrap();

    let err = handle_create_skill(Json(request))
        .await
        .expect_err("duplicate direct skill should fail closed");

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("skill_exists: skills.enabled.dev-sprint"));
}

#[tokio::test]
async fn handle_create_skill_rejects_duplicate_inherited_skill() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut parent = custom_profile("skills-parent", "Skills Parent");
    parent.skills.enabled.push("dev-sprint".to_string());
    let _ = handle_create_profile(Json(parent)).await.unwrap();
    let mut child = custom_profile("skills-child", "Skills Child");
    child.extends_profile_id = Some("skills-parent".to_string());
    let _ = handle_create_profile(Json(child)).await.unwrap();

    let err = handle_create_skill(Json(SkillMutationRequest {
        profile: Some("skills-child".to_string()),
        id: "dev-sprint".to_string(),
        kind: SkillKind::Enabled,
    }))
    .await
    .expect_err("duplicate inherited skill should fail closed");

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("skill_exists: skills.enabled.dev-sprint"));
    assert!(err.1.contains("skills-parent"));
}

#[tokio::test]
async fn handle_create_skill_moves_skill_between_enabled_and_disabled_lists() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("skills-user", "Skills User");
    profile.skills.disabled.push("dev-sprint".to_string());
    let _ = handle_create_profile(Json(profile)).await.unwrap();

    let Json(created) = handle_create_skill(Json(SkillMutationRequest {
        profile: Some("skills-user".to_string()),
        id: "dev-sprint".to_string(),
        kind: SkillKind::Enabled,
    }))
    .await
    .unwrap();

    assert_eq!(created["kind"], serde_json::json!("enabled"));
    let Json(listed) = handle_list_skills(Query(SkillsQuery {
        profile: Some("skills-user".to_string()),
        kind: None,
    }))
    .await
    .unwrap();
    assert!(listed["enabled"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("dev-sprint")));
    assert!(!listed["disabled"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("dev-sprint")));
}

#[tokio::test]
async fn handle_delete_skill_rejects_inherited_skill() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut parent = custom_profile("skills-parent", "Skills Parent");
    parent.skills.enabled.push("dev-sprint".to_string());
    let _ = handle_create_profile(Json(parent)).await.unwrap();
    let mut child = custom_profile("skills-child", "Skills Child");
    child.extends_profile_id = Some("skills-parent".to_string());
    let _ = handle_create_profile(Json(child)).await.unwrap();

    let err = handle_delete_skill(
        Path("dev-sprint".to_string()),
        Query(SkillsQuery {
            profile: Some("skills-child".to_string()),
            kind: Some(SkillKind::Enabled),
        }),
    )
    .await
    .expect_err("inherited skill delete should fail closed");

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("skill_is_locked"));
}

#[tokio::test]
async fn handle_list_pending_confirms_returns_typed_empty_s07_surface() {
    let Json(pending) = handle_list_pending_confirms().await;

    assert_eq!(pending["mode"], serde_json::json!("settings_profiles_v2"));
    assert_eq!(pending["pending_count"], serde_json::json!(0));
    assert_eq!(pending["pending"], serde_json::json!([]));
    assert_eq!(pending["resolve_available"], serde_json::json!(false));
    assert_eq!(
        pending["resolve_owner"],
        serde_json::json!("S15-confirm-ux")
    );
}

#[tokio::test]
async fn s07_route_surface_chains_profiles_skills_mcp_rules_and_confirm_listing() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(profile) = handle_create_profile(Json(custom_profile("s07-chain", "S07 Chain")))
        .await
        .unwrap();
    assert_eq!(profile["profile"]["id"], serde_json::json!("s07-chain"));

    let Json(skill) = handle_create_skill(Json(SkillMutationRequest {
        profile: Some("s07-chain".to_string()),
        id: "dev-sprint".to_string(),
        kind: SkillKind::Enabled,
    }))
    .await
    .unwrap();
    assert_eq!(skill["editable"], serde_json::json!(true));

    let Json(server) = handle_create_mcp_connector(Json(McpConnectorMutationRequest {
        profile: Some("s07-chain".to_string()),
        id: "github".to_string(),
        connector: test_mcp_connector(),
    }))
    .await
    .unwrap();
    assert_eq!(server["server"]["command"], serde_json::json!("npx"));

    let Json(rule) = handle_create_rule(Json(RuleCreateRequest {
        profile: Some("s07-chain".to_string()),
        id: "security.rules.http.ask_probe".to_string(),
        update: PolicyRuleUpdate {
            callback: "http.request".to_string(),
            condition: "request.host == 'probe.example.com'".to_string(),
            decision: capsem_core::settings_profiles::RuleDecision::Ask,
            priority: 20,
            reason: Some("S07 chained route proof".to_string()),
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    }))
    .await
    .unwrap();
    assert_eq!(
        rule["id"],
        serde_json::json!("security.rules.http.ask_probe")
    );

    let Json(evaluated) = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: Some("s07-chain".to_string()),
        callback: "http.request".to_string(),
        subject: serde_json::json!({
            "request": {
                "host": "probe.example.com",
                "method": "GET"
            }
        }),
    }))
    .await
    .unwrap();
    assert_eq!(evaluated["decision"], serde_json::json!("ask"));
    assert_eq!(evaluated["would_ask"], serde_json::json!(true));

    let Json(pending) = handle_list_pending_confirms().await;
    assert_eq!(pending["pending_count"], serde_json::json!(0));

    let Json(effective) = handle_resolve_profile(Path("s07-chain".to_string()))
        .await
        .unwrap();
    assert_eq!(effective["profile_id"], serde_json::json!("s07-chain"));
    assert!(effective["effective"]["skills"]["value"]["enabled"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("dev-sprint")));
}

#[tokio::test]
async fn handle_delete_mcp_connector_rejects_inherited_connector() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut parent = custom_profile("mcp-parent", "MCP Parent");
    parent
        .mcp
        .connectors
        .insert("github".to_string(), test_mcp_connector());
    let _ = handle_create_profile(Json(parent)).await.unwrap();
    let mut child = custom_profile("mcp-child", "MCP Child");
    child.extends_profile_id = Some("mcp-parent".to_string());
    let _ = handle_create_profile(Json(child)).await.unwrap();

    let err = handle_delete_mcp_connector(
        Path("github".to_string()),
        Query(McpConnectorsQuery {
            profile: Some("mcp-child".to_string()),
        }),
    )
    .await
    .expect_err("inherited MCP server delete should fail closed");

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("server_is_locked"));
}

#[tokio::test]
async fn handle_get_rule_returns_single_rule_with_provenance() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("custom", "Custom");
    profile.security.rules.http.insert(
        "block_openai".to_string(),
        test_profile_rule(
            "http.request",
            "request.host == 'api.openai.com'",
            capsem_core::settings_profiles::RuleDecision::Block,
            25,
            "test block",
        ),
    );
    let _ = handle_create_profile(Json(profile)).await.unwrap();

    let Json(val) = handle_get_rule(Path("security.rules.http.block_openai".to_string()))
        .await
        .unwrap();

    assert_eq!(
        val["id"],
        serde_json::json!("security.rules.http.block_openai")
    );
    assert_eq!(val["effective_id"], serde_json::json!("http.block_openai"));
    assert_eq!(val["provenance"]["profile_id"], serde_json::json!("custom"));
    assert_eq!(
        val["provenance"]["toml_path"],
        serde_json::json!("security.rules.http.block_openai")
    );
}

#[tokio::test]
async fn handle_evaluate_rule_dry_runs_v2_policy_without_enforcement() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("custom", "Custom");
    profile.security.rules.http.insert(
        "ask_openai".to_string(),
        test_profile_rule(
            "http.request",
            "request.host == 'api.openai.com'",
            capsem_core::settings_profiles::RuleDecision::Ask,
            25,
            "needs review",
        ),
    );
    let _ = handle_create_profile(Json(profile)).await.unwrap();

    let Json(val) = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: Some("custom".to_string()),
        callback: "http.request".to_string(),
        subject: serde_json::json!({
            "request": {
                "host": "api.openai.com",
                "method": "POST"
            }
        }),
    }))
    .await
    .unwrap();

    assert_eq!(
        val["matched_rule_id"],
        serde_json::json!("security.rules.http.ask_openai")
    );
    assert_eq!(val["decision"], serde_json::json!("ask"));
    assert_eq!(val["would_ask"], serde_json::json!(true));
    assert_eq!(val["reason"], serde_json::json!("needs review"));
    assert_eq!(val["enforced"], serde_json::json!(false));
}

#[tokio::test]
async fn rules_api_functional_chain_reloads_profile_changes_across_calls() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("chain", "Chain");
    profile.security.rules.http.insert(
        "ask_openai".to_string(),
        test_profile_rule(
            "http.request",
            "request.host == 'api.openai.com'",
            capsem_core::settings_profiles::RuleDecision::Ask,
            20,
            "review OpenAI access",
        ),
    );
    profile.security.rules.http.insert(
        "allow_github".to_string(),
        test_profile_rule(
            "http.request",
            "request.host == 'github.com'",
            capsem_core::settings_profiles::RuleDecision::Allow,
            30,
            "allow GitHub",
        ),
    );

    let Json(created) = handle_create_profile(Json(profile)).await.unwrap();
    assert_eq!(created["profile"]["id"], serde_json::json!("chain"));

    let Json(profiles) = handle_list_profiles().await.unwrap();
    assert!(profiles["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["profile"]["id"] == serde_json::json!("chain")));

    let Json(listed) = handle_list_rules(Query(RulesQuery {
        profile: Some("chain".to_string()),
        callback: Some("http.request".to_string()),
    }))
    .await
    .unwrap();
    let listed_rules = listed["rules"].as_array().expect("rules array");
    assert!(listed_rules
        .iter()
        .any(|rule| rule["id"] == serde_json::json!("security.rules.http.ask_openai")));
    assert!(listed_rules
        .iter()
        .any(|rule| rule["id"] == serde_json::json!("security.rules.http.allow_github")));

    let Json(rule) = handle_get_rule(Path("security.rules.http.ask_openai".to_string()))
        .await
        .unwrap();
    assert_eq!(rule["source_profile"], serde_json::json!("chain"));
    assert_eq!(rule["rule"]["decision"], serde_json::json!("ask"));

    let subject = serde_json::json!({
        "request": {
            "host": "api.openai.com",
            "method": "GET"
        }
    });
    let Json(before_update) = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: Some("chain".to_string()),
        callback: "http.request".to_string(),
        subject: subject.clone(),
    }))
    .await
    .unwrap();
    assert_eq!(
        before_update["matched_rule_id"],
        serde_json::json!("security.rules.http.ask_openai")
    );
    assert_eq!(before_update["would_ask"], serde_json::json!(true));

    let mut updated = custom_profile("chain", "Chain");
    updated.security.rules.http.insert(
        "block_openai".to_string(),
        test_profile_rule(
            "http.request",
            "request.host == 'api.openai.com'",
            capsem_core::settings_profiles::RuleDecision::Block,
            5,
            "tightened during same workflow",
        ),
    );
    let _ = handle_update_profile(Path("chain".to_string()), Json(updated))
        .await
        .unwrap();

    let Json(after_update) = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: Some("chain".to_string()),
        callback: "http.request".to_string(),
        subject,
    }))
    .await
    .unwrap();
    assert_eq!(
        after_update["matched_rule_id"],
        serde_json::json!("security.rules.http.block_openai")
    );
    assert_eq!(after_update["decision"], serde_json::json!("block"));
    assert_eq!(after_update["would_ask"], serde_json::json!(false));
}

#[tokio::test]
async fn rules_api_create_delete_roundtrip_updates_user_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let _ = handle_create_profile(Json(custom_profile("rules-user", "Rules User")))
        .await
        .unwrap();

    let Json(created) = handle_create_rule(Json(RuleCreateRequest {
        profile: Some("rules-user".to_string()),
        id: "security.rules.http.ask_openai".to_string(),
        update: PolicyRuleUpdate {
            callback: "http.request".to_string(),
            condition: "request.host == 'api.openai.com'".to_string(),
            decision: capsem_core::settings_profiles::RuleDecision::Ask,
            priority: 20,
            reason: Some("review OpenAI access".to_string()),
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    }))
    .await
    .unwrap();

    assert_eq!(
        created["id"],
        serde_json::json!("security.rules.http.ask_openai")
    );
    assert_eq!(created["source_profile"], serde_json::json!("rules-user"));
    assert_eq!(created["rule"]["decision"], serde_json::json!("ask"));

    let subject = serde_json::json!({
        "request": {
            "host": "api.openai.com",
            "method": "GET"
        }
    });
    let Json(evaluated) = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: Some("rules-user".to_string()),
        callback: "http.request".to_string(),
        subject: subject.clone(),
    }))
    .await
    .unwrap();
    assert_eq!(
        evaluated["matched_rule_id"],
        serde_json::json!("security.rules.http.ask_openai")
    );

    let Json(deleted) = handle_delete_rule(
        Path("security.rules.http.ask_openai".to_string()),
        Query(RulesMutationQuery {
            profile: Some("rules-user".to_string()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        deleted["rule_id"],
        serde_json::json!("security.rules.http.ask_openai")
    );
    assert_eq!(deleted["removed"], serde_json::json!(true));

    let Json(after_delete) = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: Some("rules-user".to_string()),
        callback: "http.request".to_string(),
        subject,
    }))
    .await
    .unwrap();
    assert!(after_delete["matched_rule_id"].is_null());
}

#[tokio::test]
async fn handle_delete_rule_rejects_locked_profile_rule() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let err = handle_delete_rule(
        Path("security.rules.http.default_read".to_string()),
        Query(RulesMutationQuery { profile: None }),
    )
    .await
    .expect_err("default built-in rule deletion should fail closed");

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("rule_is_builtin"));
}

#[tokio::test]
async fn handle_create_rule_materializes_default_builtin_profile_override() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, user_profile_path) = install_settings_profiles_env(&dir);

    assert!(!user_profile_path.exists());
    let Json(created) = handle_create_rule(Json(RuleCreateRequest {
        profile: None,
        id: "security.rules.http.ask_probe".to_string(),
        update: PolicyRuleUpdate {
            callback: "http.request".to_string(),
            condition: "request.host == 'probe.example.com'".to_string(),
            decision: capsem_core::settings_profiles::RuleDecision::Ask,
            priority: 20,
            reason: Some("probe approval".to_string()),
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    }))
    .await
    .unwrap();

    assert_eq!(
        created["id"],
        serde_json::json!("security.rules.http.ask_probe")
    );
    assert!(user_profile_path.exists());
    let text = std::fs::read_to_string(user_profile_path).unwrap();
    assert!(text.contains("[security.rules.http.ask_probe]"));
}

#[tokio::test]
async fn handle_create_rule_rejects_duplicate_user_rule() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("rules-user", "Rules User");
    profile.security.rules.http.insert(
        "ask_openai".to_string(),
        test_profile_rule(
            "http.request",
            "request.host == 'api.openai.com'",
            capsem_core::settings_profiles::RuleDecision::Ask,
            20,
            "review OpenAI access",
        ),
    );
    let _ = handle_create_profile(Json(profile)).await.unwrap();

    let err = handle_create_rule(Json(RuleCreateRequest {
        profile: Some("rules-user".to_string()),
        id: "security.rules.http.ask_openai".to_string(),
        update: PolicyRuleUpdate {
            callback: "http.request".to_string(),
            condition: "request.host == 'api.openai.com'".to_string(),
            decision: capsem_core::settings_profiles::RuleDecision::Ask,
            priority: 20,
            reason: Some("review OpenAI access".to_string()),
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    }))
    .await
    .expect_err("duplicate rule create should fail closed");

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(err.1.contains("rule_exists"));
}

#[tokio::test]
async fn handle_evaluate_rule_supports_generated_http_read_write_callbacks() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let subject = serde_json::json!({
        "request": {
            "host": "example.com",
            "method": "GET"
        }
    });
    let Json(read) = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: None,
        callback: "http.read".to_string(),
        subject: subject.clone(),
    }))
    .await
    .unwrap();
    assert_eq!(
        read["matched_rule_id"],
        serde_json::json!("security.rules.http.default_read")
    );
    assert!(read["decision"].is_string());

    let Json(write) = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: None,
        callback: "http.write".to_string(),
        subject,
    }))
    .await
    .unwrap();
    assert_eq!(
        write["matched_rule_id"],
        serde_json::json!("security.rules.http.default_write")
    );
    assert!(write["decision"].is_string());
}

#[tokio::test]
async fn handle_evaluate_rule_rejects_unknown_callback() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let err = handle_evaluate_rule(Json(RuleEvaluateRequest {
        profile: None,
        callback: "http.connect".to_string(),
        subject: serde_json::json!({
            "request": {
                "host": "api.openai.com"
            }
        }),
    }))
    .await
    .expect_err("unsupported evaluator callback should fail closed");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("unsupported policy callback"));
}

#[tokio::test]
async fn rules_api_evaluate_stays_bounded_for_large_profiles() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let mut profile = custom_profile("large", "Large");
    for index in 0..160 {
        profile.security.rules.http.insert(
            format!("miss_{index:03}"),
            test_profile_rule(
                "http.request",
                &format!("request.host == 'miss-{index}.example.com'"),
                capsem_core::settings_profiles::RuleDecision::Block,
                index + 1,
                "bulk miss",
            ),
        );
    }
    profile.security.rules.http.insert(
        "target".to_string(),
        test_profile_rule(
            "http.request",
            "request.host == 'target.example.com'",
            capsem_core::settings_profiles::RuleDecision::Block,
            900,
            "bulk target",
        ),
    );
    let _ = handle_create_profile(Json(profile)).await.unwrap();

    let request = RuleEvaluateRequest {
        profile: Some("large".to_string()),
        callback: "http.request".to_string(),
        subject: serde_json::json!({
            "request": {
                "host": "target.example.com",
                "method": "GET"
            }
        }),
    };
    let Json(warmup) = handle_evaluate_rule(Json(request.clone())).await.unwrap();
    assert_eq!(
        warmup["matched_rule_id"],
        serde_json::json!("security.rules.http.target")
    );

    let iterations = 32;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let Json(result) = handle_evaluate_rule(Json(request.clone())).await.unwrap();
        assert_eq!(
            result["matched_rule_id"],
            serde_json::json!("security.rules.http.target")
        );
    }
    let elapsed = start.elapsed();
    let budget = std::time::Duration::from_millis(1500);
    assert!(
        elapsed < budget,
        "{iterations} large-profile rule evaluations took {elapsed:?}, budget {budget:?}"
    );
}

#[tokio::test]
async fn handle_lint_config_returns_array() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_settings_profiles_env(&dir);

    let Json(val) = handle_lint_config().await;
    assert!(val.is_array(), "lint response should be an array");
}

#[tokio::test]
async fn handle_save_settings_rejects_unknown_key() {
    let mut changes = HashMap::new();
    changes.insert("nonexistent.setting.xyz".into(), serde_json::json!("value"));
    let result = handle_save_settings(Json(changes)).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn handle_save_settings_accepts_policy_rule_object() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, service_path, user_profile_path) = install_settings_profiles_env(&dir);

    let mut changes = HashMap::new();
    changes.insert(
        "policy.http.block_openai_github".into(),
        serde_json::json!({
            "on": "http.request",
            "if": "request.host == 'github.com' && request.path.matches('^/openai(/|$)')",
            "decision": "block",
            "priority": 10,
            "reason": "Do not let this session fetch OpenAI-owned GitHub code"
        }),
    );

    let result = handle_save_settings(Json(changes)).await;

    let Json(val) = result.expect("policy rule save should succeed");
    assert_eq!(
        val["effective_rules"]["http"]["block_openai_github"]["priority"],
        serde_json::json!(10)
    );
    assert!(service_path.exists());
    let profile_text = std::fs::read_to_string(&user_profile_path).unwrap();
    assert!(profile_text.contains("[security.rules.http.block_openai_github]"));
}

#[tokio::test]
async fn handle_save_settings_accepts_mcp_policy_rule_object() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, user_profile_path) = install_settings_profiles_env(&dir);

    let mut changes = HashMap::new();
    changes.insert(
        "policy.mcp.block_prod_token".into(),
        serde_json::json!({
            "on": "mcp.request",
            "if": "method == 'tools/call' && tool.name == 'local__echo' && has(arguments.prod_token)",
            "decision": "block",
            "priority": 10,
            "reason": "Do not send production tokens to MCP tools"
        }),
    );

    let result = handle_save_settings(Json(changes)).await;

    let Json(val) = result.expect("MCP policy rule save should succeed");
    assert_eq!(
        val["effective_rules"]["mcp"]["block_prod_token"]["decision"],
        serde_json::json!("block")
    );
    let profile_text = std::fs::read_to_string(&user_profile_path).unwrap();
    assert!(profile_text.contains("[security.rules.mcp.block_prod_token]"));
}

#[tokio::test]
async fn handle_save_settings_accepts_model_policy_rule_object() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, user_profile_path) = install_settings_profiles_env(&dir);

    let mut changes = HashMap::new();
    changes.insert(
        "policy.model.block_secret_prompt".into(),
        serde_json::json!({
            "on": "model.request",
            "if": "provider == 'openai' && model == 'gpt-4o-mini' && request.data.contains('prod-secret')",
            "decision": "block",
            "priority": 10,
            "reason": "Keep secret-bearing prompts local"
        }),
    );

    let result = handle_save_settings(Json(changes)).await;

    let Json(val) = result.expect("model policy rule save should succeed");
    assert_eq!(
        val["effective_rules"]["model"]["block_secret_prompt"]["decision"],
        serde_json::json!("block")
    );
    let profile_text = std::fs::read_to_string(&user_profile_path).unwrap();
    assert!(profile_text.contains("[security.rules.model.block_secret_prompt]"));
}

#[tokio::test]
async fn handle_save_settings_rejects_policy_rule_callback_mismatch() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, user_profile_path) = install_settings_profiles_env(&dir);

    let mut changes = HashMap::new();
    changes.insert(
        "policy.model.bad_callback".into(),
        serde_json::json!({
            "on": "http.request",
            "if": "request.host == 'api.openai.com'",
            "decision": "block",
            "priority": 10
        }),
    );

    let err = handle_save_settings(Json(changes))
        .await
        .expect_err("wrong callback type should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1.contains("uses callback for a different policy type"),
        "error should explain callback mismatch, got: {}",
        err.1
    );
    assert!(
        !user_profile_path.exists(),
        "rejected model policy update must not create user profile override"
    );
}

#[tokio::test]
async fn handle_save_settings_rejects_invalid_policy_condition() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, user_profile_path) = install_settings_profiles_env(&dir);

    let mut changes = HashMap::new();
    changes.insert(
        "policy.http.bad_condition".into(),
        serde_json::json!({
            "on": "http.request",
            "if": "request.path.match('^/openai')",
            "decision": "block",
            "priority": 10
        }),
    );

    let err = handle_save_settings(Json(changes))
        .await
        .expect_err("invalid CEL condition should be rejected by settings handler");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1.contains("unsupported CEL condition term"),
        "error should explain CEL validation failure, got: {}",
        err.1
    );
    assert!(
        !user_profile_path.exists(),
        "rejected policy update must not create user profile override"
    );
}

fn make_test_state_with_tempdir_at(
    dir: tempfile::TempDir,
) -> (Arc<ServiceState>, tempfile::TempDir) {
    let run_dir = dir.path().join("run");
    let registry_path = run_dir.join("persistent_registry.json");
    let assets_dir = run_dir.join("assets");
    let current_version = "0.0.0";
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: assets_dir.clone(),
        asset_locations: test_asset_locations(assets_dir.clone()),
        service_settings: test_service_settings(&run_dir),
        run_dir,
        job_counter: AtomicU64::new(1),
        asset_supervisor: test_asset_supervisor(assets_dir),
        current_version: current_version.into(),
        magika: test_magika(),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });
    (state, dir)
}

// -----------------------------------------------------------------------
// resolve_workspace_path
// -----------------------------------------------------------------------

#[test]
fn resolve_rejects_unknown_vm() {
    let state = make_test_state();
    let r = resolve_workspace_path(&state, "nonexistent", "src/main.rs");
    assert!(r.is_err());
}

#[test]
fn resolve_rejects_symlink_escape() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    // Create a symlink that points outside workspace
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(&outside, workspace.join("escape")).unwrap();

    let (state, _dir2) = make_test_state_with_tempdir();
    state.instances.lock().unwrap().insert(
        "test-vm".into(),
        InstanceInfo {
            id: "test-vm".into(),
            pid: 1,
            uds_path: PathBuf::from("/tmp/test.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: None,
            profile_pin: None,
        },
    );

    let r = resolve_workspace_path(&state, "test-vm", "escape/secret.txt");
    assert!(r.is_err());
}

#[test]
fn resolve_valid_path_inside_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("hello.txt"), "world").unwrap();

    let (state, _dir2) = make_test_state_with_tempdir();
    state.instances.lock().unwrap().insert(
        "test-vm".into(),
        InstanceInfo {
            id: "test-vm".into(),
            pid: 1,
            uds_path: PathBuf::from("/tmp/test.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: None,
            profile_pin: None,
        },
    );

    let r = resolve_workspace_path(&state, "test-vm", "hello.txt");
    assert!(r.is_ok());
    let (ws_root, resolved) = r.unwrap();
    assert!(resolved.starts_with(ws_root.canonicalize().unwrap()));
}

// -----------------------------------------------------------------------
// list_dir_recursive
// -----------------------------------------------------------------------

#[test]
fn list_dir_returns_correct_structure() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(ws.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(ws.join("README.md"), "# Hello").unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 2, &magika);

    // Should have src/ dir and README.md file
    assert!(entries.len() >= 2);
    let dir_entry = entries.iter().find(|e| e.name == "src").unwrap();
    assert_eq!(dir_entry.entry_type, "directory");
    assert!(dir_entry.children.is_some());
    let children = dir_entry.children.as_ref().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "main.rs");
    assert_eq!(children[0].entry_type, "file");

    let file_entry = entries.iter().find(|e| e.name == "README.md").unwrap();
    assert_eq!(file_entry.entry_type, "file");
    assert!(file_entry.size > 0);
}

#[test]
fn list_dir_respects_depth_limit() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("a/b/c")).unwrap();
    std::fs::write(ws.join("a/b/c/deep.txt"), "deep").unwrap();

    let magika = test_magika();
    // depth 1: should list "a" but not recurse into "a/b"
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    let a = entries.iter().find(|e| e.name == "a").unwrap();
    assert!(a.children.is_none());
}

#[test]
fn list_dir_skips_system_but_shows_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join(".hidden")).unwrap();
    std::fs::create_dir_all(ws.join("system")).unwrap();
    std::fs::write(ws.join("visible.txt"), "yes").unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    // .hidden + visible.txt shown; system/ filtered out
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e.name == ".hidden"));
    assert!(entries.iter().any(|e| e.name == "visible.txt"));
    assert!(!entries.iter().any(|e| e.name == "system"));
}

#[test]
fn list_dir_sorts_dirs_first_then_alphabetical() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("zebra.txt"), "z").unwrap();
    std::fs::create_dir_all(ws.join("alpha")).unwrap();
    std::fs::write(ws.join("apple.txt"), "a").unwrap();
    std::fs::create_dir_all(ws.join("beta")).unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    // Dirs first (alpha, beta), then files (apple.txt, zebra.txt)
    assert_eq!(entries[0].name, "alpha");
    assert_eq!(entries[1].name, "beta");
    assert_eq!(entries[2].name, "apple.txt");
    assert_eq!(entries[3].name, "zebra.txt");
}

// -----------------------------------------------------------------------
// Download / Upload via resolve_workspace_path
// -----------------------------------------------------------------------

fn setup_vm_with_workspace(state: &ServiceState, dir: &std::path::Path, vm_id: &str) {
    let session_dir = dir.join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    state.instances.lock().unwrap().insert(
        vm_id.into(),
        InstanceInfo {
            id: vm_id.into(),
            pid: 1,
            uds_path: PathBuf::from("/tmp/test.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: None,
            profile_pin: None,
        },
    );
}

#[test]
fn download_reads_correct_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "dl-vm");

    let ws = dir.path().join("session/guest/workspace");
    let content = b"hello world\nline 2\n";
    std::fs::write(ws.join("test.txt"), content).unwrap();

    let (_, resolved) = resolve_workspace_path(&state, "dl-vm", "test.txt").unwrap();
    let data = std::fs::read(&resolved).unwrap();
    assert_eq!(data, content);
}

#[test]
fn download_binary_preserves_content() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "bin-vm");

    let ws = dir.path().join("session/guest/workspace");
    let binary: Vec<u8> = (0..256).map(|i| i as u8).collect();
    std::fs::write(ws.join("data.bin"), &binary).unwrap();

    let (_, resolved) = resolve_workspace_path(&state, "bin-vm", "data.bin").unwrap();
    let data = std::fs::read(&resolved).unwrap();
    assert_eq!(data, binary);
}

#[test]
fn upload_creates_file_with_content() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "up-vm");

    let ws = dir.path().join("session/guest/workspace");
    let (_, target) = resolve_workspace_path(&state, "up-vm", "new.txt").unwrap();
    std::fs::write(&target, b"uploaded").unwrap();

    assert_eq!(
        std::fs::read_to_string(ws.join("new.txt")).unwrap(),
        "uploaded"
    );
}

#[test]
fn upload_creates_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "mkdir-vm");

    let ws = dir.path().join("session/guest/workspace");
    // resolve_workspace_path should succeed even for non-existing nested paths
    let (_, target) = resolve_workspace_path(&state, "mkdir-vm", "deep/nested/file.txt").unwrap();
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"deep content").unwrap();

    assert_eq!(
        std::fs::read_to_string(ws.join("deep/nested/file.txt")).unwrap(),
        "deep content"
    );
}

#[test]
fn upload_path_traversal_blocked() {
    let r = sanitize_file_path("../../etc/passwd");
    assert!(r.is_err());
}

#[test]
fn download_nonexistent_file_resolve_ok_but_not_exists() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "404-vm");

    // Resolving a non-existent file path still works (for upload target)
    let result = resolve_workspace_path(&state, "404-vm", "nonexistent.txt");
    assert!(result.is_ok());
    let (_, resolved) = result.unwrap();
    assert!(!resolved.exists());
}

// is_launchd_cleanup_transient identifies the misleading "missing
// entitlement" NSError that VZ emits when launchd's PETRIFIED-cleanup
// queue is saturated under rapid VM churn. The error string is
// stable across VZ releases (Apple's localizedDescription); pattern-
// match conservatively so a real codesign regression doesn't get
// silently retried.
#[test]
fn launchd_transient_matches_actual_vz_entitlement_error() {
    let tail = "Error: failed to boot VM\n\nCaused by:\n    \
        VM config validation failed: NSError { code: 2, \
        localizedDescription: \"Invalid virtual machine configuration. \
        The process doesn't have the \u{201c}com.apple.security.\
        virtualization\u{201d} entitlement.\", domain: \"VZErrorDomain\", \
        userInfo: {} }";
    assert!(is_launchd_cleanup_transient(tail));
}

#[test]
fn launchd_transient_matches_straight_quote_variant() {
    // Same content with ASCII quotes around the entitlement key.
    let tail = "VM config validation failed: NSError { code: 2, \
        localizedDescription: \"...The process doesn't have the \
        \\\"com.apple.security.virtualization\\\" entitlement.\" }";
    assert!(is_launchd_cleanup_transient(tail));
}

#[test]
fn launchd_transient_rejects_other_failures() {
    let unrelated = "Error: failed to build VmConfig\n\nCaused by:\n    \
        hash mismatch for ...img: expected abc, got def";
    assert!(!is_launchd_cleanup_transient(unrelated));

    let no_log = "(no preserved log found)";
    assert!(!is_launchd_cleanup_transient(no_log));

    let empty = "";
    assert!(!is_launchd_cleanup_transient(empty));
}

#[test]
fn launchd_transient_rejects_partial_match() {
    // The word "entitlement" alone in some unrelated error must not match;
    // the matcher requires the full VZ-specific phrase.
    let mention_only = "warn: this command may need an entitlement";
    assert!(!is_launchd_cleanup_transient(mention_only));
}

// classify_attempt_decision is the pure routing function the
// poll_until-based retry loop in handle_provision delegates to.
// Testing it directly lets us prove the retry path engages on the
// LaunchdTransient outcome (the actual fix for Bug A) without
// spawning a real VM. If a future refactor breaks the routing
// (e.g., maps LaunchdTransient to BailWithError), these fail.

fn test_provision_asset_health() -> AssetHealth {
    AssetHealth {
        ready: true,
        state: AssetHealthState::Ready,
        profile_id: Some("everyday-work".into()),
        profile_revision: Some("2026.0520.1".into()),
        profile_payload_hash: Some(format!("blake3:{}", "e".repeat(64))),
        profile_assets: Vec::new(),
        version: Some("everyday-work@2026.0520.1".into()),
        arch: Some("arm64".into()),
        missing: Vec::new(),
        progress: None,
        error: None,
        retry_count: 0,
        retryable: false,
        saved_vm_dependencies: Vec::new(),
        checked_at_unix_secs: Some(1_779_264_000),
    }
}

#[test]
fn classify_ready_outcome_succeeds() {
    let uds = PathBuf::from("/tmp/x.sock");
    let health = test_provision_asset_health();
    match classify_attempt_decision(
        ProvisionAttemptOutcome::Ready {
            uds_path: uds.clone(),
            asset_health: health.clone(),
        },
        "vm-1",
    ) {
        AttemptDecision::Succeed {
            uds_path,
            asset_health,
        } => {
            assert_eq!(uds_path, uds);
            assert_eq!(*asset_health, health);
        }
        other => panic!("expected Succeed, got {other:?}"),
    }
}

#[test]
fn classify_still_booting_timeout_succeeds_with_uds() {
    let uds = PathBuf::from("/tmp/y.sock");
    let health = test_provision_asset_health();
    match classify_attempt_decision(
        ProvisionAttemptOutcome::StillBootingTimedOut {
            uds_path: uds.clone(),
            asset_health: health.clone(),
        },
        "vm-2",
    ) {
        AttemptDecision::Succeed {
            uds_path,
            asset_health,
        } => {
            assert_eq!(uds_path, uds);
            assert_eq!(*asset_health, health);
        }
        other => panic!("expected Succeed for still-booting envelope, got {other:?}"),
    }
}

#[test]
fn classify_launchd_transient_routes_to_retry() {
    // The core of the Bug A fix: LaunchdTransient must trigger a retry,
    // not bail with the misleading entitlement error.
    match classify_attempt_decision(ProvisionAttemptOutcome::LaunchdTransient, "vm-3") {
        AttemptDecision::RetryAfterCleanup => {}
        other => panic!("expected RetryAfterCleanup for LaunchdTransient, got {other:?}"),
    }
}

#[test]
fn classify_boot_crash_bails_with_500_and_tail() {
    let tail = "Error: failed to boot VM\n\nCaused by:\n    bogus".to_string();
    match classify_attempt_decision(
        ProvisionAttemptOutcome::BootCrash { tail: tail.clone() },
        "vm-4",
    ) {
        AttemptDecision::BailWithError(AppError(status, msg)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert!(msg.contains("vm-4"), "msg should embed the id: {msg}");
            assert!(msg.contains(&tail), "msg should embed the log tail: {msg}");
            assert!(
                msg.contains("capsem logs vm-4"),
                "msg should hint at follow-up cmd"
            );
        }
        other => panic!("expected BailWithError(500), got {other:?}"),
    }
}

#[test]
fn classify_provision_error_already_exists_returns_409() {
    let err = anyhow::anyhow!("persistent VM \"vm-5\" already exists. Use `capsem resume vm-5`.");
    match classify_attempt_decision(ProvisionAttemptOutcome::ProvisionError(err), "vm-5") {
        AttemptDecision::BailWithError(AppError(status, _)) => {
            assert_eq!(
                status,
                StatusCode::CONFLICT,
                "duplicate-name errors must return 409 so clients can distinguish from server failures"
            );
        }
        other => panic!("expected BailWithError(409) for already-exists, got {other:?}"),
    }
}

#[test]
fn classify_provision_error_other_returns_500() {
    let err = anyhow::anyhow!("rootfs not found at /missing/path");
    match classify_attempt_decision(ProvisionAttemptOutcome::ProvisionError(err), "vm-6") {
        AttemptDecision::BailWithError(AppError(status, msg)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert!(
                msg.contains("rootfs not found"),
                "underlying error preserved: {msg}"
            );
        }
        other => panic!("expected BailWithError(500), got {other:?}"),
    }
}

// wait_for_vm_ready polls a cheap local sentinel file. Typical VM boot
// ready-time is sub-second, so the backoff must not overshoot readiness
// by hundreds of ms -- that shows up directly in provision->exec latency.
#[tokio::test]
async fn wait_for_vm_ready_detects_ready_within_tight_overshoot() {
    let dir = tempfile::tempdir().unwrap();
    let uds_path = dir.path().join("vm.sock");
    let ready_path = uds_path.with_extension("ready");

    // Simulate a VM that becomes ready ~200ms after provision. Real VM
    // boots land in the 400-700ms range, so 200ms is a conservative stand-in.
    let ready_clone = ready_path.clone();
    let creator = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        std::fs::write(&ready_clone, b"").unwrap();
    });

    let start = std::time::Instant::now();
    wait_for_vm_ready(&uds_path, 30, None, None)
        .await
        .expect("ready should be detected");
    let elapsed_ms = start.elapsed().as_millis();
    creator.await.unwrap();

    // Overshoot budget: a tight poll curve should catch the sentinel
    // within ~100ms of it appearing. A 500ms max_delay would miss the
    // 200ms creation and catch it at ~350ms instead.
    assert!(
        elapsed_ms < 300,
        "wait_for_vm_ready overshot: {elapsed_ms}ms (ready created at ~200ms, budget 300ms)"
    );
}
