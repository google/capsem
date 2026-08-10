use super::*;

struct InterruptedThenData {
    interrupted: bool,
    data: std::io::Cursor<Vec<u8>>,
}

impl std::io::Read for InterruptedThenData {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        }
        self.data.read(buffer)
    }
}

#[test]
fn exec_output_read_retries_interrupted_socket_reads() {
    let mut reader = InterruptedThenData {
        interrupted: false,
        data: std::io::Cursor::new(b"IRONBANK_CLIENT_RESULT={\"ok\":true}\n".to_vec()),
    };

    let (captured, total) = read_exec_output(&mut reader);

    assert_eq!(captured, b"IRONBANK_CLIENT_RESULT={\"ok\":true}\n");
    assert_eq!(total, captured.len() as u64, "nothing was dropped");
}

// -----------------------------------------------------------------------
// Vsock port classification
// -----------------------------------------------------------------------

#[test]
fn classify_terminal_port() {
    assert_eq!(
        classify_vsock_port(capsem_core::VSOCK_PORT_TERMINAL),
        VsockPortKind::Terminal
    );
}

#[test]
fn classify_control_port() {
    assert_eq!(
        classify_vsock_port(capsem_core::VSOCK_PORT_CONTROL),
        VsockPortKind::Control
    );
}

#[test]
fn classify_sni_proxy_port() {
    assert_eq!(
        classify_vsock_port(capsem_core::VSOCK_PORT_SNI_PROXY),
        VsockPortKind::SniProxy
    );
}

#[test]
fn classify_exec_port() {
    assert_eq!(
        classify_vsock_port(capsem_core::VSOCK_PORT_EXEC),
        VsockPortKind::Exec
    );
}

#[test]
fn classify_lifecycle_port() {
    assert_eq!(
        classify_vsock_port(capsem_core::VSOCK_PORT_LIFECYCLE),
        VsockPortKind::Lifecycle
    );
}

#[test]
fn classify_audit_port() {
    assert_eq!(
        classify_vsock_port(capsem_proto::VSOCK_PORT_AUDIT),
        VsockPortKind::Audit
    );
}

#[test]
fn classify_dns_proxy_port() {
    assert_eq!(
        classify_vsock_port(capsem_proto::VSOCK_PORT_DNS_PROXY),
        VsockPortKind::DnsProxy
    );
}

#[test]
fn classify_unknown_port() {
    assert_eq!(classify_vsock_port(99999), VsockPortKind::Unknown);
}

#[test]
fn classify_port_zero_unknown() {
    assert_eq!(classify_vsock_port(0), VsockPortKind::Unknown);
}

// -----------------------------------------------------------------------
// Handshake retry classification
// -----------------------------------------------------------------------

fn make_conn(port: u32) -> VsockConnection {
    // Dummy fd value (-1) is fine: these tests never read/write the fd,
    // they only exercise the collection and classification logic.
    VsockConnection::new(-1, port, Box::new(()))
}

fn empty_plugin_policy() -> PluginPolicyHandle {
    Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()))
}

fn file_import_event_with_content(content: &str) -> capsem_core::security_engine::SecurityEvent {
    capsem_core::security_engine::SecurityEvent::new(
        capsem_core::security_engine::RuntimeSecurityEventType::FileImport,
    )
    .with_file(capsem_core::security_engine::FileSecurityEvent {
        import_content: Some(content.to_string()),
        ..Default::default()
    })
}

fn add_plugin_rewrite_marker(
    event: &mut capsem_core::security_engine::SecurityEvent,
    plugin_id: &str,
    stage: capsem_core::security_engine::SecurityPluginStage,
) {
    event.record_plugin_execution(capsem_core::security_engine::SecurityPluginExecution {
        plugin_id: plugin_id.to_string(),
        stage,
        applied: true,
        duration_us: 7,
    });
    event.record_detection(capsem_core::security_engine::SecurityDetectionEvent {
        source: capsem_core::security_engine::SecurityDetectionSource::Plugin,
        detection_level: capsem_core::net::policy_config::DetectionLevel::Informational,
        rule_id: None,
        plugin_id: Some(plugin_id.to_string()),
        action: None,
        plugin_mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Rewrite),
        reason: None,
    });
}

