use super::dns::emit_dns_security_write_and_rules;
use super::*;

mod ports;

mod ack;
mod audit;

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

fn encoded_exec_output(channel: capsem_proto::ExecOutputChannel, total: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut remaining = total;
    while remaining > 0 {
        let size = remaining.min(capsem_proto::MAX_EXEC_DATA_BYTES);
        capsem_proto::write_exec_output(
            &mut bytes,
            &capsem_proto::ExecOutputFrame {
                channel,
                data: vec![b'y'; size],
            },
        )
        .unwrap();
        remaining -= size;
    }
    bytes
}

#[test]
fn exec_output_read_retries_interrupted_socket_reads() {
    let mut reader = InterruptedThenData {
        interrupted: false,
        data: std::io::Cursor::new({
            let mut bytes = Vec::new();
            capsem_proto::write_exec_output(
                &mut bytes,
                &capsem_proto::ExecOutputFrame {
                    channel: capsem_proto::ExecOutputChannel::Stdout,
                    data: b"IRONBANK_CLIENT_RESULT={\"ok\":true}\n".to_vec(),
                },
            )
            .unwrap();
            bytes
        }),
    };

    let captured = exec_output::read_exec_output(&mut reader);

    assert_eq!(captured.stdout, b"IRONBANK_CLIENT_RESULT={\"ok\":true}\n");
    assert_eq!(
        captured.stdout_bytes,
        captured.stdout.len() as u64,
        "nothing was dropped"
    );
}

#[test]
fn serial_log_writer_runs_on_a_dedicated_thread() {
    struct TrackingWriter {
        bytes: Arc<std::sync::Mutex<Vec<u8>>>,
        writer_thread: Arc<std::sync::Mutex<Option<std::thread::ThreadId>>>,
    }

    impl std::io::Write for TrackingWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            *self.writer_thread.lock().unwrap() = Some(std::thread::current().id());
            self.bytes.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let caller_thread = std::thread::current().id();
    let bytes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer_thread = Arc::new(std::sync::Mutex::new(None));
    let bytes_for_writer = Arc::clone(&bytes);
    let thread_for_writer = Arc::clone(&writer_thread);
    let (tx, handle) = spawn_serial_log_writer(move || {
        Ok(TrackingWriter {
            bytes: bytes_for_writer,
            writer_thread: thread_for_writer,
        })
    });

    tx.blocking_send(b"first".to_vec()).unwrap();
    tx.blocking_send(b"second".to_vec()).unwrap();
    drop(tx);
    handle.join().unwrap();

    assert_eq!(*bytes.lock().unwrap(), b"firstsecond");
    assert_ne!(*writer_thread.lock().unwrap(), Some(caller_thread));
}

#[test]
fn bounded_frame_reader_returns_one_complete_payload() {
    let payload = b"audit-record";
    let mut frame = Vec::from((payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);

    let decoded = read_bounded_frame(&mut std::io::Cursor::new(frame))
        .unwrap()
        .expect("complete frame");

    assert_eq!(decoded, payload);
}

#[test]
fn bounded_frame_reader_rejects_oversized_length_before_payload_read() {
    let oversized = capsem_proto::MAX_FRAME_SIZE + 1;
    let mut reader = std::io::Cursor::new(oversized.to_be_bytes().to_vec());

    let error = read_bounded_frame(&mut reader).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(reader.position(), 4, "only the length prefix may be read");
}

#[test]
fn bounded_frame_reader_rejects_truncated_payload() {
    let mut frame = Vec::from(5u32.to_be_bytes());
    frame.extend_from_slice(b"no");

    let error = read_bounded_frame(&mut std::io::Cursor::new(frame)).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn bounded_frame_reader_returns_none_on_clean_eof() {
    let frame = read_bounded_frame(&mut std::io::Cursor::new(Vec::new())).unwrap();

    assert!(frame.is_none());
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
    Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new().into()))
}

fn file_import_event_with_content(content: &str) -> capsem_core::security_engine::SecurityEvent {
    capsem_core::security_engine::SecurityEvent::new(capsem_core::security_engine::RuntimeSecurityEventType::FileImport)
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
    tx.send(make_conn(capsem_proto::VSOCK_PORT_CONTROL)).unwrap();
    tx.send(make_conn(capsem_proto::VSOCK_PORT_TERMINAL)).unwrap();

    let mut deferred = Vec::new();
    let (terminal, control) = collect_terminal_control_pair(&mut rx, &mut deferred)
        .await
        .expect("pair collected");
    assert_eq!(terminal.port, capsem_proto::VSOCK_PORT_TERMINAL);
    assert_eq!(control.port, capsem_proto::VSOCK_PORT_CONTROL);
    assert!(deferred.is_empty());
}

