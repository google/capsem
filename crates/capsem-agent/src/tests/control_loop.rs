use super::*;

// -------------------------------------------------------------------
// control_loop integration tests
// -------------------------------------------------------------------

/// Feed host messages into control_loop and collect guest responses.
fn run_control_loop_with_messages(messages: Vec<HostToGuest>) -> Vec<GuestToHost> {
    run_control_loop_with_messages_and_pending(messages, None).0
}

/// Same as `run_control_loop_with_messages` but exposes the
/// pending_responses map so AckReply tests can inspect it.
fn run_control_loop_with_messages_and_pending(
    messages: Vec<HostToGuest>,
    seed: Option<Vec<(u64, GuestToHost)>>,
) -> (
    Vec<GuestToHost>,
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, GuestToHost>>>,
) {
    let (ctrl_read_fd, ctrl_write_fd) = make_pipe();
    let pty = openpty(None, None).expect("openpty");
    let master_fd = pty.master.as_raw_fd();
    // Spawn a child so we have a real PID for control_loop.
    let mut child = std::process::Command::new("sleep")
        .arg("300")
        .spawn()
        .expect("spawn sleep");
    let child_pid = Pid::from_raw(child.id() as i32);

    let (ctrl_tx, ctrl_rx) = test_ctrl_channel();

    // Write all messages then close the write end so control_loop
    // sees EOF and exits.
    for msg in &messages {
        let frame = capsem_proto::encode_host_msg(msg).unwrap();
        write_all_fd(ctrl_write_fd, &frame).unwrap();
    }
    unsafe {
        libc::close(ctrl_write_fd);
    }

    let pending = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(entries) = seed {
        let mut p = pending.lock().unwrap();
        for (id, msg) in entries {
            p.insert(id, msg);
        }
    }
    let pending_for_loop = std::sync::Arc::clone(&pending);

    let handle = thread::spawn(move || {
        control_loop(
            ctrl_read_fd,
            master_fd,
            child_pid,
            &[],
            ctrl_tx,
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            pending_for_loop,
        );
    });

    handle.join().unwrap();

    // Kill the sleep process immediately using std (handles waitpid internally).
    let _ = child.kill();
    let _ = child.wait();
    unsafe {
        libc::close(ctrl_read_fd);
    }

    // Drain the channel.
    let mut responses = Vec::new();
    while let Ok(msg) = ctrl_rx.lock().unwrap().try_recv() {
        responses.push(msg);
    }
    (responses, pending)
}

#[test]
fn control_loop_ping_responds_with_pong() {
    let responses = run_control_loop_with_messages(vec![HostToGuest::Ping { epoch_secs: 0 }]);
    assert_eq!(responses.len(), 1);
    assert!(matches!(responses[0], GuestToHost::Pong));
}

#[test]
fn control_loop_multiple_pings() {
    let responses = run_control_loop_with_messages(vec![
        HostToGuest::Ping { epoch_secs: 0 },
        HostToGuest::Ping { epoch_secs: 0 },
        HostToGuest::Ping { epoch_secs: 0 },
    ]);
    assert_eq!(responses.len(), 3);
    for r in &responses {
        assert!(matches!(r, GuestToHost::Pong));
    }
}

#[test]
fn control_loop_resize_changes_pty_winsize() {
    let (ctrl_read_fd, ctrl_write_fd) = make_pipe();
    let pty = openpty(None, None).expect("openpty");
    let master_fd = pty.master.as_raw_fd();
    let mut child = std::process::Command::new("sleep")
        .arg("300")
        .spawn()
        .expect("spawn sleep");
    let child_pid = Pid::from_raw(child.id() as i32);
    let (ctrl_tx, _ctrl_rx) = test_ctrl_channel();

    // Send resize then close.
    let frame = capsem_proto::encode_host_msg(&HostToGuest::Resize { cols: 132, rows: 43 }).unwrap();
    write_all_fd(ctrl_write_fd, &frame).unwrap();
    unsafe {
        libc::close(ctrl_write_fd);
    }

    let master_fd_check = master_fd;
    let handle = thread::spawn(move || {
        control_loop(
            ctrl_read_fd,
            master_fd,
            child_pid,
            &[],
            ctrl_tx,
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        );
    });
    handle.join().unwrap();

    // Verify the PTY was resized.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::ioctl(master_fd_check, libc::TIOCGWINSZ, &mut ws) };
    assert_eq!(ret, 0);
    assert_eq!(ws.ws_col, 132);
    assert_eq!(ws.ws_row, 43);

    let _ = child.kill();
    let _ = child.wait();
    unsafe {
        libc::close(ctrl_read_fd);
    }
}

#[test]
fn control_loop_file_write_path_traversal_rejected() {
    // Path traversal is rejected by validate_file_path (before workspace check),
    // so this works on macOS even though /root doesn't exist.
    let responses = run_control_loop_with_messages(vec![HostToGuest::FileWrite {
        id: 20,
        path: "/etc/../etc/passwd".into(),
        data: b"evil".to_vec(),
        mode: 0o644,
    }]);
    // First response is the Ack (sent on receipt before processing,
    // so the host bridge can clear the pending-ack map even when
    // the agent later rejects the request); second is the Error.
    assert_eq!(responses.len(), 2);
    assert!(matches!(responses[0], GuestToHost::Ack { id: 20 }));
    match &responses[1] {
        GuestToHost::Error { id, message } => {
            assert_eq!(*id, 20);
            assert!(
                message.contains("rejected") || message.contains("traversal"),
                "got: {message}"
            );
        }
        other => panic!("expected Error for traversal, got {other:?}"),
    }
}