#[test]
fn file_boundary_preview_is_not_rewrite_data() {
    let preview = b"x".repeat(FILE_SECURITY_CONTENT_PREVIEW_MAX);
    let preview_text = String::from_utf8(preview.clone()).unwrap();
    let event = file_import_event_with_content(&preview_text);

    assert_eq!(
        rewritten_file_content(&preview, 100_000, &event),
        None,
        "file boundary previews must not truncate larger data-plane payloads"
    );
}

#[test]
fn file_boundary_logging_rewrite_is_not_data_plane_rewrite() {
    let original = b"token=secret";
    let mut event = file_import_event_with_content("token=hash:abc123");
    add_plugin_rewrite_marker(
        &mut event,
        "log_sanitizer",
        capsem_core::security_engine::SecurityPluginStage::Logging,
    );

    assert_eq!(
        rewritten_file_content(original, original.len() as u64, &event),
        None,
        "logging plugins sanitize the ledger and must not rewrite guest bytes"
    );
}

#[test]
fn file_boundary_preprocess_rewrite_changes_complete_payload() {
    let original = b"EICAR";
    let mut event = file_import_event_with_content("CAPSEM_REWRITTEN_EICAR");
    add_plugin_rewrite_marker(
        &mut event,
        "dummy_pre_eicar",
        capsem_core::security_engine::SecurityPluginStage::Preprocess,
    );

    assert_eq!(
        rewritten_file_content(original, original.len() as u64, &event),
        Some(b"CAPSEM_REWRITTEN_EICAR".to_vec())
    );
}

#[test]
fn broken_pipe_is_retryable() {
    let io_err = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
    let err: anyhow::Error = anyhow::Error::new(io_err).context("restore BootConfig write failed");
    assert!(is_retryable_handshake_error(&err));
}

#[test]
fn connection_reset_is_retryable() {
    let io_err = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
    let err: anyhow::Error = anyhow::Error::new(io_err).context("initial Ready read failed");
    assert!(is_retryable_handshake_error(&err));
}

#[test]
fn unexpected_eof_is_retryable() {
    // UnexpectedEof during handshake is the dominant failure mode under
    // heavy suspend/resume churn: Apple VZ tears the post-restoreState
    // vsock conn down between guest frames, so the host's read_exact hits
    // EOF mid-frame. This is the same Apple VZ half-open fingerprint as
    // BrokenPipe / ConnectionReset, just with a clean rather than hard
    // close. Retrying lets the guest's RECONNECT_TIMEOUT_SECS=30 reconnect
    // loop hand us a fresh terminal+control pair within the
    // HANDSHAKE_RETRY_MAX budget.
    let io_err = std::io::Error::from(std::io::ErrorKind::UnexpectedEof);
    let err: anyhow::Error = anyhow::Error::new(io_err).context("BootReady read failed");
    assert!(is_retryable_handshake_error(&err));
}

#[test]
fn decode_error_not_retryable() {
    let err: anyhow::Error = anyhow::anyhow!("malformed control frame");
    assert!(!is_retryable_handshake_error(&err));
}

#[test]
fn not_found_not_retryable() {
    let io_err = std::io::Error::from(std::io::ErrorKind::NotFound);
    let err: anyhow::Error = anyhow::Error::new(io_err).context("unrelated");
    assert!(!is_retryable_handshake_error(&err));
}

// -----------------------------------------------------------------------
// collect_terminal_control_pair
// -----------------------------------------------------------------------

