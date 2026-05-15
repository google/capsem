use super::*;
use std::sync::atomic::AtomicU64;

static SETTINGS_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const UNSIGNED_MANIFEST: &str = r#"{
    "format": 2,
    "assets": {
        "current": "2026.0415.1",
        "releases": {
            "2026.0415.1": {
                "date": "2026-04-15",
                "deprecated": false,
                "min_binary": "1.0.0",
                "arches": {
                    "arm64": {
                        "vmlinuz": { "hash": "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c", "size": 7797248 },
                        "initrd.img": { "hash": "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456", "size": 2270154 },
                        "rootfs.squashfs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }
                    }
                }
            }
        }
    },
    "binaries": {
        "current": "1.0.1776269479",
        "releases": {
            "1.0.1776269479": {
                "date": "2026-04-15",
                "deprecated": false,
                "min_assets": "2026.0415.1"
            }
        }
    }
}"#;

#[test]
fn startup_manifest_loader_rejects_unsigned_manifest() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("manifest.json"), UNSIGNED_MANIFEST).unwrap();

    let err = load_startup_manifest_for_assets(dir.path()).unwrap_err();
    assert!(
        format!("{err:#}").contains("signature missing"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn startup_manifest_loader_rejects_invalid_signature() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("manifest.json"), UNSIGNED_MANIFEST).unwrap();
    std::fs::write(dir.path().join("manifest.json.minisig"), "not a signature").unwrap();

    let err = load_startup_manifest_for_assets(dir.path()).unwrap_err();
    assert!(
        format!("{err:#}").contains("verify"),
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
async fn triage_session_db_surfaces_policy_v2_signals() {
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
fn timeline_allowed_layers_include_policy_v2_tables() {
    for expected in ["dns", "hook", "audit", "snapshot"] {
        assert!(
            ALLOWED_TIMELINE_LAYERS.contains(&expected),
            "timeline layer allowlist missing {expected}"
        );
    }
}

#[test]
fn timeline_existing_tables_lists_policy_v2_tables() {
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
async fn timeline_handler_returns_policy_v2_layers_and_null_trace_rows() {
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

fn test_asset_supervisor(
    assets_dir: PathBuf,
    manifest: Option<Arc<capsem_core::asset_manager::ManifestV2>>,
    current_version: &str,
) -> Arc<AssetSupervisor> {
    Arc::new(AssetSupervisor::new(
        assets_dir,
        manifest,
        current_version.to_string(),
        host_asset_arch().to_string(),
        std::time::Duration::from_secs(60),
    ))
}

fn make_test_state() -> Arc<ServiceState> {
    let registry_path = PathBuf::from("/tmp/capsem-test-svc/persistent_registry.json");
    let assets_dir = PathBuf::from("/nonexistent/assets");
    let manifest = None;
    let current_version = "0.0.0";
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: assets_dir.clone(),
        run_dir: PathBuf::from("/tmp/capsem-test-svc"),
        job_counter: AtomicU64::new(1),
        manifest: manifest.clone(),
        asset_supervisor: test_asset_supervisor(assets_dir, manifest, current_version),
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
    assert!(report.text.contains("manifest_present: false"));
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

#[test]
fn saved_vm_current_base_assets_from_manifest_records_boot_hashes() {
    let manifest = capsem_core::asset_manager::ManifestV2::from_json(UNSIGNED_MANIFEST).unwrap();
    let base_assets =
        saved_vm_assets::current_base_assets(Some(&manifest), "1.0.1776269479", "arm64")
            .unwrap()
            .unwrap();

    assert_eq!(base_assets.asset_version, "2026.0415.1");
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
        registry.data.vms.insert(
            "saved-old".into(),
            PersistentVmEntry {
                name: "saved-old".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                base_assets: Some(test_saved_vm_base_assets()),
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
async fn mcp_refresh_surfaces_process_failure() {
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
                    ServiceToProcess::McpRefreshTools { id } => {
                        tx.send(ProcessToService::McpRefreshResult {
                            id,
                            success: false,
                            error: Some("refresh exploded".into()),
                        })
                        .await
                        .unwrap();
                    }
                    other => panic!("unexpected command: {other:?}"),
                }
            });
    });

    state.instances.lock().unwrap().insert(
        "vm-refresh".to_string(),
        InstanceInfo {
            id: "vm-refresh".to_string(),
            pid: std::process::id(),
            uds_path: sock_path,
            session_dir: dir.path().join("sessions/vm-refresh"),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
            base_assets: None,
        },
    );

    let err = handle_mcp_refresh(State(state)).await.unwrap_err();

    server.join().unwrap();
    assert_eq!(err.0, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        err.1.contains("refresh exploded"),
        "unexpected error body: {}",
        err.1
    );
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
    let manifest = None;
    let current_version = "0.0.0";
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: assets_dir.clone(),
        run_dir,
        job_counter: AtomicU64::new(1),
        manifest: manifest.clone(),
        asset_supervisor: test_asset_supervisor(assets_dir, manifest, current_version),
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
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("must start with") || err.contains("must contain only"),
        "expected name validation error, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Image handler tests (service-level unit tests)
// -----------------------------------------------------------------------

fn make_test_state_with_tempdir() -> (Arc<ServiceState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let registry_path = dir.path().join("persistent_registry.json");
    let assets_dir = dir.path().join("assets");
    let manifest = None;
    let current_version = "0.0.0";
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: assets_dir.clone(),
        run_dir: dir.path().to_path_buf(),
        job_counter: AtomicU64::new(1),
        manifest: manifest.clone(),
        asset_supervisor: test_asset_supervisor(assets_dir, manifest, current_version),
        current_version: current_version.into(),
        magika: test_magika(),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });
    (state, dir)
}

#[tokio::test]
async fn handle_fork_creates_persistent_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    // Create a real session dir for the fake instance
    let session_dir = state.run_dir.join("sessions/fork-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    let base_assets = test_saved_vm_base_assets();
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
            base_assets: None,
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
    let base_assets = test_saved_vm_base_assets();
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
    assert!(info.total_input_tokens.is_none());
    assert!(info.total_estimated_cost.is_none());
    assert!(info.model_call_count.is_none());
    assert!(info.created_at.is_none());
    assert!(info.uptime_secs.is_none());
}

#[test]
fn sandbox_info_telemetry_fields_serialize_when_present() {
    let mut info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
    info.total_input_tokens = Some(1000);
    info.total_estimated_cost = Some(0.42);
    info.model_call_count = Some(5);
    let json = serde_json::to_string(&info).unwrap();
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
}

#[test]
fn sandbox_info_backwards_compatible_deserialization() {
    // Old JSON without telemetry fields should still deserialize
    let json = r#"{"id":"x","pid":1,"status":"Running","persistent":false}"#;
    let info: SandboxInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.id, "x");
    assert!(info.total_input_tokens.is_none());
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
    previous_user: Option<std::ffi::OsString>,
    previous_corp: Option<std::ffi::OsString>,
}

impl Drop for SettingsEnvGuard {
    fn drop(&mut self) {
        if let Some(previous_user) = self.previous_user.take() {
            std::env::set_var("CAPSEM_USER_CONFIG", previous_user);
        } else {
            std::env::remove_var("CAPSEM_USER_CONFIG");
        }

        if let Some(previous_corp) = self.previous_corp.take() {
            std::env::set_var("CAPSEM_CORP_CONFIG", previous_corp);
        } else {
            std::env::remove_var("CAPSEM_CORP_CONFIG");
        }
    }
}

fn install_empty_settings_env(dir: &tempfile::TempDir) -> (SettingsEnvGuard, PathBuf, PathBuf) {
    let user_path = dir.path().join("user.toml");
    let corp_path = dir.path().join("corp.toml");
    capsem_core::net::policy_config::write_settings_file(
        &user_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();
    capsem_core::net::policy_config::write_settings_file(
        &corp_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();

    let guard = SettingsEnvGuard {
        previous_user: std::env::var_os("CAPSEM_USER_CONFIG"),
        previous_corp: std::env::var_os("CAPSEM_CORP_CONFIG"),
    };
    std::env::set_var("CAPSEM_USER_CONFIG", &user_path);
    std::env::set_var("CAPSEM_CORP_CONFIG", &corp_path);
    (guard, user_path, corp_path)
}

#[tokio::test]
async fn handle_get_settings_returns_tree() {
    let Json(val) = handle_get_settings().await.unwrap();
    assert!(val.get("tree").is_some(), "response must have 'tree'");
    assert!(val.get("issues").is_some(), "response must have 'issues'");
    assert!(val.get("presets").is_some(), "response must have 'presets'");
    assert!(val.get("policy").is_some(), "response must have 'policy'");
    assert!(val["tree"].is_array());
    assert!(val["issues"].is_array());
    assert!(val["presets"].is_array());
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
    let Json(val) = handle_get_presets().await.unwrap();
    let arr = val.as_array().expect("presets should be an array");
    assert!(!arr.is_empty(), "should have at least one preset");
    assert!(arr[0].get("id").is_some());
    assert!(arr[0].get("name").is_some());
    assert!(arr[0].get("settings").is_some());
}

#[tokio::test]
async fn handle_lint_config_returns_array() {
    let Json(val) = handle_lint_config().await.unwrap();
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
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);

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
        val["policy"]["http"]["block_openai_github"]["priority"],
        serde_json::json!(10)
    );
    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert!(loaded.policy.http.contains_key("block_openai_github"));
}

#[tokio::test]
async fn handle_save_settings_accepts_mcp_policy_rule_object() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);

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
        val["policy"]["mcp"]["block_prod_token"]["decision"],
        serde_json::json!("block")
    );
    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert!(loaded.policy.mcp.contains_key("block_prod_token"));
}

