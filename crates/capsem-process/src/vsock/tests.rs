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
    handle_guest_msg(GuestToHost::ExecDone { id, exit_code: 0 }, &js, &db).await;
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
async fn blocked_exec_resolves_job_without_guest_dispatch_state() {
    use crate::job_store::{ActiveExec, JobResult, JobStore};
    use std::sync::Arc;
    use tokio::sync::oneshot;

    let js = Arc::new(JobStore::new());
    let id: u64 = 77;
    let (tx, rx) = oneshot::channel::<JobResult>();
    js.jobs.lock().unwrap().insert(id, tx);
    *js.active_exec.lock().unwrap() = Some(ActiveExec::new(id));

    resolve_blocked_exec_job(&js, id, "blocked by process rule".into());

    assert!(js.active_exec.lock().unwrap().is_none());
    assert!(js.jobs.lock().unwrap().is_empty());
    let result = rx.await.expect("blocked exec must resolve job");
    match result {
        JobResult::Error { message } => assert_eq!(message, "blocked by process rule"),
        other => panic!("expected blocked exec error, got {other:?}"),
    }
}

fn blocked_process_exec_evaluation(
) -> capsem_core::process_security_events::ProcessExecSecurityEvaluation {
    use capsem_logger::ExecEvent;
    use capsem_security_engine::{
        CelEnforcementEvaluator, CelEnforcementRule, SecurityDecisionAction, SecurityEngine,
    };
    use std::time::SystemTime;

    let event = ExecEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        exec_id: 88,
        command: "bash -lc 'echo blocked'".into(),
        source: "api".into(),
        mcp_call_id: Some(12),
        trace_id: Some("trace-process-log".into()),
        process_name: Some("capsem-agent".into()),
    };
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(
        CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
            id: "runtime.block-shell".into(),
            pack_id: Some("runtime-pack".into()),
            condition:
                "process.activity.operation == 'exec' && process.activity.command_class == 'shell'"
                    .into(),
            decision: SecurityDecisionAction::Block,
            reason: Some("shell exec blocked".into()),
        }])
        .unwrap(),
    ));
    let engine = std::sync::Mutex::new(engine);

    capsem_core::process_security_events::evaluate_exec_security_event(&event, Some(&engine))
}

#[test]
fn process_exec_security_log_record_carries_attribution_rule_and_reason() {
    let evaluation = blocked_process_exec_evaluation();
    let record = process_exec_security_log_record(&evaluation.resolved_event);

    assert_eq!(record.event_type, "process.exec");
    assert_eq!(record.event_family, "process");
    assert_eq!(record.source_engine, "process");
    assert_eq!(record.final_action, "block");
    assert_eq!(record.enforceability, "inline_blockable");
    assert_eq!(record.attribution_scope, "vm");
    assert_eq!(record.origin_kind, "host_service");
    assert_eq!(record.trace_id, Some("trace-process-log"));
    assert_eq!(record.exec_id, Some("88"));
    assert_eq!(record.mcp_call_id, Some("12"));
    assert_eq!(record.operation, Some("exec"));
    assert_eq!(record.command_class, Some("shell"));
    assert_eq!(record.rule_id, Some("runtime.block-shell"));
    assert_eq!(record.pack_id, Some("runtime-pack"));
    assert_eq!(record.reason, Some("shell exec blocked"));
    assert_eq!(record.finding_count, 0);
}

#[test]
fn process_exec_security_decision_tracing_line_serializes_debug_fields() {
    use std::io::{Result as IoResult, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    let log_bytes = Arc::new(Mutex::new(Vec::new()));
    let writer_bytes = log_bytes.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || SharedWriter(writer_bytes.clone()))
        .finish();
    let dispatch = tracing::Dispatch::new(subscriber);
    let evaluation = blocked_process_exec_evaluation();

    tracing::dispatcher::with_default(&dispatch, || {
        log_process_exec_security_decision(&evaluation.resolved_event);
    });

    let output = String::from_utf8(log_bytes.lock().unwrap().clone()).unwrap();
    let line = output
        .lines()
        .find(|line| line.contains("process_exec_security_decision"))
        .expect("structured process security decision log line");
    let json: serde_json::Value = serde_json::from_str(line).unwrap();
    let fields = &json["fields"];

    assert_eq!(json["target"], "security.process");
    assert_eq!(fields["message"], "process_exec_security_decision");
    assert_eq!(fields["event_type"], "process.exec");
    assert_eq!(fields["event_family"], "process");
    assert_eq!(fields["source_engine"], "process");
    assert_eq!(fields["final_action"], "block");
    assert_eq!(fields["enforceability"], "inline_blockable");
    assert_eq!(fields["attribution_scope"], "vm");
    assert_eq!(fields["origin_kind"], "host_service");
    assert_eq!(fields["trace_id"], "trace-process-log");
    assert_eq!(fields["exec_id"], "88");
    assert_eq!(fields["mcp_call_id"], "12");
    assert_eq!(fields["operation"], "exec");
    assert_eq!(fields["command_class"], "shell");
    assert_eq!(fields["rule_id"], "runtime.block-shell");
    assert_eq!(fields["pack_id"], "runtime-pack");
    assert_eq!(fields["reason"], "shell exec blocked");
    assert_eq!(fields["finding_count"], serde_json::json!(0));
}