#[tokio::test]
async fn collect_returns_terminal_and_control_in_any_order() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(make_conn(capsem_core::VSOCK_PORT_CONTROL)).unwrap();
    tx.send(make_conn(capsem_core::VSOCK_PORT_TERMINAL))
        .unwrap();

    let mut deferred = Vec::new();
    let (terminal, control) = collect_terminal_control_pair(&mut rx, &mut deferred)
        .await
        .expect("pair collected");
    assert_eq!(terminal.port, capsem_core::VSOCK_PORT_TERMINAL);
    assert_eq!(control.port, capsem_core::VSOCK_PORT_CONTROL);
    assert!(deferred.is_empty());
}

#[tokio::test]
async fn collect_parks_sni_but_ignores_removed_legacy_mcp_port() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(make_conn(capsem_core::VSOCK_PORT_SNI_PROXY))
        .unwrap();
    tx.send(make_conn(5003)).unwrap();
    tx.send(make_conn(capsem_core::VSOCK_PORT_TERMINAL))
        .unwrap();
    tx.send(make_conn(capsem_core::VSOCK_PORT_CONTROL)).unwrap();

    let mut deferred = Vec::new();
    collect_terminal_control_pair(&mut rx, &mut deferred)
        .await
        .expect("pair collected");
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].port, capsem_core::VSOCK_PORT_SNI_PROXY);
    assert_eq!(classify_vsock_port(5003), VsockPortKind::Unknown);
}

#[tokio::test]
async fn collect_errors_when_channel_closes_early() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(make_conn(capsem_core::VSOCK_PORT_TERMINAL))
        .unwrap();
    drop(tx); // close before control arrives

    let mut deferred = Vec::new();
    let err = match collect_terminal_control_pair(&mut rx, &mut deferred).await {
        Ok(_) => panic!("expected error, got pair"),
        Err(e) => e,
    };
    assert!(format!("{err:#}").contains("vsock channel closed"));
}

// -----------------------------------------------------------------------
// handle_guest_msg(ExecDone) must not stall on commands with no stdout.
//
// Prior behavior (bug): a blanket `sleep(500ms)` fired whenever the
// captured buffer was empty, so every no-output command (true, sleep,
// exit, the fsfreeze pipeline used by /fork) paid 500ms of dead time.
// Here the EXEC thread has already deposited its (empty) result before
// ExecDone arrives, which is the common fast path; the handler must
// return immediately.
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_done_with_empty_stdout_resolves_without_500ms_stall() {
    use crate::job_store::{JobResult, JobStore};
    use capsem_proto::GuestToHost;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    let js = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )));
    let plugin_policy = empty_plugin_policy();

    let id: u64 = 42;
    let (tx, rx) = oneshot::channel::<JobResult>();
    js.jobs.lock().unwrap().insert(id, tx);

    // Simulate the dispatch path: the ServiceToProcess::Exec handler has
    // set active_exec, and the EXEC-port reader thread has already
    // deposited its (empty) local_buf and signalled completion. ExecDone
    // arriving after that must return immediately -- no blanket stall.
    let active = crate::job_store::ActiveExec::new(id);
    active.deposited.notify_one();
    *js.active_exec.lock().unwrap() = Some(active);

    let start = std::time::Instant::now();
    handle_guest_msg(
        GuestToHost::ExecDone { id, exit_code: 0 },
        &js,
        &db,
        &security_rules,
        &plugin_policy,
    )
    .await;
    let elapsed_ms = start.elapsed().as_millis();

    assert!(
        elapsed_ms < 100,
        "ExecDone stalled {elapsed_ms}ms on empty-stdout command (budget 100ms)"
    );

    let result = rx.await.expect("job oneshot must resolve");
    match result {
        JobResult::Exec {
            stdout, exit_code, ..
        } => {
            assert!(
                stdout.is_empty(),
                "no-output command should return empty stdout"
            );
            assert_eq!(exit_code, 0);
        }
        other => panic!("expected Exec result, got {other:?}"),
    }
}

