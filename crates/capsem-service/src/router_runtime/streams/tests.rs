use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_test_state};
use capsem_api::stream::{decode_server_frame, encode_control, encode_data, ServerFrame};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};

mod flow;

type Client = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Fixture {
    state: Arc<ServiceState>,
    address: std::net::SocketAddr,
    uds_path: PathBuf,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let state = make_test_state();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(session_dir.join("guest/workspace")).unwrap();
    insert_fake_instance_with_session_dir(&state, "box", 1, session_dir);
    let uds_path = state.instances.lock().unwrap()["box"].uds_path.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = build_service_router(Arc::clone(&state));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Fixture {
        state,
        address,
        uds_path,
        _dir: dir,
    }
}

/// A scripted VM owner: accepts one stream-role connection and runs `script`.
fn owner<F, Fut>(uds_path: &StdPath, script: F) -> tokio::task::JoinHandle<()>
where
    F: FnOnce(
            capsem_foundation::ipc_channel::Sender<ProcessToService>,
            capsem_foundation::ipc_channel::Receiver<ServiceToProcess>,
        ) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let _ = std::fs::remove_file(uds_path);
    let listener = tokio::net::UnixListener::bind(uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let stream = stream.into_std().unwrap();
        let (stream, peer) = tokio::task::spawn_blocking(move || {
            let mut stream = stream;
            let peer =
                capsem_foundation::ipc_handshake::negotiate_responder(&mut stream, "capsem-process-test", "").unwrap();
            (stream, peer)
        })
        .await
        .unwrap();
        assert_eq!(
            peer.peer,
            capsem_proto::handshake::STREAM_PEER_ID,
            "streams use stream-role owner connections"
        );
        let (tx, rx) = capsem_foundation::ipc_channel::channel_from_std(stream).unwrap();
        script(tx, rx).await;
    })
}

async fn connect(
    address: std::net::SocketAddr,
    protocol: Option<&str>,
) -> Result<Client, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("ws://{address}/vms/box/stream").into_client_request().unwrap();
    if let Some(protocol) = protocol {
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.parse().unwrap());
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(client, _)| client)
}

async fn next_server_frame(client: &mut Client) -> Option<(u8, Vec<u8>)> {
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("stream stalled")?
        {
            Ok(ClientMessage::Binary(bytes)) => return Some((bytes[0], bytes.to_vec())),
            Ok(ClientMessage::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        }
    }
}

fn status(frame: &[u8]) -> StreamStatus {
    match decode_server_frame(frame).unwrap() {
        ServerFrame::Status(status) => status,
        other => panic!("expected a status frame, got {other:?}"),
    }
}

#[tokio::test]
async fn exec_stream_relays_output_and_ends_with_the_exit_status() {
    let fx = fixture().await;
    let owner = owner(&fx.uds_path, |tx, rx| async move {
        let ServiceToProcess::ExecStream { id, command } = rx.recv().await.unwrap() else {
            panic!("expected ExecStream")
        };
        assert_eq!(command, "echo hi");
        tx.send(ProcessToService::StateChanged {
            id: "noise".into(),
            state: "x".into(),
            trigger: "y".into(),
        })
        .await
        .unwrap();
        tx.send(ProcessToService::ExecOutput {
            id: id + 1,
            channel: capsem_proto::ExecOutputChannel::Stdout,
            data: b"not ours".to_vec(),
        })
        .await
        .unwrap();
        tx.send(ProcessToService::ExecOutput {
            id,
            channel: capsem_proto::ExecOutputChannel::Stdout,
            data: b"hi\n\xff".to_vec(),
        })
        .await
        .unwrap();
        tx.send(ProcessToService::ExecResult {
            id,
            stdout: vec![],
            stderr: vec![],
            exit_code: 7,
            truncated: false,
        })
        .await
        .unwrap();
    });
    let mut client = connect(fx.address, Some(stream::STREAM_SUBPROTOCOL)).await.unwrap();
    client
        .send(ClientMessage::Binary(
            encode_control(&StreamControl::Start {
                kind: StreamKind::Exec,
                command: Some("echo hi".into()),
            })
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap().1),
        StreamStatus::Started
    );
    let (channel, output) = next_server_frame(&mut client).await.unwrap();
    assert_eq!(
        (channel, &output[1..]),
        (StreamChannel::Stdout as u8, &b"hi\n\xff"[..]),
        "raw bytes, not lossy text"
    );
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap().1),
        StreamStatus::Exit {
            code: 7,
            truncated: false
        }
    );
    assert!(
        next_server_frame(&mut client).await.is_none(),
        "the stream closes after its exit status"
    );
    owner.await.unwrap();
}

