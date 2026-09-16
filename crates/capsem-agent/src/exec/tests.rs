use super::*;
use crate::control_writer::SharedCtrlReceiver;
use crate::tests::test_ctrl_channel;
use capsem_proto::GuestToHost;
use nix::libc;

// -----------------------------------------------------------------------
// Exec over vsock
// -----------------------------------------------------------------------

/// Helper: read ExecStarted handshake from exec fd, return exec id.
fn read_exec_started(exec_host: &mut std::os::unix::net::UnixStream) -> u64 {
    use std::io::Read;
    let mut len_buf = [0u8; 4];
    exec_host.read_exact(&mut len_buf).unwrap();
    let frame_len = u32::from_be_bytes(len_buf) as usize;
    let mut frame = vec![0u8; frame_len];
    exec_host.read_exact(&mut frame).unwrap();
    match capsem_proto::decode_guest_msg(&frame).unwrap() {
        GuestToHost::ExecStarted { id } => id,
        other => panic!("expected ExecStarted, got {other:?}"),
    }
}

fn read_exec_lanes(exec_host: &mut std::os::unix::net::UnixStream) -> (Vec<u8>, Vec<u8>) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        match capsem_proto::read_exec_output(exec_host) {
            Ok(frame) => match frame.channel {
                capsem_proto::ExecOutputChannel::Stdout => stdout.extend(frame.data),
                capsem_proto::ExecOutputChannel::Stderr => stderr.extend(frame.data),
            },
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => panic!("exec output frame failed: {error}"),
        }
    }
    (stdout, stderr)
}

fn recv_exec_done(rx: &SharedCtrlReceiver) -> (u64, i32) {
    let received = rx
        .lock()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    match received.message {
        GuestToHost::ExecDone { id, exit_code } => (id, exit_code),
        other => panic!("expected ExecDone, got {other:?}"),
    }
}

#[test]
fn exec_default_cwd_uses_root_only_for_root_user() {
    if unsafe { libc::geteuid() } == 0 && std::path::Path::new("/root").is_dir() {
        assert_eq!(default_exec_cwd(), "/root");
    } else {
        assert_eq!(default_exec_cwd(), "/");
    }
}

#[test]
fn exec_echo_captures_output_and_exit_code() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();

    let exec_fd = exec_guest.into_raw_fd();

    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 42, "echo hello", &[]);
    });

    let id = read_exec_started(&mut exec_host);
    assert_eq!(id, 42);

    let (output, stderr) = read_exec_lanes(&mut exec_host);
    assert_eq!(String::from_utf8_lossy(&output).trim(), "hello");
    assert!(stderr.is_empty());

    let (done_id, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(done_id, 42);
    assert_eq!(exit_code, 0);
}

#[test]
fn exec_nonzero_exit_code() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 7, "exit 42", &[]);
    });

    let _id = read_exec_started(&mut exec_host);
    let _output = read_exec_lanes(&mut exec_host);

    let (done_id, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(done_id, 7);
    assert_eq!(exit_code, 42);
}

#[test]
fn exec_boot_env_passed_to_child() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    let env = vec![("CAPSEM_TEST_VAR".to_string(), "test_value_42".to_string())];

    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 1, "echo $CAPSEM_TEST_VAR", &env);
    });

    let _id = read_exec_started(&mut exec_host);
    let (output, stderr) = read_exec_lanes(&mut exec_host);
    assert_eq!(String::from_utf8_lossy(&output).trim(), "test_value_42");
    assert!(stderr.is_empty());

    let (_, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(exit_code, 0);
}

#[test]
fn exec_accepts_binary_stdin_until_explicit_eof() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();
    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 2, "cat", &[]);
    });

    assert_eq!(read_exec_started(&mut exec_host), 2);
    capsem_proto::write_exec_input(
        &mut exec_host,
        &capsem_proto::ExecInputFrame::Data(b"input\0bytes".to_vec()),
    )
    .unwrap();
    capsem_proto::write_exec_input(&mut exec_host, &capsem_proto::ExecInputFrame::StdinEof).unwrap();
    let (stdout, stderr) = read_exec_lanes(&mut exec_host);
    assert_eq!(stdout, b"input\0bytes");
    assert!(stderr.is_empty());
    assert_eq!(recv_exec_done(&ctrl_rx), (2, 0));
}

