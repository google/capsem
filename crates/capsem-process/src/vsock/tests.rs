use super::*;

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
    let row = rows["rows"][0].as_array().expect("one joined row");

    assert_eq!(row[0].as_str(), Some(event_id.as_str()));
    assert_eq!(row[1].as_str(), Some(event_id.as_str()));
    assert_eq!(row[2].as_str(), Some("profiles.rules.openai_dns_seen"));
    assert_eq!(row[3].as_str(), Some("informational"));
}