#[tokio::test]
async fn terminal_stream_relays_output_input_and_resize() {
    let fx = fixture().await;
    let (seen_tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let owner = owner(&fx.uds_path, move |tx, rx| async move {
        assert!(matches!(
            rx.recv().await.unwrap(),
            ServiceToProcess::StartTerminalStream
        ));
        tx.send(ProcessToService::TerminalOutput { data: b"$ ".to_vec() })
            .await
            .unwrap();
        for _ in 0..2 {
            seen_tx.send(rx.recv().await.unwrap()).unwrap();
        }
        tx.send(ProcessToService::TerminalStreamEnded {
            reason: "terminal closed".into(),
        })
        .await
        .unwrap();
        let _ = rx.recv().await;
    });
    let mut client = connect(fx.address, Some(stream::STREAM_SUBPROTOCOL)).await.unwrap();
    client
        .send(ClientMessage::Binary(
            encode_control(&StreamControl::Start {
                kind: StreamKind::Terminal,
                command: None,
            })
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap().1),
        StreamStatus::Started
    );
    assert_eq!(next_server_frame(&mut client).await.unwrap().1[1..], *b"$ ");
    client
        .send(ClientMessage::Binary(encode_data(StreamChannel::Stdin, b"ls\n").into()))
        .await
        .unwrap();
    client
        .send(ClientMessage::Binary(
            encode_control(&StreamControl::Resize { cols: 100, rows: 30 }).into(),
        ))
        .await
        .unwrap();
    assert!(matches!(seen.recv().await.unwrap(), ServiceToProcess::TerminalInput { data } if data == b"ls\n"));
    assert!(matches!(
        seen.recv().await.unwrap(),
        ServiceToProcess::TerminalResize { cols: 100, rows: 30 }
    ));
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap().1),
        StreamStatus::Error {
            message: "terminal closed".into()
        }
    );
    owner.await.unwrap();
}

