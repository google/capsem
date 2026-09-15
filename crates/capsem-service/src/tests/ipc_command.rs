use super::*;

/// Every IPC connection to a VM owner also receives its lifecycle broadcasts.
/// `send_ipc_command` took the first non-filtered message as the reply, so a
/// `ShutdownRequested` that raced an exec became "unexpected IPC response for
/// exec". The reply is the message carrying the request's id.
#[tokio::test]
async fn send_ipc_command_ignores_lifecycle_broadcasts() {
    let dir = tempfile::tempdir().unwrap();
    let uds_path = dir.path().join("vm.sock");
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
        &uds_path,
        ServiceToProcess::Exec {
            id: 41,
            command: "true".into(),
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