#[tokio::test]
async fn exec_done_waits_for_delayed_output_deposit_without_truncation() {
    use crate::job_store::{JobResult, JobStore};
    use capsem_proto::GuestToHost;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    let js = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )));
    let plugin_policy = empty_plugin_policy();

    let id: u64 = 43;
    let (tx, rx) = oneshot::channel::<JobResult>();
    js.jobs.lock().unwrap().insert(id, tx);
    *js.active_exec.lock().unwrap() = Some(crate::job_store::ActiveExec::new(id));

    // Reproduce a loaded runner after resume: the serialized control channel
    // delivers ExecDone promptly, while the dedicated EXEC-port reader does
    // not get scheduled to deposit the already-produced bytes for >100 ms.
    let js_for_deposit = Arc::clone(&js);
    let deposit = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let notify = {
            let mut guard = js_for_deposit.active_exec.lock().unwrap();
            let active = guard
                .as_mut()
                .filter(|active| active.id == id)
                .expect("ExecDone must not discard the capture slot before deposit");
            active.captured = b"/run/capsem-venv\n".to_vec();
            Arc::clone(&active.deposited)
        };
        notify.notify_one();
    });

    handle_guest_msg(
        GuestToHost::ExecDone { id, exit_code: 0 },
        &js,
        &db,
        &security_rules,
        &plugin_policy,
    )
    .await;
    deposit.await.unwrap();

    match rx.await.expect("job oneshot must resolve") {
        JobResult::Exec {
            stdout, exit_code, ..
        } => {
            assert_eq!(stdout, b"/run/capsem-venv\n");
            assert_eq!(exit_code, 0);
        }
        other => panic!("expected Exec result, got {other:?}"),
    }
}

#[tokio::test]
async fn read_file_content_emits_file_export_before_job_result() {
    use capsem_proto::GuestToHost;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(capsem_logger::DbWriter::open(&db_path, 16).unwrap());
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.file_export_seen]
name = "file_export_seen"
action = "allow"
detection_level = "informational"
match = 'file.export.path == "/workspace/out.txt" && file.export.content.contains("guest export")'
"#,
    )
    .expect("rules parse");
    let rules = capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("rules compile");
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(rules)));
    let plugin_policy = empty_plugin_policy();
    let js = Arc::new(JobStore::new());
    let id: u64 = 77;
    js.active_file_ops.lock().unwrap().insert(
        id,
        ActiveFileOp::Read {
            path: "/workspace/out.txt".to_string(),
        },
    );
    let (tx, rx) = oneshot::channel::<JobResult>();
    js.jobs.lock().unwrap().insert(id, tx);

    handle_guest_msg(
        GuestToHost::FileContent {
            id,
            path: "/ignored/guest/path.txt".to_string(),
            data: b"guest export bytes".to_vec(),
        },
        &js,
        &db,
        &security_rules,
        &plugin_policy,
    )
    .await;

    let result = rx.await.expect("read job must resolve");
    match result {
        JobResult::ReadFile {
            data: Some(data), ..
        } => assert_eq!(data, b"guest export bytes"),
        other => panic!("expected read file result with data, got {other:?}"),
    }
    db.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let fs_rows: serde_json::Value = serde_json::from_str(
        &reader
            .query_raw("SELECT action FROM fs_events WHERE path = '/workspace/out.txt'")
            .expect("file event should be written"),
    )
    .unwrap();
    assert_eq!(fs_rows["rows"][0][0].as_str(), Some("export"));
    let rule_rows: serde_json::Value = serde_json::from_str(
        &reader
            .query_raw(
                "SELECT rule_id, event_type FROM security_rule_events WHERE rule_id = 'profiles.rules.file_export_seen'",
            )
            .expect("file export rule event should be written"),
    )
    .unwrap();
    assert_eq!(
        rule_rows["rows"][0][0].as_str(),
        Some("profiles.rules.file_export_seen")
    );
    assert_eq!(rule_rows["rows"][0][1].as_str(), Some("file.export"));
}