#[test]
fn control_loop_file_read_rejected_outside_workspace() {
    // /etc/hostname is outside /root workspace, rejected by validate_file_path_safe
    // (or by workspace root canonicalization failure on macOS).
    let responses = run_control_loop_with_messages(vec![HostToGuest::FileRead {
        id: 10,
        path: "/etc/hostname".into(),
    }]);
    assert_eq!(responses.len(), 2);
    assert!(matches!(responses[0], GuestToHost::Ack { id: 10 }));
    match &responses[1] {
        GuestToHost::Error { id, .. } => assert_eq!(*id, 10),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn control_loop_file_delete_rejected_outside_workspace() {
    let responses = run_control_loop_with_messages(vec![HostToGuest::FileDelete {
        id: 30,
        path: "/tmp/some-file".into(),
    }]);
    assert_eq!(responses.len(), 2);
    assert!(matches!(responses[0], GuestToHost::Ack { id: 30 }));
    match &responses[1] {
        GuestToHost::Error { id, .. } => assert_eq!(*id, 30),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn control_loop_unhandled_message_does_not_crash() {
    // BootConfig is unexpected during control_loop (it's a boot-phase message).
    // control_loop should log it and continue.
    let responses = run_control_loop_with_messages(vec![
        HostToGuest::BootConfig {
            epoch_secs: 12345,
            traceparent: String::new(),
        },
        HostToGuest::Ping { epoch_secs: 0 },
    ]);
    // The BootConfig is just logged, only the Ping produces a response.
    assert_eq!(responses.len(), 1);
    assert!(matches!(responses[0], GuestToHost::Pong));
}

#[test]
fn control_loop_eof_exits_cleanly() {
    // Empty message list = immediate EOF on the pipe = control_loop exits.
    let responses = run_control_loop_with_messages(vec![]);
    assert!(responses.is_empty());
}

#[test]
fn control_loop_ack_reply_removes_pending_entry() {
    // Seed pending_responses with two entries, send AckReply for one,
    // verify only the matching entry was removed.
    let seed = vec![
        (42, GuestToHost::ExecDone { id: 42, exit_code: 0 }),
        (43, GuestToHost::FileOpDone { id: 43 }),
    ];
    let (_responses, pending) =
        run_control_loop_with_messages_and_pending(vec![HostToGuest::AckReply { id: 42 }], Some(seed));
    let p = pending.lock().unwrap();
    assert_eq!(p.len(), 1);
    assert!(!p.contains_key(&42));
    assert!(p.contains_key(&43));
    drop(p);
}

#[test]
fn control_loop_ack_reply_for_unknown_id_is_no_op() {
    // AckReply for an id that is not in pending_responses should be a no-op
    // (e.g. a duplicate AckReply from a replayed response that landed twice).
    let (_responses, pending) =
        run_control_loop_with_messages_and_pending(vec![HostToGuest::AckReply { id: 9999 }], None);
    assert!(pending.lock().unwrap().is_empty());
}

#[test]
fn ackable_response_id_covers_response_variants() {
    assert_eq!(
        ackable_response_id(&GuestToHost::ExecDone { id: 1, exit_code: 0 }),
        Some(1)
    );
    assert_eq!(ackable_response_id(&GuestToHost::FileOpDone { id: 2 }), Some(2));
    assert_eq!(
        ackable_response_id(&GuestToHost::FileContent {
            id: 3,
            path: "/x".into(),
            data: vec![]
        }),
        Some(3)
    );
    assert_eq!(
        ackable_response_id(&GuestToHost::Error {
            id: 4,
            message: "x".into()
        }),
        Some(4)
    );
    // Non-ackable variants
    assert_eq!(ackable_response_id(&GuestToHost::Pong), None);
    assert_eq!(ackable_response_id(&GuestToHost::Ack { id: 5 }), None);
    assert_eq!(ackable_response_id(&GuestToHost::SnapshotReady), None);
    assert_eq!(ackable_response_id(&GuestToHost::Ready { version: "x".into() }), None);
}

// Every per-connection thread must end promptly once the connection's lease
// drops, so run_bridge can join them before the outer loop closes the fds:
// a thread that outlived the connection kept using fd numbers the next
// connection was handed.
#[test]
fn heartbeat_thread_ends_promptly_when_the_lease_drops() {
    let (sender, _rx) = test_ctrl_channel();
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let handle = {
        let alive = std::sync::Arc::clone(&alive);
        thread::spawn(move || heartbeat_loop(&sender, &alive))
    };
    thread::sleep(std::time::Duration::from_millis(100));
    alive.store(false, std::sync::atomic::Ordering::SeqCst);
    let started = std::time::Instant::now();
    handle.join().unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the heartbeat must notice the dropped lease within one poll, took {:?}",
        started.elapsed()
    );
}