#[test]
fn exec_cancellation_kills_the_child_process_group() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();
    let cancellation = std::sync::Arc::new(ExecCancellation::default());
    let cancellation_for_exec = std::sync::Arc::clone(&cancellation);
    std::thread::spawn(move || {
        run_exec_on_fds_with_cancel(
            exec_fd,
            &ctrl_tx,
            4,
            "trap '' TERM; sleep 30",
            &[],
            &cancellation_for_exec,
        );
    });

    assert_eq!(read_exec_started(&mut exec_host), 4);
    let started = std::time::Instant::now();
    cancellation.cancel();
    let _ = read_exec_lanes(&mut exec_host);
    let (id, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(id, 4);
    assert!(
        [128 + libc::SIGTERM, 128 + libc::SIGKILL].contains(&exit_code),
        "cancellation must report the terminating signal, got {exit_code}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn exec_stderr_captured() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 3, "echo out; echo err >&2", &[]);
    });

    let _id = read_exec_started(&mut exec_host);
    let (stdout, stderr) = read_exec_lanes(&mut exec_host);
    assert!(String::from_utf8_lossy(&stdout).contains("out"));
    assert!(String::from_utf8_lossy(&stderr).contains("err"));

    let (_, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(exit_code, 0);
}

#[test]
fn exec_sentinel_in_output_is_not_stripped() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 99, r#"printf '\033_CAPSEM_EXIT:999:0\033\\'"#, &[]);
    });

    let _id = read_exec_started(&mut exec_host);
    let (output, _) = read_exec_lanes(&mut exec_host);
    assert!(
        output.windows(14).any(|w| w == b"\x1b_CAPSEM_EXIT:"),
        "sentinel sequence should pass through as plain output"
    );

    let (done_id, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(done_id, 99);
    assert_eq!(exit_code, 0);
}

#[test]
fn exec_large_output_no_truncation() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    std::thread::spawn(move || {
        run_exec_on_fds(
            exec_fd,
            &ctrl_tx,
            5,
            "dd if=/dev/zero bs=1024 count=100 2>/dev/null | base64",
            &[],
        );
    });

    let _id = read_exec_started(&mut exec_host);
    let (output, _) = read_exec_lanes(&mut exec_host);
    assert!(output.len() > 100_000, "output too small: {} bytes", output.len());

    let (_, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(exit_code, 0);
}

// -----------------------------------------------------------------------
// ECONNRESET-retry helper for vsock_connect (Bug C)
//
// Post-`restoreMachineStateFromURL`, the host's vsock listener is
// registered but the kernel-side accept queue can briefly reset
// incoming connections. A single-shot vsock_connect from run_exec
// hits ECONNRESET, returns 126, and the agent's exec_done dedup
// cache poisons every host replay. The retry helper isolates
// the transient with a tight backoff; non-ECONNRESET errors bail
// immediately so we don't paper over real misconfiguration.
// -----------------------------------------------------------------------

#[test]
fn vsock_connect_econnreset_retry_succeeds_on_first_try() {
    let mut calls = 0;
    let result = vsock_connect_with_econnreset_retry(|| {
        calls += 1;
        Ok(42 as RawFd)
    });
    assert_eq!(result.unwrap(), 42);
    assert_eq!(calls, 1, "no retries needed when first call succeeds");
}

#[test]
fn vsock_connect_econnreset_retry_recovers_after_two_resets() {
    let mut calls = 0;
    let result = vsock_connect_with_econnreset_retry(|| {
        calls += 1;
        if calls < 3 {
            Err(io::Error::from(io::ErrorKind::ConnectionReset))
        } else {
            Ok(99 as RawFd)
        }
    });
    assert_eq!(result.unwrap(), 99);
    assert_eq!(calls, 3);
}