#[tokio::test]
async fn dns_security_write_emits_joined_rule_ledger_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(capsem_logger::DbWriter::open(&db_path, 16).unwrap());
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.openai_dns_seen]
name = "openai_dns_seen"
action = "allow"
detection_level = "informational"
match = 'dns.qname == "api.openai.com" && dns.qtype == "1"'
"#,
    )
    .expect("rules parse");
    let rules = capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("rules compile");
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(rules)));
    let event = capsem_logger::DnsEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        qname: "api.openai.com".to_string(),
        qtype: 1,
        qclass: 1,
        rcode: 0,
        answer_ip: Some("93.184.216.34".to_string()),
        decision: "allowed".to_string(),
        matched_rule: None,
        source_proto: Some("udp".to_string()),
        process_name: Some("curl".to_string()),
        upstream_resolver_ms: 0,
        trace_id: Some("trace_dns".to_string()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        credential_ref: None,
    };

    let event_id = emit_dns_security_write_and_rules(&db, &security_rules, event)
        .await
        .expect("event id allocated");
    let db_for_flush = Arc::clone(&db);
    tokio::task::spawn_blocking(move || db_for_flush.shutdown_blocking())
        .await
        .expect("db writer flush task joined");

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let rows: serde_json::Value = serde_json::from_str(
        &reader
            .query_raw(
                "SELECT dns_events.event_id AS dns_event_id, security_rule_events.event_id AS rule_event_id, security_rule_events.rule_id, security_rule_events.detection_level
             FROM dns_events
             JOIN security_rule_events ON security_rule_events.event_id = dns_events.event_id
             WHERE dns_events.qname = 'api.openai.com'",
            )
            .expect("joined DNS rule ledger row"),
    )
    .unwrap();
    let rows = rows["rows"].as_array().expect("joined row array");
    assert_eq!(rows.len(), 1, "expected one joined row, got {rows:?}");
    let row = rows[0].as_array().expect("one joined row");

    assert_eq!(row[0].as_str(), Some(event_id.as_str()));
    assert_eq!(row[1].as_str(), Some(event_id.as_str()));
    assert_eq!(row[2].as_str(), Some("profiles.rules.openai_dns_seen"));
    assert_eq!(row[3].as_str(), Some("informational"));
}

// ── Ack-eligible message sets ──────────────────────────────────────
//
// ackable_id and ackable_response_id decide which messages join the retry /
// replay path: the sender keeps them pending and replays them on every fresh
// control connection until an AckReply arrives. Getting the set wrong is
// silent both ways -- a missing variant is a message that can be lost across
// a reconnect, an extra one is a message replayed forever. Neither had a test.

#[test]
fn every_host_to_guest_side_effect_is_ack_eligible() {
    let cases: Vec<(HostToGuest, u64)> = vec![
        (
            HostToGuest::Exec {
                id: 1,
                command: "ls".into(),
            },
            1,
        ),
        (
            HostToGuest::FileWrite {
                id: 2,
                path: "/w/a".into(),
                data: vec![1, 2, 3],
                mode: 0o644,
            },
            2,
        ),
        (
            HostToGuest::FileRead {
                id: 3,
                path: "/w/b".into(),
            },
            3,
        ),
        (
            HostToGuest::FileDelete {
                id: 4,
                path: "/w/c".into(),
            },
            4,
        ),
    ];

    for (msg, want) in cases {
        assert_eq!(
            ackable_id(&msg),
            Some(want),
            "{msg:?} performs guest-side work and must survive a reconnect"
        );
    }
}

#[test]
fn control_chatter_is_not_ack_eligible() {
    // Replaying these would be noise at best; Shutdown replayed after a
    // reconnect would be actively wrong.
    for msg in [
        HostToGuest::Ping { epoch_secs: 0 },
        HostToGuest::Shutdown,
        HostToGuest::AckReply { id: 9 },
    ] {
        assert_eq!(ackable_id(&msg), None, "{msg:?} must not be replayed");
    }
}