#[tokio::test]
async fn collect_parks_sni_but_ignores_removed_legacy_mcp_port() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(make_conn(capsem_proto::VSOCK_PORT_SNI_PROXY)).unwrap();
    tx.send(make_conn(5003)).unwrap();
    tx.send(make_conn(capsem_proto::VSOCK_PORT_TERMINAL)).unwrap();
    tx.send(make_conn(capsem_proto::VSOCK_PORT_CONTROL)).unwrap();

    let mut deferred = Vec::new();
    collect_terminal_control_pair(&mut rx, &mut deferred)
        .await
        .expect("pair collected");
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].port, capsem_proto::VSOCK_PORT_SNI_PROXY);
    assert_eq!(HostVsockService::from_port(5003), None);
}

#[tokio::test]
async fn collect_errors_when_channel_closes_early() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(make_conn(capsem_proto::VSOCK_PORT_TERMINAL)).unwrap();
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
    // installed the active exec slot, and the EXEC-port reader has already
    // deposited its (empty) local_buf and signalled completion. ExecDone
    // arriving after that must return immediately -- no blanket stall.
    let active = crate::job_store::ActiveExec::new();
    active.deposited.notify_one();
    js.active_execs.lock().unwrap().insert(id, active);

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
        JobResult::Exec { stdout, exit_code, .. } => {
            assert!(stdout.is_empty(), "no-output command should return empty stdout");
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
    js.active_execs
        .lock()
        .unwrap()
        .insert(id, crate::job_store::ActiveExec::new());

    // Reproduce a loaded runner after resume: the serialized control channel
    // delivers ExecDone promptly, while the dedicated EXEC-port reader does
    // not get scheduled to deposit the already-produced bytes for >100 ms.
    let js_for_deposit = Arc::clone(&js);
    let deposit = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let notify = {
            let mut guard = js_for_deposit.active_execs.lock().unwrap();
            let active = guard
                .get_mut(&id)
                .expect("ExecDone must not discard the capture slot before deposit");
            active.captured = b"/run/capsem-venv\n".to_vec();
            let deposited = Arc::clone(&active.deposited);
            drop(guard);
            deposited
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
        JobResult::Exec { stdout, exit_code, .. } => {
            assert_eq!(stdout, b"/run/capsem-venv\n");
            assert_eq!(exit_code, 0);
        }
        other => panic!("expected Exec result, got {other:?}"),
    }
}

#[tokio::test]
async fn concurrent_exec_completions_keep_each_stdout() {
    use crate::job_store::{ActiveExec, JobResult, JobStore};
    use capsem_proto::GuestToHost;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    let js = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )));
    let plugin_policy = empty_plugin_policy();
    let first_id = 501;
    let second_id = 502;
    let (first_tx, first_rx) = oneshot::channel();
    let (second_tx, second_rx) = oneshot::channel();
    js.jobs.lock().unwrap().insert(first_id, first_tx);
    js.jobs.lock().unwrap().insert(second_id, second_tx);

    {
        let mut active = js.active_execs.lock().unwrap();
        active.insert(first_id, ActiveExec::new());
        active.insert(second_id, ActiveExec::new());
    }
    if let Some(notify) = exec_output::deposit(
        &js,
        first_id,
        exec_output::ExecCapture {
            stdout: b"first\n".to_vec(),
            stdout_bytes: 6,
            ..Default::default()
        },
    ) {
        notify.notify_one();
    }
    if let Some(notify) = exec_output::deposit(
        &js,
        second_id,
        exec_output::ExecCapture {
            stdout: b"second\n".to_vec(),
            stdout_bytes: 7,
            ..Default::default()
        },
    ) {
        notify.notify_one();
    }

    handle_guest_msg(
        GuestToHost::ExecDone {
            id: first_id,
            exit_code: 0,
        },
        &js,
        &db,
        &security_rules,
        &plugin_policy,
    )
    .await;
    handle_guest_msg(
        GuestToHost::ExecDone {
            id: second_id,
            exit_code: 0,
        },
        &js,
        &db,
        &security_rules,
        &plugin_policy,
    )
    .await;

    let stdout = |result| match result {
        JobResult::Exec { stdout, .. } => stdout,
        other => panic!("expected Exec result, got {other:?}"),
    };
    assert_eq!(stdout(first_rx.await.unwrap()), b"first\n");
    assert_eq!(stdout(second_rx.await.unwrap()), b"second\n");
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
        JobResult::ReadFile { data: Some(data), .. } => assert_eq!(data, b"guest export bytes"),
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

