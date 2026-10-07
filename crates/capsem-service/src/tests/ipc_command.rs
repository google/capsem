use super::*;

#[test]
fn socket_replacement_victim_child() {
    let Some(path) = std::env::var_os("CAPSEM_OWNER_SOCKET_TEST") else {
        return;
    };
    use std::io::{Read, Write};
    use std::os::fd::AsFd;
    let _listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    let mut control =
        std::os::unix::net::UnixStream::from(capsem_foundation::unix::fd::duplicate(std::io::stdin().as_fd()).unwrap());
    control
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    control.write_all(b"ready").unwrap();
    let mut done = [0; 1];
    control.read_exact(&mut done).unwrap();
}

#[tokio::test]
async fn replaced_owner_socket_cannot_receive_another_sessions_commands() {
    use std::io::{Read, Write};
    use std::os::fd::AsFd;
    use std::process::{Command, Stdio};
    let state = make_test_state();
    let session_dir = state.run_dir.join("sessions/victim");
    std::fs::create_dir_all(&session_dir).unwrap();
    let socket_path = session_dir.join("process.sock");
    let (mut control, child_control) = std::os::unix::net::UnixStream::pair().unwrap();
    control
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut victim = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::ipc_command::socket_replacement_victim_child",
            "--nocapture",
        ])
        .env_clear()
        .env("CAPSEM_OWNER_SOCKET_TEST", &socket_path)
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(child_control)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut ready = [0; 5];
    control.read_exact(&mut ready).unwrap();
    assert_eq!(&ready, b"ready");
    insert_fake_instance_with_session_dir(&state, "victim", victim.id(), session_dir);

    // The victim still owns its listener, but this same-UID process replaces
    // the filesystem entry with its own listener and a valid protocol peer.
    std::fs::remove_file(&socket_path).unwrap();
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    std::fs::write(socket_path.with_extension("ready"), b"ready").unwrap();
    let attacker = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = socket.into_std().unwrap();
        let (socket, hello, queued) = tokio::task::spawn_blocking(move || {
            let mut socket = socket;
            assert!(
                capsem_foundation::unix::fd::wait_readable(socket.as_fd(), std::time::Duration::from_secs(5)).unwrap()
            );
            let queued = capsem_foundation::unix::fd::stream_queues(socket.as_fd()).unread;
            let hello = capsem_foundation::ipc_handshake::negotiate_responder(&mut socket, "capsem-process", "");
            (socket, hello, queued)
        })
        .await
        .unwrap();
        if hello.is_err() {
            return (queued, false);
        }
        let (tx, rx): (Sender<ProcessToService>, Receiver<ServiceToProcess>) = channel_from_std(socket).unwrap();
        let ServiceToProcess::Exec { id, command, .. } = rx.recv().await.unwrap() else {
            panic!("expected the victim's exec")
        };
        assert_eq!(command, "echo protected-command");
        tx.send(ProcessToService::ExecResult {
            id,
            stdout: b"forged owner reply".to_vec(),
            stderr: Vec::new(),
            exit_code: 0,
            truncated: false,
        })
        .await
        .unwrap();
        (queued, true)
    });
    let (status, body) = route_request(
        build_service_router(Arc::clone(&state)),
        axum::http::Method::POST,
        "/vms/victim/exec",
        Some(json!({"command": "echo protected-command", "timeout_secs": 5})),
    )
    .await;
    let (protocol_bytes, received_command) = attacker.await.unwrap();
    control.write_all(b"x").unwrap();
    assert!(victim.wait().unwrap().success());
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(protocol_bytes, Some(0), "authenticate before sending even Hello");
    assert!(
        !received_command,
        "the replacement process received another session's command"
    );
}