#[test]
fn vsock_connect_econnreset_retry_bails_immediately_on_other_kinds() {
    let mut calls = 0;
    let result = vsock_connect_with_econnreset_retry(|| {
        calls += 1;
        Err(io::Error::from(io::ErrorKind::ConnectionRefused))
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
    assert_eq!(calls, 1, "connection-refused should not retry");
}

#[test]
fn vsock_connect_econnreset_retry_exhausts_on_persistent_reset() {
    let mut calls = 0;
    let result = vsock_connect_with_econnreset_retry(|| {
        calls += 1;
        Err(io::Error::from(io::ErrorKind::ConnectionReset))
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(calls, ECONNRESET_MAX_ATTEMPTS, "should retry up to the cap");
}

// -----------------------------------------------------------------------
// ExecOutcome (Bug C): distinguishes a real exec exit from a transport
// failure that never reached the child. Only Done outcomes are cached
// in exec_done so a transient ECONNRESET cannot poison subsequent
// host retries/replays with a permanent 126.
// -----------------------------------------------------------------------

#[test]
fn exec_outcome_done_should_cache() {
    assert!(ExecOutcome::Done(0).should_cache());
    assert!(
        ExecOutcome::Done(126).should_cache(),
        "real exit_code=126 from a child process is still a real outcome to dedup"
    );
    assert!(ExecOutcome::Done(255).should_cache());
}

#[test]
fn exec_outcome_transport_failed_should_not_cache() {
    assert!(!ExecOutcome::TransportFailed.should_cache());
}

#[test]
fn exec_outcome_exit_code_for_host() {
    assert_eq!(ExecOutcome::Done(0).exit_code(), 0);
    assert_eq!(ExecOutcome::Done(42).exit_code(), 42);
    assert_eq!(
        ExecOutcome::TransportFailed.exit_code(),
        126,
        "transport failure surfaces as 126 to the host so the caller still sees an ExecResult"
    );
}

// -------------------------------------------------------------------
// Exec: distinct stdout and stderr lanes
// -------------------------------------------------------------------

#[test]
fn exec_stdout_and_stderr_keep_their_lanes() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    // Generate distinct output on both stdout and stderr
    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 50, "echo STDOUT_MARKER; echo STDERR_MARKER >&2", &[]);
    });

    let _id = read_exec_started(&mut exec_host);
    let (stdout, stderr) = read_exec_lanes(&mut exec_host);
    assert!(String::from_utf8_lossy(&stdout).contains("STDOUT_MARKER"));
    assert!(!String::from_utf8_lossy(&stdout).contains("STDERR_MARKER"));
    assert!(String::from_utf8_lossy(&stderr).contains("STDERR_MARKER"));
    assert!(!String::from_utf8_lossy(&stderr).contains("STDOUT_MARKER"));

    let (_, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(exit_code, 0);
}

#[test]
fn exec_invalid_command_returns_nonzero() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 60, "nonexistent_command_xyz", &[]);
    });

    let _id = read_exec_started(&mut exec_host);
    let _output = read_exec_lanes(&mut exec_host);

    let (_, exit_code) = recv_exec_done(&ctrl_rx);
    assert_ne!(exit_code, 0);
}

#[test]
fn exec_empty_command_succeeds() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 70, "true", &[]);
    });

    let _id = read_exec_started(&mut exec_host);
    let (stdout, stderr) = read_exec_lanes(&mut exec_host);
    assert!(stdout.is_empty(), "true should produce no stdout");
    assert!(stderr.is_empty(), "true should produce no stderr");

    let (_, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(exit_code, 0);
}

#[test]
fn exec_fd_closed_before_exec_done() {
    // Verify the agent closes exec_fd (EOF to host) before sending ExecDone.
    // The host relies on this ordering to accumulate all output before the
    // ExecDone arrives on the control channel.
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();
    let exec_fd = exec_guest.into_raw_fd();

    std::thread::spawn(move || {
        run_exec_on_fds(exec_fd, &ctrl_tx, 80, "echo ordering_test", &[]);
    });

    let _id = read_exec_started(&mut exec_host);

    // Read until EOF -- this blocks until exec_fd is closed.
    let (output, _) = read_exec_lanes(&mut exec_host);

    // EOF received. ExecDone should now be available (or arrive shortly).
    let (done_id, exit_code) = recv_exec_done(&ctrl_rx);
    assert_eq!(done_id, 80);
    assert_eq!(exit_code, 0);
    assert!(String::from_utf8_lossy(&output).contains("ordering_test"));
}