#[test]
fn every_guest_to_host_completion_is_ack_eligible() {
    let cases: Vec<(GuestToHost, u64)> = vec![
        (
            GuestToHost::ExecDone {
                id: 10,
                exit_code: 0,
            },
            10,
        ),
        (GuestToHost::FileOpDone { id: 11 }, 11),
        (
            GuestToHost::FileContent {
                id: 12,
                path: "/w/b".into(),
                data: vec![0xde, 0xad],
            },
            12,
        ),
        (
            GuestToHost::Error {
                id: 13,
                message: "denied".into(),
            },
            13,
        ),
    ];

    for (msg, want) in cases {
        assert_eq!(
            ackable_response_id(&msg),
            Some(want),
            "{msg:?} is a completion the host must not lose"
        );
    }
}

#[test]
fn guest_liveness_messages_are_not_ack_eligible() {
    for msg in [
        GuestToHost::Pong,
        GuestToHost::Ready {
            version: "1.0".into(),
        },
    ] {
        assert_eq!(
            ackable_response_id(&msg),
            None,
            "{msg:?} carries no correlation id to ack"
        );
    }
}

#[test]
fn only_periodic_pong_is_expected_post_handshake_liveness() {
    assert!(is_guest_liveness_message(&GuestToHost::Pong));
    assert!(!is_guest_liveness_message(&GuestToHost::Ready {
        version: "1.0".into(),
    }));
    assert!(!is_guest_liveness_message(&GuestToHost::Error {
        id: 9,
        message: "broken".into(),
    }));
}

#[test]
fn the_two_directions_agree_on_which_ids_they_carry() {
    // A request that is ack-eligible must have a completion that is too,
    // otherwise one half of the pair survives a reconnect and the other does
    // not, and the caller waits forever on a reply that was dropped.
    let request = HostToGuest::FileRead {
        id: 77,
        path: "/w/x".into(),
    };
    let completion = GuestToHost::FileContent {
        id: 77,
        path: "/w/x".into(),
        data: vec![],
    };

    assert_eq!(ackable_id(&request), ackable_response_id(&completion));
}

// ── Exec output cap ────────────────────────────────────────────────
//
// The Exec vsock port is a raw stream, so the MAX_FRAME_SIZE bound that
// read_control_msg applies to length-prefixed control frames never reaches it.
// Before the cap, a guest running `yes` grew this process until the OOM killer
// took it and every in-flight job with it -- and the 5s deposit timeout did not
// help, because the reader thread is detached and keeps allocating after
// ExecDone has given up and dropped the slot.

/// A reader that never reaches EOF, standing in for `yes` or `cat /dev/urandom`.
struct EndlessReader {
    served: usize,
    limit: usize,
}

impl std::io::Read for EndlessReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        // Stop eventually so a regression fails the test instead of hanging it.
        if self.served >= self.limit {
            return Ok(0);
        }
        let n = buffer.len().min(self.limit - self.served);
        buffer[..n].fill(b'y');
        self.served += n;
        Ok(n)
    }
}

#[test]
fn exec_output_is_capped_against_an_endless_guest_stream() {
    // Offer twice the cap. A pre-fix build retains all of it.
    let mut reader = EndlessReader {
        served: 0,
        limit: MAX_EXEC_OUTPUT_BYTES * 2,
    };

    let (captured, total) = read_exec_output(&mut reader);

    assert_eq!(
        captured.len(),
        MAX_EXEC_OUTPUT_BYTES,
        "retained buffer must stop at the cap"
    );
    assert_eq!(
        total,
        (MAX_EXEC_OUTPUT_BYTES * 2) as u64,
        "the reported total is what the guest wrote, not what was kept"
    );
}

#[test]
fn exec_output_keeps_the_prefix_and_drains_to_eof() {
    // Draining past the cap matters: stopping the read early would leave the
    // guest blocked on a full socket instead of finishing its command.
    let mut reader = EndlessReader {
        served: 0,
        limit: MAX_EXEC_OUTPUT_BYTES + 4096,
    };

    let (captured, total) = read_exec_output(&mut reader);

    assert!(captured.iter().all(|b| *b == b'y'), "prefix is intact");
    assert_eq!(captured.len(), MAX_EXEC_OUTPUT_BYTES);
    assert_eq!(total, (MAX_EXEC_OUTPUT_BYTES + 4096) as u64);
}

