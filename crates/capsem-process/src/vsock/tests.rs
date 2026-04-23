use super::*;

// -----------------------------------------------------------------------
// Vsock port classification
// -----------------------------------------------------------------------

#[test]
fn classify_terminal_port() {
    assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_TERMINAL), VsockPortKind::Terminal);
}

#[test]
fn classify_control_port() {
    assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_CONTROL), VsockPortKind::Control);
}

#[test]
fn classify_sni_proxy_port() {
    assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_SNI_PROXY), VsockPortKind::SniProxy);
}

#[test]
fn classify_mcp_gateway_port() {
    assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_MCP_GATEWAY), VsockPortKind::McpGateway);
}

#[test]
fn classify_exec_port() {
    assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_EXEC), VsockPortKind::Exec);
}

#[test]
fn classify_lifecycle_port() {
    assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_LIFECYCLE), VsockPortKind::Lifecycle);
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
fn unexpected_eof_not_retryable() {
    // UnexpectedEof is intentionally NOT retryable: it signals the guest
    // wedged mid-handshake (e.g. kernel I/O failure on the overlay), not
    // the Apple VZ half-open vsock case. Retrying would just burn the
    // readiness budget against a genuinely broken guest.
    let io_err = std::io::Error::from(std::io::ErrorKind::UnexpectedEof);
    let err: anyhow::Error = anyhow::Error::new(io_err).context("BootReady read failed");
    assert!(!is_retryable_handshake_error(&err));
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
    tx.send(make_conn(capsem_core::VSOCK_PORT_TERMINAL)).unwrap();

    let mut deferred = Vec::new();
    let (terminal, control) = collect_terminal_control_pair(&mut rx, &mut deferred)
        .await
        .expect("pair collected");
    assert_eq!(terminal.port, capsem_core::VSOCK_PORT_TERMINAL);
    assert_eq!(control.port, capsem_core::VSOCK_PORT_CONTROL);
    assert!(deferred.is_empty());
}

#[tokio::test]
async fn collect_parks_sni_and_mcp_as_deferred() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(make_conn(capsem_core::VSOCK_PORT_SNI_PROXY)).unwrap();
    tx.send(make_conn(capsem_core::VSOCK_PORT_MCP_GATEWAY)).unwrap();
    tx.send(make_conn(capsem_core::VSOCK_PORT_TERMINAL)).unwrap();
    tx.send(make_conn(capsem_core::VSOCK_PORT_CONTROL)).unwrap();

    let mut deferred = Vec::new();
    collect_terminal_control_pair(&mut rx, &mut deferred)
        .await
        .expect("pair collected");
    assert_eq!(deferred.len(), 2);
    assert_eq!(deferred[0].port, capsem_core::VSOCK_PORT_SNI_PROXY);
    assert_eq!(deferred[1].port, capsem_core::VSOCK_PORT_MCP_GATEWAY);
}

#[tokio::test]
async fn collect_errors_when_channel_closes_early() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(make_conn(capsem_core::VSOCK_PORT_TERMINAL)).unwrap();
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
    use crate::job_store::{JobStore, JobResult};
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
        JobResult::Exec { stdout, exit_code, .. } => {
            assert!(stdout.is_empty(), "no-output command should return empty stdout");
            assert_eq!(exit_code, 0);
        }
        other => panic!("expected Exec result, got {other:?}"),
    }
}