#[tokio::test]
async fn handle_save_settings_accepts_model_policy_rule_object() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);

    let mut changes = HashMap::new();
    changes.insert(
        "policy.model.block_secret_prompt".into(),
        serde_json::json!({
            "on": "model.request",
            "if": "provider == 'openai' && model == 'gpt-4o-mini' && request.body.contains('prod-secret')",
            "decision": "block",
            "priority": 10,
            "reason": "Keep secret-bearing prompts local"
        }),
    );

    let result = handle_save_settings(Json(changes)).await;

    let Json(val) = result.expect("model policy rule save should succeed");
    assert_eq!(
        val["policy"]["model"]["block_secret_prompt"]["decision"],
        serde_json::json!("block")
    );
    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert!(loaded.policy.model.contains_key("block_secret_prompt"));
}

#[tokio::test]
async fn handle_save_settings_rejects_policy_rule_callback_mismatch() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);

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
    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert!(
        loaded.policy.model.is_empty(),
        "rejected model policy update must not mutate user config"
    );
}

#[tokio::test]
async fn handle_save_settings_rejects_invalid_policy_condition() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);

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
    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert!(
        loaded.policy.http.is_empty(),
        "rejected policy update must not mutate user config"
    );
}

fn make_test_state_with_tempdir_at(
    dir: tempfile::TempDir,
) -> (Arc<ServiceState>, tempfile::TempDir) {
    let run_dir = dir.path().join("run");
    let registry_path = run_dir.join("persistent_registry.json");
    let assets_dir = run_dir.join("assets");
    let manifest = None;
    let current_version = "0.0.0";
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: assets_dir.clone(),
        run_dir,
        job_counter: AtomicU64::new(1),
        manifest: manifest.clone(),
        asset_supervisor: test_asset_supervisor(assets_dir, manifest, current_version),
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

#[test]
fn classify_ready_outcome_succeeds() {
    let uds = PathBuf::from("/tmp/x.sock");
    match classify_attempt_decision(
        ProvisionAttemptOutcome::Ready {
            uds_path: uds.clone(),
        },
        "vm-1",
    ) {
        AttemptDecision::Succeed(p) => assert_eq!(p, uds),
        other => panic!("expected Succeed, got {other:?}"),
    }
}

#[test]
fn classify_still_booting_timeout_succeeds_with_uds() {
    let uds = PathBuf::from("/tmp/y.sock");
    match classify_attempt_decision(
        ProvisionAttemptOutcome::StillBootingTimedOut {
            uds_path: uds.clone(),
        },
        "vm-2",
    ) {
        AttemptDecision::Succeed(p) => assert_eq!(p, uds),
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