#[test]
fn output_at_exactly_the_cap_is_not_reported_as_truncated() {
    let mut reader = EndlessReader {
        served: 0,
        limit: MAX_EXEC_OUTPUT_BYTES,
    };

    let (captured, total) = read_exec_output(&mut reader);

    assert_eq!(captured.len(), MAX_EXEC_OUTPUT_BYTES);
    assert_eq!(
        total,
        captured.len() as u64,
        "the boundary case must not look truncated"
    );
}

#[test]
fn ordinary_output_is_unaffected_by_the_cap() {
    let mut reader = std::io::Cursor::new(b"total 42\r\n".to_vec());

    let (captured, total) = read_exec_output(&mut reader);

    assert_eq!(captured, b"total 42\r\n");
    assert_eq!(total, 10);
}

#[test]
fn a_read_error_ends_capture_without_losing_what_was_already_read() {
    struct DataThenError {
        sent: bool,
    }
    impl std::io::Read for DataThenError {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.sent {
                return Err(std::io::Error::other("socket died"));
            }
            self.sent = true;
            buffer[..5].copy_from_slice(b"hello");
            Ok(5)
        }
    }

    let (captured, total) = read_exec_output(&mut DataThenError { sent: false });

    assert_eq!(captured, b"hello");
    assert_eq!(total, 5);
}

fn emission_with(
    action: capsem_core::security_engine::SecurityEnforcementAction,
    rule_id: Option<&str>,
    reason: Option<&str>,
) -> capsem_core::security_engine::SecurityRuleEmission {
    capsem_core::security_engine::SecurityRuleEmission {
        event_id: capsem_core::security_engine::SecurityEventId::parse("0123456789ab").unwrap(),
        emitted: 1,
        enforcement: capsem_core::security_engine::SecurityEnforcementDecision {
            action,
            rule_id: rule_id.map(str::to_string),
            rule_name: rule_id.map(str::to_string),
            reason: reason.map(str::to_string),
            ask_id: None,
        },
        event: capsem_core::security_engine::SecurityEvent::new(
            capsem_core::security_engine::RuntimeSecurityEventType::ProcessExec,
        ),
        rule_events: Vec::new(),
    }
}

#[test]
fn exec_boundary_allows_only_an_allow_decision() {
    use capsem_core::security_engine::SecurityEnforcementAction as Action;

    assert_eq!(
        exec_boundary_refusal(1, &Ok(Some(emission_with(Action::Allow, None, None)))),
        None,
        "an allowing boundary must dispatch the command"
    );

    let blocked = exec_boundary_refusal(
        2,
        &Ok(Some(emission_with(
            Action::Block,
            Some("profiles.rules.guard_curl"),
            Some("curl is not allowed"),
        ))),
    );
    assert_eq!(
        blocked.as_deref(),
        Some("curl is not allowed"),
        "the rule's own reason is what the caller sees"
    );

    let asked = exec_boundary_refusal(
        3,
        &Ok(Some(emission_with(
            Action::Ask,
            Some("profiles.rules.guard_curl"),
            None,
        ))),
    );
    assert_eq!(
        asked.as_deref(),
        Some("capsem: command requires approval by security rule: profiles.rules.guard_curl"),
        "an ask with no resolution path still withholds the command"
    );
}

#[test]
fn exec_boundary_refuses_when_it_cannot_decide() {
    let unwritten = exec_boundary_refusal(4, &Ok(None));
    assert_eq!(
        unwritten.as_deref(),
        Some("capsem: command refused, security ledger unavailable"),
        "a boundary that could not be recorded must not dispatch"
    );

    let failed = exec_boundary_refusal(5, &Err("rule set is broken".to_string()));
    assert_eq!(
        failed.as_deref(),
        Some("capsem: command refused, security evaluation failed: rule set is broken"),
        "an unevaluated boundary must not dispatch"
    );
}