/// Every IPC connection to a VM owner also receives its lifecycle broadcasts.
/// `send_ipc_command` took the first non-filtered message as the reply, so a
/// `ShutdownRequested` that raced an exec became "unexpected IPC response for
/// exec". The reply is the message carrying the request's id.
#[tokio::test]
async fn replaced_generation_cannot_supply_an_accepted_command_reply() {
    let state = make_test_state();
    let session = state.run_dir.join("sessions/replaced");
    std::fs::create_dir_all(&session).unwrap();
    insert_fake_instance_with_session_dir(&state, "replaced", std::process::id(), session);
    let path = state.instances.lock().unwrap()["replaced"].uds_path.clone();
    let peer_state = Arc::clone(&state);
    let owner = spawn_fake_process(&path, 1, move |message| {
        let ServiceToProcess::Exec { id, .. } = message else {
            panic!("expected exec")
        };
        peer_state
            .instances
            .lock()
            .unwrap()
            .get_mut("replaced")
            .unwrap()
            .generation = uuid::Uuid::new_v4();
        let reply = ProcessToService::ExecResult {
            id: *id,
            stdout: b"old generation".to_vec(),
            stderr: vec![],
            exit_code: 0,
            truncated: false,
        };
        Box::pin(async move { Some(reply) })
    });
    let result = send_ipc_command(
        &state,
        &path,
        ServiceToProcess::Exec {
            id: 42,
            command: "true".into(),
            target: capsem_proto::ipc::ExecTarget::Vm,
        },
        Some(5),
    )
    .await;
    owner.await.unwrap();
    assert!(result.unwrap_err().contains("VM owner changed"));
}

#[tokio::test]
async fn send_ipc_command_ignores_lifecycle_broadcasts() {
    let state = make_test_state();
    let session_dir = state.run_dir.join("sessions/vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "vm", std::process::id(), session_dir);
    let uds_path = state.instances.lock().unwrap()["vm"].uds_path.clone();
    let listener = tokio::net::UnixListener::bind(&uds_path).unwrap();
    let owner = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let std_stream = stream.into_std().unwrap();
        let std_stream = tokio::task::spawn_blocking(move || {
            let mut std_stream = std_stream;
            capsem_foundation::ipc_handshake::negotiate_responder(&mut std_stream, "capsem-process-test", "")
                .map(|_| std_stream)
        })
        .await
        .unwrap()
        .unwrap();
        let (tx, rx): (
            capsem_foundation::ipc_channel::Sender<ProcessToService>,
            capsem_foundation::ipc_channel::Receiver<ServiceToProcess>,
        ) = capsem_foundation::ipc_channel::channel_from_std(std_stream).unwrap();
        let ServiceToProcess::Exec { id, .. } = rx.recv().await.unwrap() else {
            panic!("expected exec");
        };
        for noise in [
            ProcessToService::ShutdownRequested { id: "vm".into() },
            ProcessToService::SuspendRequested { id: "vm".into() },
            ProcessToService::SuspendFailed {
                id: "vm".into(),
                error: "busy".into(),
            },
            ProcessToService::ExecOutput {
                id,
                channel: capsem_proto::ExecOutputChannel::Stdout,
                data: b"partial".to_vec(),
            },
            ProcessToService::ExecResult {
                id: id + 1,
                stdout: b"other".to_vec(),
                stderr: vec![],
                exit_code: 9,
                truncated: false,
            },
            ProcessToService::ExecResult {
                id,
                stdout: b"ok".to_vec(),
                stderr: vec![],
                exit_code: 0,
                truncated: false,
            },
        ] {
            tx.send(noise).await.unwrap();
        }
    });

    let reply = send_ipc_command(
        &state,
        &uds_path,
        ServiceToProcess::Exec {
            id: 41,
            command: "true".into(),
            target: capsem_proto::ipc::ExecTarget::Vm,
        },
        Some(5),
    )
    .await
    .expect("command must complete despite broadcasts");
    match reply {
        ProcessToService::ExecResult {
            id, stdout, exit_code, ..
        } => {
            assert_eq!((id, stdout.as_slice(), exit_code), (41, &b"ok"[..], 0));
        }
        other => panic!("broadcast or unrelated message returned as the reply: {other:?}"),
    }
    owner.await.unwrap();
}