#[tokio::test]
async fn exec_stream_relays_stdin_eof_separate_stderr_and_disconnect_cancellation() {
    let fx = fixture().await;
    let owner = owner(&fx.uds_path, |tx, rx| async move {
        let ServiceToProcess::ExecStream { id, command } = rx.recv().await.unwrap() else {
            panic!("expected ExecStream")
        };
        assert_eq!(command, "cat; echo problem >&2");
        assert!(matches!(
            rx.recv().await.unwrap(),
            ServiceToProcess::ExecStreamInput { id: input_id, data }
                if input_id == id && data == b"hello\n"
        ));
        assert!(matches!(
            rx.recv().await.unwrap(),
            ServiceToProcess::ExecStreamCloseStdin { id: input_id } if input_id == id
        ));
        tx.send(ProcessToService::ExecOutput {
            id,
            channel: capsem_proto::ExecOutputChannel::Stdout,
            data: b"hello\n".to_vec(),
        })
        .await
        .unwrap();
        tx.send(ProcessToService::ExecOutput {
            id,
            channel: capsem_proto::ExecOutputChannel::Stderr,
            data: b"problem\n".to_vec(),
        })
        .await
        .unwrap();
        assert!(matches!(
            rx.recv().await.unwrap(),
            ServiceToProcess::CancelExec { id: cancelled } if cancelled == id
        ));
    });
    let mut client = connect(fx.address, Some(stream::STREAM_SUBPROTOCOL)).await.unwrap();
    client
        .send(ClientMessage::Binary(
            encode_control(&StreamControl::Start {
                kind: StreamKind::Exec,
                command: Some("cat; echo problem >&2".into()),
            })
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap().1),
        StreamStatus::Started
    );
    client
        .send(ClientMessage::Binary(
            encode_data(StreamChannel::Stdin, b"hello\n").into(),
        ))
        .await
        .unwrap();
    client
        .send(ClientMessage::Binary(encode_control(&StreamControl::CloseStdin).into()))
        .await
        .unwrap();
    let (channel, stdout) = next_server_frame(&mut client).await.unwrap();
    assert_eq!((channel, &stdout[1..]), (StreamChannel::Stdout as u8, &b"hello\n"[..]));
    let (channel, stderr) = next_server_frame(&mut client).await.unwrap();
    assert_eq!(
        (channel, &stderr[1..]),
        (StreamChannel::Stderr as u8, &b"problem\n"[..])
    );
    client.close(None).await.unwrap();
    owner.await.unwrap();
}

#[tokio::test]
async fn upgrade_without_the_stream_subprotocol_is_refused() {
    let fx = fixture().await;
    for protocol in [None, Some("capsem.stream.v0")] {
        assert!(connect(fx.address, protocol).await.is_err(), "{protocol:?}");
    }
}

#[tokio::test]
async fn a_stream_must_start_before_anything_else() {
    let fx = fixture().await;
    let mut client = connect(fx.address, Some(stream::STREAM_SUBPROTOCOL)).await.unwrap();
    client
        .send(ClientMessage::Binary(encode_data(StreamChannel::Stdin, b"ls\n").into()))
        .await
        .unwrap();
    assert!(matches!(
        status(&next_server_frame(&mut client).await.unwrap().1),
        StreamStatus::Error { .. }
    ));
}

#[tokio::test]
async fn container_stream_starts_a_staged_workload_once_and_records_its_exit() {
    let fx = fixture().await;
    let generation = fx.state.containers.stage_for_tests("box", "registry.example/app:1");
    let owner = owner(&fx.uds_path, |tx, rx| async move {
        let ServiceToProcess::ExecStream { id, command } = rx.recv().await.unwrap() else {
            panic!("expected ExecStream")
        };
        assert_eq!(command, capsem_core::container::LAUNCH_COMMAND);
        tx.send(ProcessToService::ExecResult {
            id,
            stdout: vec![],
            stderr: vec![],
            exit_code: 3,
            truncated: false,
        })
        .await
        .unwrap();
    });
    let start = encode_control(&StreamControl::Start {
        kind: StreamKind::Container,
        command: None,
    });
    let mut client = connect(fx.address, Some(stream::STREAM_SUBPROTOCOL)).await.unwrap();
    client.send(ClientMessage::Binary(start.clone().into())).await.unwrap();
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap().1),
        StreamStatus::Started
    );
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap().1),
        StreamStatus::Exit {
            code: 3,
            truncated: false
        }
    );
    owner.await.unwrap();
    let recorded = fx.state.containers.status("box").unwrap();
    assert_eq!(
        (recorded.state, recorded.exit_code),
        (capsem_api::ContainerState::Exited, Some(3))
    );
    let _ = generation;

    let mut second = connect(fx.address, Some(stream::STREAM_SUBPROTOCOL)).await.unwrap();
    second.send(ClientMessage::Binary(start.into())).await.unwrap();
    match status(&next_server_frame(&mut second).await.unwrap().1) {
        StreamStatus::Error { message } => assert!(message.contains("not staged"), "{message}"),
        other => panic!("a finished workload must not start again: {other:?}"),
    }
}
