use super::*;

async fn accept_and_handshake(listener: &tokio::net::UnixListener) -> std::os::unix::net::UnixStream {
    let (stream, _) = listener.accept().await.unwrap();
    let std_stream = stream.into_std().unwrap();
    tokio::task::spawn_blocking(move || {
        let mut s = std_stream;
        capsem_foundation::ipc_handshake::negotiate_responder(&mut s, "capsem-process", "").unwrap();
        s
    })
    .await
    .unwrap()
}

fn peer_channel(s: std::os::unix::net::UnixStream) -> (Sender<ProcessToService>, Receiver<ServiceToProcess>) {
    channel_from_std(s).unwrap()
}

#[tokio::test]
async fn send_ipc_command_covers_connect_handshake_send_close_timeout_and_reply_paths() {
    let failed = IpcCommandError::Failed("channel broken".to_string());
    assert_eq!(
        failed.clone().into_internal_app_error().status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        failed.clone().into_exec_app_error(true).body.error,
        "exec failed: channel broken"
    );
    assert_eq!(String::from(failed), "channel broken");
    let tmp = tempfile::tempdir().unwrap();
    let missing_sock = tmp.path().join("missing.sock");
    let err = send_ipc_command(&missing_sock, ServiceToProcess::Ping, Some(1))
        .await
        .unwrap_err();
    assert!(matches!(&err, IpcCommandError::Failed(msg) if msg.contains("failed to connect to sandbox")));

    let drop_sock = tmp.path().join("drop.sock");
    let drop_listener = tokio::net::UnixListener::bind(&drop_sock).unwrap();
    tokio::spawn(async move {
        let (_stream, _) = drop_listener.accept().await.unwrap();
    });
    let err = send_ipc_command(&drop_sock, ServiceToProcess::Ping, Some(1))
        .await
        .unwrap_err();
    assert!(matches!(&err, IpcCommandError::Failed(msg) if msg.contains("IPC handshake failed")));

    let sock_path = tmp.path().join("owner.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
    let server = tokio::spawn(async move {
        let (tx, rx) = peer_channel(accept_and_handshake(&listener).await);
        assert!(matches!(rx.recv().await.unwrap(), ServiceToProcess::Ping));
        tx.send(ProcessToService::Pong).await.unwrap();
        let _std_stream2 = accept_and_handshake(&listener).await;
        let (_tx3, rx3) = peer_channel(accept_and_handshake(&listener).await);
        let _ = rx3.recv().await;
        drop((_tx3, rx3));
        let (_tx4, rx4) = peer_channel(accept_and_handshake(&listener).await);
        let _ = rx4.recv().await;
        drop((_tx4, rx4));
        let (_tx5, rx5) = peer_channel(accept_and_handshake(&listener).await);
        let _ = rx5.recv().await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    let ping = |t| send_ipc_command(&sock_path, ServiceToProcess::Ping, t);
    let reply = ping(None).await.unwrap();
    assert!(matches!(reply, ProcessToService::Pong));

    let oversized = ServiceToProcess::Exec {
        id: 1,
        command: "x".repeat(capsem_foundation::ipc_channel::MAX_IPC_FRAME_SIZE as usize + 1),
        target: capsem_proto::ipc::ExecTarget::Vm,
    };
    let send_err = send_ipc_command(&sock_path, oversized, Some(5)).await.unwrap_err();
    assert!(matches!(&send_err, IpcCommandError::Failed(msg) if msg.contains("failed to send IPC command")));

    let closed_deadline_err = ping(Some(5)).await.unwrap_err();
    assert!(matches!(&closed_deadline_err, IpcCommandError::Failed(msg) if msg.contains("IPC connection closed")));

    let closed_no_deadline_err = ping(None).await.unwrap_err();
    assert!(matches!(&closed_no_deadline_err, IpcCommandError::Failed(msg) if msg.contains("IPC connection closed")));

    let timeout_err = ping(Some(0)).await.unwrap_err();
    assert_eq!(timeout_err, IpcCommandError::Timeout { timeout_secs: 0 });
    assert_eq!(String::from(timeout_err), "IPC command timed out after 0s");
    server.abort();
}