/// An export nobody could record is refused: with the ledger writer gone, the
/// guest's bytes never reach the caller.
#[tokio::test]
async fn read_file_content_is_refused_when_its_export_cannot_be_recorded() {
    use capsem_proto::GuestToHost;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(capsem_logger::DbWriter::open(&dir.path().join("session.db"), 16).unwrap());
    db.shutdown_blocking();
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml("").expect("rules parse");
    let rules = capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("rules compile");
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(rules)));
    let plugin_policy = empty_plugin_policy();
    let js = Arc::new(JobStore::new());
    let id: u64 = 78;
    js.active_file_ops.lock().unwrap().insert(
        id,
        ActiveFileOp::Read {
            path: "/workspace/secret.txt".to_string(),
        },
    );
    let (tx, rx) = oneshot::channel::<JobResult>();
    js.jobs.lock().unwrap().insert(id, tx);

    handle_guest_msg(
        GuestToHost::FileContent {
            id,
            path: "/workspace/secret.txt".to_string(),
            data: b"must not leave unrecorded".to_vec(),
        },
        &js,
        &db,
        &security_rules,
        &plugin_policy,
    )
    .await;

    match rx.await.expect("read job must resolve") {
        JobResult::ReadFile {
            data: None,
            error: Some(error),
        } => {
            assert!(error.contains("refused"), "{error}");
        }
        other => panic!("expected a refused export with no data, got {other:?}"),
    }
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
    // The caller has completed its write and can shut down immediately.
    // No yield to an unowned derived-rule task may be required.
    db.shutdown_blocking();

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

// ── Exec output cap ────────────────────────────────────────────────
//
// The Exec vsock port is a raw stream, so the MAX_FRAME_SIZE bound that
// read_control_msg applies to length-prefixed control frames never reaches it.
#[test]
fn exec_output_is_capped_against_an_endless_guest_stream() {
    let mut reader = std::io::Cursor::new(encoded_exec_output(
        capsem_proto::ExecOutputChannel::Stdout,
        exec_output::EXEC_LEDGER_BODY_BYTES * 2,
    ));
    let captured = exec_output::read_exec_output(&mut reader);

    assert_eq!(
        captured.stdout.len(),
        exec_output::EXEC_LEDGER_BODY_BYTES,
        "retained buffer must stop at the cap"
    );
    assert_eq!(
        captured.stdout_bytes,
        (exec_output::EXEC_LEDGER_BODY_BYTES * 2) as u64,
        "the reported total is what the guest wrote, not what was kept"
    );
}

#[test]
fn exec_output_keeps_the_prefix_and_drains_to_eof() {
    // Draining past the cap matters: stopping the read early would leave the
    // guest blocked on a full socket instead of finishing its command.
    let mut reader = std::io::Cursor::new(encoded_exec_output(
        capsem_proto::ExecOutputChannel::Stdout,
        exec_output::EXEC_LEDGER_BODY_BYTES + 4096,
    ));
    let captured = exec_output::read_exec_output(&mut reader);

    assert!(captured.stdout.iter().all(|b| *b == b'y'), "prefix is intact");
    assert_eq!(captured.stdout.len(), exec_output::EXEC_LEDGER_BODY_BYTES);
    assert_eq!(
        captured.stdout_bytes,
        (exec_output::EXEC_LEDGER_BODY_BYTES + 4096) as u64
    );
}

#[test]
fn output_at_exactly_the_cap_is_not_reported_as_truncated() {
    let mut reader = std::io::Cursor::new(encoded_exec_output(
        capsem_proto::ExecOutputChannel::Stdout,
        exec_output::EXEC_LEDGER_BODY_BYTES,
    ));
    let captured = exec_output::read_exec_output(&mut reader);

    assert_eq!(captured.stdout.len(), exec_output::EXEC_LEDGER_BODY_BYTES);
    assert_eq!(
        captured.stdout_bytes,
        captured.stdout.len() as u64,
        "the boundary case must not look truncated"
    );
}

#[test]
fn ordinary_output_is_unaffected_by_the_cap() {
    let mut bytes = Vec::new();
    capsem_proto::write_exec_output(
        &mut bytes,
        &capsem_proto::ExecOutputFrame {
            channel: capsem_proto::ExecOutputChannel::Stderr,
            data: b"total 42\r\n".to_vec(),
        },
    )
    .unwrap();
    let captured = exec_output::read_exec_output(&mut std::io::Cursor::new(bytes));

    assert_eq!(captured.stderr, b"total 42\r\n");
    assert_eq!(captured.stderr_bytes, 10);
}

#[test]
fn a_read_error_ends_capture_without_losing_what_was_already_read() {
    struct DataThenError {
        data: std::io::Cursor<Vec<u8>>,
    }
    impl std::io::Read for DataThenError {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.data.read(buffer)?;
            if read == 0 {
                return Err(std::io::Error::other("socket died"));
            }
            Ok(read)
        }
    }
    let mut bytes = Vec::new();
    capsem_proto::write_exec_output(
        &mut bytes,
        &capsem_proto::ExecOutputFrame {
            channel: capsem_proto::ExecOutputChannel::Stdout,
            data: b"hello".to_vec(),
        },
    )
    .unwrap();
    let captured = exec_output::read_exec_output(&mut DataThenError {
        data: std::io::Cursor::new(bytes),
    });

    assert_eq!(captured.stdout, b"hello");
    assert_eq!(captured.stdout_bytes, 5);
}
